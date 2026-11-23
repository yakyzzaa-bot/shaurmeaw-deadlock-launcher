//! Модуль 1: Каталог и менеджер профилей (пресетов) `gameinfo.gi` и резервных копий.
//!
//! - Пресеты хранятся в `%APPDATA%\ShaurMeaw\presets\<имя>.gi` + `meta.json` (URL-источники).
//! - Резервные копии с датой и временем создаются в `%APPDATA%\ShaurMeaw\backups\`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

pub const DEFAULT_PRESET_SHAURMEAW: &str = "ShaurMeaw (Мой конфиг)";
pub const DEFAULT_PRESET_PERFORMANCE_FPS: &str = "Перформанс FPS (OptimizationLock 4.0)";
pub const DEFAULT_PRESET_ULTRA_FPS: &str = "Ультра FPS (Без теней и тумана)";
pub const DEFAULT_PRESET_VALVE: &str = "Официальный от Valve (без модов)";
pub const DEFAULT_PRESET_MODS: &str = "Для модов";
pub const LEGACY_PRESET_VANILLA: &str = "Чистый оригинал";
pub const CLEAN_VANILLA_MARKER: &str = "// [ShaurMeaw Clean Vanilla Preset]";

pub const VALVE_STEAMDB_URL: &str =
    "https://raw.githubusercontent.com/SteamDatabase/GameTracking-Deadlock/master/game/citadel/gameinfo.gi";

const MODS_PRESET_CONTENT: &str = r#""GameInfo"
{
    FileSystem
    {
// Deadlock Mod Manager - Start
        SearchPaths
        {
            Game_UILanguage     citadel_*LANGUAGE*
            Game_LowViolence    citadel_lv
            Game                citadel/dmm_localization_default
            Game                citadel/addons
            Mod                 citadel
            Write               citadel
            Game                citadel
            Mod                 core
            Write               core
            Game                core
        }
// Deadlock Mod Manager - End
    }
}
"#;

const MAX_BACKUPS_TO_KEEP: usize = 40;

#[derive(Clone, Debug)]
pub struct PresetEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_label: String,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BackupEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct PresetMeta {
    #[serde(default)]
    source_url: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct PresetsMetaFile {
    #[serde(default)]
    presets: HashMap<String, PresetMeta>,
}

// ============================================================ публичные обёртки

pub fn ensure_default_presets() {
    let _ = ensure_default_presets_in(&Settings::gameinfo_presets_dir());
}

pub fn read_preset(name: &str) -> Result<String, String> {
    read_preset_in(&Settings::gameinfo_presets_dir(), name)
}

pub fn save_preset(name: &str, content: &str, source_url: Option<String>) -> Result<String, String> {
    save_preset_in(&Settings::gameinfo_presets_dir(), name, content, source_url)
}

pub fn create_from_current(game_path: &Path, name: &str) -> Result<String, String> {
    create_from_current_in(&Settings::gameinfo_presets_dir(), game_path, name)
}

pub fn import_from_file(src: &Path, name_override: Option<&str>) -> Result<String, String> {
    import_from_file_in(&Settings::gameinfo_presets_dir(), src, name_override)
}

pub fn import_from_url(url: &str, name: &str) -> Result<String, String> {
    import_from_url_in(&Settings::gameinfo_presets_dir(), url, name)
}

pub fn refresh_from_url(name: &str) -> Result<String, String> {
    refresh_from_url_in(&Settings::gameinfo_presets_dir(), name)
}

pub fn fetch_official_valve_from_steamdb() -> Result<String, String> {
    fetch_official_valve_from_steamdb_in(&Settings::gameinfo_presets_dir())
}

pub fn rename_preset(old_name: &str, new_name: &str) -> Result<String, String> {
    rename_preset_in(&Settings::gameinfo_presets_dir(), old_name, new_name)
}

pub fn delete_preset(name: &str) -> Result<(), String> {
    delete_preset_in(&Settings::gameinfo_presets_dir(), name)
}

pub fn create_timestamped_backup(game_path: &Path) -> Result<Option<PathBuf>, String> {
    create_timestamped_backup_in(&Settings::gameinfo_backups_dir(), game_path)
}

pub fn restore_backup(backup_path: &Path, game_path: &Path) -> Result<String, String> {
    restore_backup_in(&Settings::gameinfo_backups_dir(), backup_path, game_path)
}

// ============================================================ реализация с явными папками

pub fn ensure_default_presets_in(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Не удалось создать папку presets: {e}"))?;
    let has_any = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let p = e.path();
            p.is_file()
                && p.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gi"))
        });

    if !has_any {
        save_preset_in(
            dir,
            DEFAULT_PRESET_SHAURMEAW,
            crate::embedded::shaurmeaw_gi(),
            None,
        )?;
        save_preset_in(
            dir,
            DEFAULT_PRESET_PERFORMANCE_FPS,
            crate::embedded::performance_fps_gi(),
            None,
        )?;
        save_preset_in(
            dir,
            DEFAULT_PRESET_ULTRA_FPS,
            crate::embedded::ultra_fps_gi(),
            None,
        )?;
        save_preset_in(
            dir,
            DEFAULT_PRESET_VALVE,
            crate::embedded::valve_vanilla_gi(),
            None,
        )?;
        save_preset_in(dir, DEFAULT_PRESET_MODS, MODS_PRESET_CONTENT, None)?;
    } else {
        // Миграция со старой 2-строчной заглушки «Чистый оригинал» на полноценный файл Valve
        let legacy_path = dir.join(format!("{LEGACY_PRESET_VANILLA}.gi"));
        if legacy_path.exists() {
            if let Ok(txt) = fs::read_to_string(&legacy_path) {
                if txt.contains(CLEAN_VANILLA_MARKER) {
                    let _ = fs::remove_file(&legacy_path);
                }
            }
        }
        let perf_path = dir.join(format!("{DEFAULT_PRESET_PERFORMANCE_FPS}.gi"));
        if !perf_path.exists() {
            save_preset_in(
                dir,
                DEFAULT_PRESET_PERFORMANCE_FPS,
                crate::embedded::performance_fps_gi(),
                None,
            )?;
        }
        let ultra_fps_path = dir.join(format!("{DEFAULT_PRESET_ULTRA_FPS}.gi"));
        if !ultra_fps_path.exists() {
            save_preset_in(
                dir,
                DEFAULT_PRESET_ULTRA_FPS,
                crate::embedded::ultra_fps_gi(),
                None,
            )?;
        }
        let valve_path = dir.join(format!("{DEFAULT_PRESET_VALVE}.gi"));
        if !valve_path.exists() {
            save_preset_in(
                dir,
                DEFAULT_PRESET_VALVE,
                crate::embedded::valve_vanilla_gi(),
                None,
            )?;
        }
    }
    Ok(())
}

pub fn list_presets_in(dir: &Path) -> Vec<PresetEntry> {
    let meta = load_meta(dir);
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("gi"))
            {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let meta_fs = entry.metadata().ok();
                    let size_bytes = meta_fs.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified_label = meta_fs
                        .and_then(|m| m.modified().ok())
                        .map(format_system_time_human)
                        .unwrap_or_else(|| "—".to_owned());
                    let source_url = meta
                        .presets
                        .get(stem)
                        .and_then(|m| m.source_url.clone())
                        .filter(|u| !u.trim().is_empty());
                    list.push(PresetEntry {
                        name: stem.to_owned(),
                        path: p,
                        size_bytes,
                        modified_label,
                        source_url,
                    });
                }
            }
        }
    }
    list.sort_by(|a, b| {
        preset_sort_rank(&a.name)
            .cmp(&preset_sort_rank(&b.name))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

fn preset_sort_rank(name: &str) -> u8 {
    if name == DEFAULT_PRESET_SHAURMEAW {
        0
    } else if name == DEFAULT_PRESET_PERFORMANCE_FPS {
        1
    } else if name == DEFAULT_PRESET_ULTRA_FPS {
        2
    } else if name == DEFAULT_PRESET_VALVE {
        3
    } else if name == DEFAULT_PRESET_MODS {
        4
    } else {
        5
    }
}

pub fn fetch_official_valve_from_steamdb_in(dir: &Path) -> Result<String, String> {
    let content = download_url_text(VALVE_STEAMDB_URL)?;
    validate_gameinfo_text(&content)?;
    if !content.contains("SearchPaths") || !content.contains("ConVars") {
        return Err("Скачанный со SteamDB файл не содержит SearchPaths/ConVars".into());
    }
    save_preset_in(dir, DEFAULT_PRESET_VALVE, &content, None)?;
    Ok(content)
}

pub fn read_preset_in(dir: &Path, name: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.gi"));
    crate::fs_safe::read_text_lossy_with_retry(&path)
        .map_err(|e| format!("Не удалось прочитать пресет «{clean}»: {e}"))
}

pub fn save_preset_in(
    dir: &Path,
    name: &str,
    content: &str,
    source_url: Option<String>,
) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.gi"));
    crate::fs_safe::atomic_write_with_retry(&path, content.as_bytes())
        .map_err(|e| format!("Не удалось сохранить пресет «{clean}»: {e}"))?;

    let mut meta = load_meta(dir);
    if let Some(url) = source_url.filter(|u| !u.trim().is_empty()) {
        meta.presets.insert(
            clean.clone(),
            PresetMeta {
                source_url: Some(url),
            },
        );
    }
    let _ = save_meta(dir, &meta);
    Ok(clean)
}

pub fn create_from_current_in(dir: &Path, game_path: &Path, name: &str) -> Result<String, String> {
    let gi = gameinfo_file(game_path);
    if !gi.exists() {
        return Err("Файл game\\citadel\\gameinfo.gi не найден в папке Deadlock".into());
    }
    let content = crate::fs_safe::read_text_lossy_with_retry(&gi)
        .map_err(|e| format!("Не удалось прочитать текущий gameinfo.gi: {e}"))?;
    save_preset_in(dir, name, &content, None)
}

pub fn import_from_file_in(
    dir: &Path,
    src: &Path,
    name_override: Option<&str>,
) -> Result<String, String> {
    let content = crate::fs_safe::read_text_lossy_with_retry(src)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", src.display()))?;
    validate_gameinfo_text(&content)?;

    let fallback_name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Импортированный профиль");
    let raw_name = name_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_name);

    save_preset_in(dir, raw_name, &content, None)
}

pub fn import_from_url_in(dir: &Path, url: &str, name: &str) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Ссылка должна начинаться с https:// или http://".into());
    }

    let content = download_url_text(url)?;
    validate_gameinfo_text(&content)?;

    let preset_name = if name.trim().is_empty() {
        guess_name_from_url(url)
    } else {
        name.trim().to_owned()
    };

    save_preset_in(dir, &preset_name, &content, Some(url.to_owned()))
}

pub fn refresh_from_url_in(dir: &Path, name: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let meta = load_meta(dir);
    let url = meta
        .presets
        .get(&clean)
        .and_then(|m| m.source_url.as_deref())
        .ok_or_else(|| format!("У профиля «{clean}» не привязана Raw-ссылка"))?;

    let content = download_url_text(url)?;
    validate_gameinfo_text(&content)?;
    save_preset_in(dir, &clean, &content, Some(url.to_owned()))
}

pub fn rename_preset_in(dir: &Path, old_name: &str, new_name: &str) -> Result<String, String> {
    let old_clean = sanitize_name(old_name)?;
    let new_clean = sanitize_name(new_name)?;
    if old_clean == new_clean {
        return Ok(new_clean);
    }

    let old_path = dir.join(format!("{old_clean}.gi"));
    let new_path = dir.join(format!("{new_clean}.gi"));
    if !old_path.exists() {
        return Err(format!("Пресет «{old_clean}» не найден"));
    }
    if new_path.exists() {
        return Err(format!("Пресет с именем «{new_clean}» уже существует"));
    }

    fs::rename(&old_path, &new_path)
        .map_err(|e| format!("Не удалось переименовать пресет: {e}"))?;

    let mut meta = load_meta(dir);
    if let Some(entry) = meta.presets.remove(&old_clean) {
        meta.presets.insert(new_clean.clone(), entry);
        let _ = save_meta(dir, &meta);
    }
    Ok(new_clean)
}

pub fn delete_preset_in(dir: &Path, name: &str) -> Result<(), String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.gi"));
    if !path.exists() {
        return Err(format!("Пресет «{clean}» не найден"));
    }
    fs::remove_file(&path).map_err(|e| format!("Не удалось удалить пресет: {e}"))?;

    let mut meta = load_meta(dir);
    if meta.presets.remove(&clean).is_some() {
        let _ = save_meta(dir, &meta);
    }
    Ok(())
}

// ============================================================ бэкапы (backups/)

pub fn create_timestamped_backup_in(
    backups_dir: &Path,
    game_path: &Path,
) -> Result<Option<PathBuf>, String> {
    let gi = gameinfo_file(game_path);
    if !gi.exists() {
        return Ok(None);
    }

    fs::create_dir_all(backups_dir)
        .map_err(|e| format!("Не удалось создать папку backups: {e}"))?;

    let stamp = format_timestamp_now();
    let mut dst = backups_dir.join(format!("gameinfo_{stamp}.gi"));
    let mut idx = 1usize;
    while dst.exists() {
        dst = backups_dir.join(format!("gameinfo_{stamp}_{idx}.gi"));
        idx += 1;
    }

    let bytes = crate::fs_safe::read_bytes_with_retry(&gi)
        .map_err(|e| format!("Не удалось прочитать gameinfo.gi: {e}"))?;
    crate::fs_safe::atomic_write_with_retry(&dst, &bytes)
        .map_err(|e| format!("Не удалось создать бэкап gameinfo.gi: {e}"))?;

    rotate_backups(backups_dir, MAX_BACKUPS_TO_KEEP);
    Ok(Some(dst))
}

pub fn list_backups_in(backups_dir: &Path) -> Vec<BackupEntry> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("gi"))
            {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("backup.gi")
                    .to_owned();
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push(BackupEntry {
                    name,
                    path: p,
                    size_bytes,
                });
            }
        }
    }
    // Обратный порядок по имени `gameinfo_YYYY-MM-DD_HH-MM-SS.gi` -> свежие сверху
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

pub fn restore_backup_in(
    backups_dir: &Path,
    backup_path: &Path,
    game_path: &Path,
) -> Result<String, String> {
    if !backup_path.exists() {
        return Err("Файл бэкапа не найден".into());
    }
    let gi = gameinfo_file(game_path);
    if gi.exists() {
        let _ = create_timestamped_backup_in(backups_dir, game_path);
    } else if let Some(parent) = gi.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let bytes = crate::fs_safe::read_bytes_with_retry(backup_path)
        .map_err(|e| format!("Не удалось прочитать бэкап: {e}"))?;
    crate::fs_safe::atomic_write_with_retry(&gi, &bytes)
        .map_err(|e| format!("Не удалось восстановить бэкап: {e}"))?;
    let name = backup_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("backup.gi");
    Ok(format!("Восстановлен бэкап {name}"))
}

fn rotate_backups(backups_dir: &Path, keep: usize) {
    let list = list_backups_in(backups_dir);
    for old in list.into_iter().skip(keep) {
        let _ = fs::remove_file(old.path);
    }
}

// ============================================================ вспомогательные

fn gameinfo_file(game_path: &Path) -> PathBuf {
    game_path.join("game").join("citadel").join("gameinfo.gi")
}

fn validate_gameinfo_text(text: &str) -> Result<(), String> {
    let lower = text.to_ascii_lowercase();
    if text.trim().is_empty() || (!lower.contains("gameinfo") && !text.contains(CLEAN_VANILLA_MARKER))
        || (!text.contains('{') && !text.contains(CLEAN_VANILLA_MARKER))
    {
        return Err("Файл не похож на корректный gameinfo.gi (не найдена секция GameInfo)".into());
    }
    Ok(())
}

pub fn sanitize_name(raw: &str) -> Result<String, String> {
    crate::fs_safe::sanitize_filename(raw)
        .map_err(|e| if e == "Введите корректное название" { "Введите корректное название профиля".into() } else { e })
}

#[cfg(windows)]
mod win_http {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};

    const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
    const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
    const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
    const WINHTTP_OPTION_SECURE_PROTOCOLS: u32 = 84;
    const WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2: u32 = 0x0000_0800;
    const WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3: u32 = 0x0000_2000;
    const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
    const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
    const INTERNET_SCHEME_HTTP: i32 = 1;
    const INTERNET_SCHEME_HTTPS: i32 = 2;
    const MAX_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;

    #[repr(C)]
    struct UrlComponentsW {
        dw_struct_size: u32,
        lpsz_scheme: *mut u16,
        dw_scheme_length: u32,
        n_scheme: i32,
        lpsz_host_name: *mut u16,
        dw_host_name_length: u32,
        n_port: u16,
        lpsz_user_name: *mut u16,
        dw_user_name_length: u32,
        lpsz_password: *mut u16,
        dw_password_length: u32,
        lpsz_url_path: *mut u16,
        dw_url_path_length: u32,
        lpsz_extra_info: *mut u16,
        dw_extra_info_length: u32,
    }

    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpOpen(
            psz_agent_w: *const u16,
            dw_access_type: u32,
            psz_proxy_w: *const u16,
            psz_proxy_bypass_w: *const u16,
            dw_flags: u32,
        ) -> *mut c_void;
        fn WinHttpCloseHandle(h_internet: *mut c_void) -> i32;
        fn WinHttpSetTimeouts(
            h_internet: *mut c_void,
            n_resolve_timeout: i32,
            n_connect_timeout: i32,
            n_send_timeout: i32,
            n_receive_timeout: i32,
        ) -> i32;
        fn WinHttpSetOption(
            h_internet: *mut c_void,
            dw_option: u32,
            lp_buffer: *const c_void,
            dw_buffer_length: u32,
        ) -> i32;
        fn WinHttpCrackUrl(
            pwsz_url: *const u16,
            dw_url_length: u32,
            dw_flags: u32,
            lp_url_components: *mut UrlComponentsW,
        ) -> i32;
        fn WinHttpConnect(
            h_session: *mut c_void,
            pswz_server_name: *const u16,
            n_server_port: u16,
            dw_reserved: u32,
        ) -> *mut c_void;
        fn WinHttpOpenRequest(
            h_connect: *mut c_void,
            pwsz_verb: *const u16,
            pwsz_object_name: *const u16,
            pwsz_version: *const u16,
            pwsz_referrer: *const u16,
            ppwsz_accept_types: *const *const u16,
            dw_flags: u32,
        ) -> *mut c_void;
        fn WinHttpSendRequest(
            h_request: *mut c_void,
            lpsz_headers: *const u16,
            dw_headers_length: u32,
            lp_optional: *mut c_void,
            dw_optional_length: u32,
            dw_total_length: u32,
            dw_context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(h_request: *mut c_void, lp_reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            h_request: *mut c_void,
            dw_info_level: u32,
            pwsz_name: *const u16,
            lp_buffer: *mut c_void,
            lpdw_buffer_length: *mut u32,
            lpdw_index: *mut u32,
        ) -> i32;
        fn WinHttpQueryDataAvailable(
            h_request: *mut c_void,
            lpdw_number_of_bytes_available: *mut u32,
        ) -> i32;
        fn WinHttpReadData(
            h_request: *mut c_void,
            lp_buffer: *mut c_void,
            dw_number_of_bytes_to_read: u32,
            lpdw_number_of_bytes_read: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLastError() -> u32;
    }

    struct HttpHandle(*mut c_void);

    impl Drop for HttpHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct ParsedUrl {
        pub is_https: bool,
        pub host: String,
        pub port: u16,
        pub path_and_query: String,
    }

    fn to_wide_z(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn last_err(stage: &str) -> String {
        let code = unsafe { GetLastError() };
        format!("Ошибка сети WinHTTP ({stage}, код {code})")
    }

    pub(super) fn crack_url(url: &str) -> Result<ParsedUrl, String> {
        let trimmed = url.trim();
        let url_no_nul: Vec<u16> = trimmed.encode_utf16().collect();
        if url_no_nul.is_empty() {
            return Err("Укажите корректный URL".into());
        }
        let buf_len = url_no_nul.len() + 16;
        let mut scheme = vec![0u16; buf_len];
        let mut host = vec![0u16; buf_len];
        let mut path = vec![0u16; buf_len];
        let mut extra = vec![0u16; buf_len];

        let mut url_z = url_no_nul.clone();
        url_z.push(0);

        let mut comps: UrlComponentsW = unsafe { zeroed() };
        comps.dw_struct_size = size_of::<UrlComponentsW>() as u32;
        comps.lpsz_scheme = scheme.as_mut_ptr();
        comps.dw_scheme_length = scheme.len() as u32;
        comps.lpsz_host_name = host.as_mut_ptr();
        comps.dw_host_name_length = host.len() as u32;
        comps.lpsz_url_path = path.as_mut_ptr();
        comps.dw_url_path_length = path.len() as u32;
        comps.lpsz_extra_info = extra.as_mut_ptr();
        comps.dw_extra_info_length = extra.len() as u32;

        let ok = unsafe { WinHttpCrackUrl(url_z.as_ptr(), url_no_nul.len() as u32, 0, &mut comps) };
        if ok == 0 {
            return Err(last_err("разбор URL"));
        }

        let is_https = match comps.n_scheme {
            INTERNET_SCHEME_HTTPS => true,
            INTERNET_SCHEME_HTTP => false,
            _ => return Err("Поддерживаются только ссылки http:// и https://".into()),
        };

        let host_len = (comps.dw_host_name_length as usize).min(host.len());
        let host_s = String::from_utf16_lossy(&host[..host_len]);
        if host_s.is_empty() {
            return Err("В ссылке не указан хост".into());
        }

        let path_len = (comps.dw_url_path_length as usize).min(path.len());
        let extra_len = (comps.dw_extra_info_length as usize).min(extra.len());
        let path_s = String::from_utf16_lossy(&path[..path_len]);
        let extra_s = String::from_utf16_lossy(&extra[..extra_len]);
        let mut path_and_query = if path_s.is_empty() {
            format!("/{extra_s}")
        } else {
            format!("{path_s}{extra_s}")
        };
        if !path_and_query.starts_with('/') {
            path_and_query.insert(0, '/');
        }

        Ok(ParsedUrl {
            is_https,
            host: host_s,
            port: comps.n_port,
            path_and_query,
        })
    }

    pub(super) fn download_text(url: &str) -> Result<String, String> {
        let parsed = crack_url(url)?;
        let ua_w = to_wide_z(concat!("ShaurMeaw/", env!("CARGO_PKG_VERSION")));

        let raw_session = unsafe {
            let automatic = WinHttpOpen(
                ua_w.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            );
            if automatic.is_null() {
                // Запасной режим прокси для Windows 7
                WinHttpOpen(
                    ua_w.as_ptr(),
                    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                    null(),
                    null(),
                    0,
                )
            } else {
                automatic
            }
        };
        if raw_session.is_null() {
            return Err(last_err("WinHttpOpen"));
        }
        let session = HttpHandle(raw_session);

        unsafe {
            WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000);
            if parsed.is_https {
                let mut tls_both =
                    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
                if WinHttpSetOption(
                    session.0,
                    WINHTTP_OPTION_SECURE_PROTOCOLS,
                    std::ptr::from_ref(&tls_both).cast::<c_void>(),
                    size_of::<u32>() as u32,
                ) == 0
                {
                    tls_both = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
                    let _ = WinHttpSetOption(
                        session.0,
                        WINHTTP_OPTION_SECURE_PROTOCOLS,
                        std::ptr::from_ref(&tls_both).cast::<c_void>(),
                        size_of::<u32>() as u32,
                    );
                }
            }
        }

        let host_w = to_wide_z(&parsed.host);
        let raw_conn = unsafe { WinHttpConnect(session.0, host_w.as_ptr(), parsed.port, 0) };
        if raw_conn.is_null() {
            return Err(last_err("WinHttpConnect"));
        }
        let conn = HttpHandle(raw_conn);

        let method_w = to_wide_z("GET");
        let object_w = to_wide_z(&parsed.path_and_query);
        let flags = if parsed.is_https { WINHTTP_FLAG_SECURE } else { 0 };
        let raw_req = unsafe {
            WinHttpOpenRequest(
                conn.0,
                method_w.as_ptr(),
                object_w.as_ptr(),
                null(),
                null(),
                null(),
                flags,
            )
        };
        if raw_req.is_null() {
            return Err(last_err("WinHttpOpenRequest"));
        }
        let req = HttpHandle(raw_req);

        if unsafe { WinHttpSendRequest(req.0, null(), 0, null_mut(), 0, 0, 0) } == 0 {
            return Err(last_err("отправка запроса"));
        }
        if unsafe { WinHttpReceiveResponse(req.0, null_mut()) } == 0 {
            return Err(last_err("получение ответа"));
        }

        let mut status: u32 = 0;
        let mut status_len = size_of::<u32>() as u32;
        if unsafe {
            WinHttpQueryHeaders(
                req.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                std::ptr::from_mut(&mut status).cast::<c_void>(),
                &mut status_len,
                null_mut(),
            )
        } == 0
        {
            return Err(last_err("чтение HTTP-статуса"));
        }
        if !(200..=299).contains(&status) {
            return Err(format!("Сервер вернул HTTP статус {status}"));
        }

        let mut body = Vec::new();
        loop {
            let mut avail: u32 = 0;
            if unsafe { WinHttpQueryDataAvailable(req.0, &mut avail) } == 0 {
                return Err(last_err("чтение размера данных"));
            }
            if avail == 0 {
                break;
            }
            let chunk = avail as usize;
            if body.len().saturating_add(chunk) > MAX_DOWNLOAD_BYTES {
                return Err("Скачиваемый файл превышает допустимый предел 16 МБ".into());
            }
            let old_len = body.len();
            body.resize(old_len + chunk, 0);
            let mut read: u32 = 0;
            if unsafe {
                WinHttpReadData(
                    req.0,
                    body[old_len..].as_mut_ptr().cast::<c_void>(),
                    avail,
                    &mut read,
                )
            } == 0
            {
                return Err(last_err("чтение тела ответа"));
            }
            body.truncate(old_len + read as usize);
            if read == 0 {
                break;
            }
        }

        if body.contains(&0) {
            return Err("Скачанный файл является бинарным, а не текстовым конфигом".into());
        }
        let text = match String::from_utf8(body) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        if text.trim().is_empty() {
            return Err("Сервер вернул пустой ответ".into());
        }
        Ok(text)
    }
}

fn download_url_text(url: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        win_http::download_text(url)
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("Загрузка по URL поддерживается только в Windows".into())
    }
}

fn guess_name_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    let last_seg = without_query.rsplit('/').next().unwrap_or("RemoteConfig");
    let stem = last_seg
        .strip_suffix(".gi")
        .or_else(|| last_seg.strip_suffix(".txt"))
        .unwrap_or(last_seg);
    sanitize_name(stem).unwrap_or_else(|_| "Конфиг по ссылке".to_owned())
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join("meta.json")
}

fn load_meta(dir: &Path) -> PresetsMetaFile {
    crate::fs_safe::read_text_lossy_with_retry(&meta_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_meta(dir: &Path, meta: &PresetsMetaFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    crate::fs_safe::atomic_write_with_retry(&meta_path(dir), json.as_bytes())
        .map_err(|e| e.to_string())
}

/// Форматирует `SystemTime` в читаемую дату (`DD.MM.YYYY HH:MM`, UTC).
pub fn format_system_time_human(time: SystemTime) -> String {
    let Ok(dur) = time.duration_since(UNIX_EPOCH) else {
        return "—".to_owned();
    };
    let unix_secs = dur.as_secs() as i64;
    let days = unix_secs.div_euclid(86_400);
    let sod = unix_secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{day:02}.{month:02}.{year:04} {hour:02}:{minute:02}")
}

/// Форматирует текущее время (UTC) в `YYYY-MM-DD_HH-MM-SS` без внешних зависимостей.
pub fn format_timestamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_unix_timestamp(secs)
}

fn format_unix_timestamp(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let sod = unix_secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

/// Алгоритм Говарда Хиннанта для преобразования дней с 1970-01-01 в (год, месяц, день).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_timestamp_format() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01_00-00-00");
        // 2026-11-21 00:00:00 UTC
        assert_eq!(format_unix_timestamp(1_795_219_200), "2026-11-21_00-00-00");
    }

    #[test]
    fn test_presets_crud_and_backups() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_presets_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let presets_dir = root.join("presets");
        let backups_dir = root.join("backups");
        let game_dir = root.join("Deadlock");
        let gi_dir = game_dir.join("game").join("citadel");
        fs::create_dir_all(&gi_dir).unwrap();
        fs::write(gi_dir.join("gameinfo.gi"), "\"GameInfo\"\n{\n    game \"citadel\"\n}\n").unwrap();

        ensure_default_presets_in(&presets_dir).unwrap();
        let list = list_presets_in(&presets_dir);
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].name, DEFAULT_PRESET_SHAURMEAW);
        assert_eq!(list[1].name, DEFAULT_PRESET_PERFORMANCE_FPS);
        assert_eq!(list[2].name, DEFAULT_PRESET_ULTRA_FPS);

        let created = create_from_current_in(&presets_dir, &game_dir, "Мой тест").unwrap();
        assert_eq!(created, "Мой тест");

        let renamed = rename_preset_in(&presets_dir, "Мой тест", "Тест 2").unwrap();
        assert_eq!(renamed, "Тест 2");
        assert!(read_preset_in(&presets_dir, "Тест 2").is_ok());

        let bak = create_timestamped_backup_in(&backups_dir, &game_dir).unwrap();
        assert!(bak.is_some());
        assert_eq!(list_backups_in(&backups_dir).len(), 1);

        delete_preset_in(&presets_dir, "Тест 2").unwrap();
        assert!(read_preset_in(&presets_dir, "Тест 2").is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(windows)]
    fn test_winhttp_crack_url_parses_https_and_query() {
        let p = win_http::crack_url(VALVE_STEAMDB_URL).unwrap();
        assert!(p.is_https);
        assert_eq!(p.host, "raw.githubusercontent.com");
        assert_eq!(p.port, 443);
        assert_eq!(
            p.path_and_query,
            "/SteamDatabase/GameTracking-Deadlock/master/game/citadel/gameinfo.gi"
        );

        let p2 = win_http::crack_url("http://example.com:8080/cfg?a=1").unwrap();
        assert!(!p2.is_https);
        assert_eq!(p2.host, "example.com");
        assert_eq!(p2.port, 8080);
        assert_eq!(p2.path_and_query, "/cfg?a=1");

        assert!(win_http::crack_url("file:///C:/test.txt").is_err());
    }
}
