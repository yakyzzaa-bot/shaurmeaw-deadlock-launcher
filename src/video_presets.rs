//! Каталог и менеджер пресетов `video.txt` (`video_presets/`).
//!
//! Управляет настройками графики Source 2 (`"video.cfg"`), хранящимися в
//! `<Steam>\userdata\<SteamID>\1422450\local\cfg\video.txt` и/или `game\citadel\cfg\video.txt`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::gameinfo_presets::{format_system_time_human, format_timestamp_now, sanitize_name};
use crate::settings::Settings;
use crate::steam;

pub const PRESET_SHAURMEAW: &str = "ShaurMeaw (Мой video.txt)";
pub const PRESET_VALVE: &str = "Официальный от Valve (Стандарт)";

const GPU_ID_KEYS: [(&str, &str); 3] = [
    ("vendorid", "VendorID"),
    ("deviceid", "DeviceID"),
    ("setting.knowndevice", "setting.knowndevice"),
];

const RESOLUTION_KEYS: [(&str, &str); 6] = [
    ("setting.defaultres", "setting.defaultres"),
    ("setting.defaultresheight", "setting.defaultresheight"),
    (
        "setting.refreshrate_numerator",
        "setting.refreshrate_numerator",
    ),
    (
        "setting.refreshrate_denominator",
        "setting.refreshrate_denominator",
    ),
    ("setting.monitor_index", "setting.monitor_index"),
    ("setting.aspectratiomode", "setting.aspectratiomode"),
];

#[derive(Clone, Debug)]
pub struct VideoPresetEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_label: String,
}

// ============================================================ публичные обёртки

pub fn ensure_default_presets() {
    let _ = ensure_default_presets_in(&Settings::video_presets_dir());
}

pub fn read_preset(name: &str) -> Result<String, String> {
    read_preset_in(&Settings::video_presets_dir(), name)
}

pub fn save_preset(name: &str, content: &str) -> Result<String, String> {
    save_preset_in(&Settings::video_presets_dir(), name, content)
}

pub fn create_from_current(
    steam_path: &Path,
    game_path: &Path,
    name: &str,
) -> Result<String, String> {
    create_from_current_in(&Settings::video_presets_dir(), steam_path, game_path, name)
}

pub fn import_from_file(src: &Path) -> Result<String, String> {
    import_from_file_in(&Settings::video_presets_dir(), src)
}

pub fn delete_preset(name: &str) -> Result<(), String> {
    delete_preset_in(&Settings::video_presets_dir(), name)
}

pub fn apply_video_preset(s: &Settings, preset_name: &str) -> Result<String, String> {
    let active_user = active_steam_user_id();
    apply_video_preset_in(
        &Settings::video_presets_dir(),
        &Settings::gameinfo_backups_dir(),
        s,
        preset_name,
        active_user.as_deref(),
    )
}

pub fn reset_video_for_valve_autogen(s: &Settings) -> Result<String, String> {
    let active_user = active_steam_user_id();
    reset_video_for_valve_autogen_in(
        &Settings::gameinfo_backups_dir(),
        s,
        active_user.as_deref(),
    )
}

// ============================================================ CRUD с явной папкой

pub fn ensure_default_presets_in(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("Не удалось создать папку video_presets: {e}"))?;

    let shaurmeaw_path = dir.join(format!("{PRESET_SHAURMEAW}.txt"));
    if !shaurmeaw_path.exists() {
        save_preset_in(dir, PRESET_SHAURMEAW, crate::embedded::shaurmeaw_video())?;
    }

    let valve_path = dir.join(format!("{PRESET_VALVE}.txt"));
    if !valve_path.exists() {
        save_preset_in(dir, PRESET_VALVE, crate::embedded::valve_video())?;
    }

    Ok(())
}

pub fn list_presets_in(dir: &Path) -> Vec<VideoPresetEntry> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("txt"))
            {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let meta_fs = entry.metadata().ok();
                    let size_bytes = meta_fs.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified_label = meta_fs
                        .and_then(|m| m.modified().ok())
                        .map(format_system_time_human)
                        .unwrap_or_else(|| "—".to_owned());
                    list.push(VideoPresetEntry {
                        name: stem.to_owned(),
                        path: p,
                        size_bytes,
                        modified_label,
                    });
                }
            }
        }
    }

    list.sort_by(|a, b| {
        rank(&a.name)
            .cmp(&rank(&b.name))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

fn rank(name: &str) -> u8 {
    if name == PRESET_SHAURMEAW {
        0
    } else if name == PRESET_VALVE {
        1
    } else {
        2
    }
}

pub fn read_preset_in(dir: &Path, name: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.txt"));
    crate::fs_safe::read_text_lossy_with_retry(&path)
        .map_err(|e| format!("Не удалось прочитать video-пресет «{clean}»: {e}"))
}

pub fn save_preset_in(dir: &Path, name: &str, content: &str) -> Result<String, String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.txt"));
    crate::fs_safe::atomic_write_with_retry(&path, content.as_bytes())
        .map_err(|e| format!("Не удалось сохранить video-пресет «{clean}»: {e}"))?;
    Ok(clean)
}

fn validate_video_cfg_text(text: &str) -> Result<(), String> {
    let clean = text.trim_start_matches('\u{feff}');
    let lower = clean.to_ascii_lowercase();
    let looks_like_vdf = clean.contains('{')
        && clean.contains('}')
        && (lower.contains("video.cfg") || lower.contains("setting."));
    if clean.trim().is_empty() || !looks_like_vdf {
        return Err(
            "Файл не похож на корректный video.txt / video.cfg (не найдены \"video.cfg\" или параметры setting.*)".into(),
        );
    }
    Ok(())
}

pub fn create_from_current_in(
    dir: &Path,
    steam_path: &Path,
    game_path: &Path,
    name: &str,
) -> Result<String, String> {
    let src = primary_existing_video_path(steam_path, game_path).ok_or_else(|| {
        "Текущий video.txt не найден ни в Steam\\userdata, ни в game\\citadel\\cfg".to_owned()
    })?;
    let content = crate::fs_safe::read_text_lossy_with_retry(&src)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", src.display()))?;
    validate_video_cfg_text(&content)?;
    save_preset_in(dir, name, &content)
}

pub fn import_from_file_in(dir: &Path, src: &Path) -> Result<String, String> {
    let content = crate::fs_safe::read_text_lossy_with_retry(src)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", src.display()))?;
    validate_video_cfg_text(&content)?;
    let fallback = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Импорт video");
    save_preset_in(dir, fallback, &content)
}

pub fn delete_preset_in(dir: &Path, name: &str) -> Result<(), String> {
    let clean = sanitize_name(name)?;
    let path = dir.join(format!("{clean}.txt"));
    if !path.exists() {
        return Err(format!("video-пресет «{clean}» не найден"));
    }
    fs::remove_file(&path).map_err(|e| format!("Не удалось удалить пресет: {e}"))
}

// ============================================================ пути Steam userdata и cfg

/// Возвращает Steam3 ID текущего активного пользователя Steam из реестра Windows.
pub fn active_steam_user_id() -> Option<String> {
    steam::active_steam_user_id().map(|uid| uid.to_string())
}

/// Возвращает все найденные пути `<steam>\userdata\<id>\1422450\local\cfg\video.txt`.
pub fn steam_userdata_video_paths(steam_path: &Path) -> Vec<(String, PathBuf)> {
    let userdata_dir = steam_path.join("userdata");
    let Ok(entries) = fs::read_dir(&userdata_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if !p.join(steam::APP_ID).is_dir() {
            continue;
        }
        let video_path = p
            .join(steam::APP_ID)
            .join("local")
            .join("cfg")
            .join("video.txt");
        out.push((name.to_owned(), video_path));
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn game_cfg_video_path(game_path: &Path) -> PathBuf {
    steam::cfg_dir(game_path).join("video.txt")
}

fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    let norm = candidate.to_string_lossy().to_ascii_lowercase();
    if !paths
        .iter()
        .any(|p| p.to_string_lossy().to_ascii_lowercase() == norm)
    {
        paths.push(candidate);
    }
}

/// Собирает все целевые пути `video.txt`, куда нужно записать настройки.
pub fn resolve_target_paths(
    steam_path: &Path,
    game_path: &Path,
    active_user: Option<&str>,
    apply_all_accounts: bool,
) -> Vec<PathBuf> {
    let mut targets = Vec::new();

    if !steam_path.as_os_str().is_empty() && steam_path.exists() {
        let accounts = steam_userdata_video_paths(steam_path);
        let active_uid = active_user.map(str::trim).filter(|s| !s.is_empty());

        if apply_all_accounts {
            for (_, path) in accounts {
                push_unique_path(&mut targets, path);
            }
            if let Some(uid) = active_uid {
                let user_root = steam_path.join("userdata").join(uid);
                if user_root.is_dir() {
                    let active_path = user_root
                        .join(steam::APP_ID)
                        .join("local")
                        .join("cfg")
                        .join("video.txt");
                    push_unique_path(&mut targets, active_path);
                }
            }
        } else if let Some(uid) = active_uid.filter(|u| steam_path.join("userdata").join(u).is_dir()) {
            let active_path = steam_path
                .join("userdata")
                .join(uid)
                .join(steam::APP_ID)
                .join("local")
                .join("cfg")
                .join("video.txt");
            push_unique_path(&mut targets, active_path);
        } else {
            for (_, path) in accounts {
                push_unique_path(&mut targets, path);
            }
        }
    }

    if steam::is_valid_game_dir(game_path) {
        push_unique_path(&mut targets, game_cfg_video_path(game_path));
    }

    targets
}

/// Выбирает наиболее приоритетный существующий файл `video.txt` на ПК.
pub fn primary_existing_video_path(steam_path: &Path, game_path: &Path) -> Option<PathBuf> {
    if !steam_path.as_os_str().is_empty() && steam_path.exists() {
        if let Some(uid) = active_steam_user_id() {
            let p = steam_path
                .join("userdata")
                .join(&uid)
                .join(steam::APP_ID)
                .join("local")
                .join("cfg")
                .join("video.txt");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    if steam::is_valid_game_dir(game_path) {
        let in_game = game_cfg_video_path(game_path);
        if in_game.is_file() {
            return Some(in_game);
        }
    }

    if !steam_path.as_os_str().is_empty() && steam_path.exists() {
        for (_, p) in steam_userdata_video_paths(steam_path) {
            if p.is_file() {
                return Some(p);
            }
        }
    }

    None
}

// ============================================================ адаптация под текущий ПК

fn extract_quoted_kv(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return None;
    }
    let after_key_open = trimmed.strip_prefix('"')?;
    let key_end = crate::gameinfo_sync::find_closing_quote(after_key_open)?;
    let key = &after_key_open[..key_end];
    let rest = after_key_open[key_end + 1..].trim_start();
    let after_val_open = rest.strip_prefix('"')?;
    let val_end = crate::gameinfo_sync::find_closing_quote(after_val_open)?;
    let val = &after_val_open[..val_end];
    Some((key.to_owned(), val.to_owned()))
}

fn replace_quoted_kv_value(line: &str, key_lower: &str, new_val: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return None;
    }
    let prefix_len = line.len() - trimmed.len();
    let after_key_open = trimmed.strip_prefix('"')?;
    let key_end = crate::gameinfo_sync::find_closing_quote(after_key_open)?;
    let key = &after_key_open[..key_end];
    if !key.eq_ignore_ascii_case(key_lower) {
        return None;
    }

    let after_key_close = &after_key_open[key_end + 1..];
    let ws_len = after_key_close.len() - after_key_close.trim_start().len();
    let val_slice = &after_key_close[ws_len..];
    let after_val_open = val_slice.strip_prefix('"')?;
    let val_end = crate::gameinfo_sync::find_closing_quote(after_val_open)?;
    let tail = &after_val_open[val_end + 1..];

    let before_val_offset = prefix_len + 1 + key_end + 1 + ws_len;
    Some(format!("{}\"{new_val}\"{tail}", &line[..before_val_offset]))
}

fn extract_kv_pairs_map(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = extract_quoted_kv(line) {
            map.insert(k.to_ascii_lowercase(), v);
        }
    }
    map
}

/// Адаптирует `video.txt` под текущий ПК: по флагам переносит `VendorID`/`DeviceID` и разрешение из существующего файла.
pub fn adapt_video_cfg_for_machine(
    preset_text: &str,
    existing_text: Option<&str>,
    preserve_gpu_ids: bool,
    preserve_resolution: bool,
) -> String {
    let normalized = preset_text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    let mut overrides: Vec<(String, String, String)> = Vec::new();
    if let Some(existing) = existing_text {
        let existing_map = extract_kv_pairs_map(existing);

        if preserve_gpu_ids {
            let vendor = existing_map
                .get("vendorid")
                .map(String::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty() && *v != "0");
            let device = existing_map
                .get("deviceid")
                .map(String::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty() && *v != "0");

            if vendor.is_some() || device.is_some() {
                for (kl, display) in GPU_ID_KEYS {
                    if let Some(val) = existing_map.get(kl).filter(|v| !v.trim().is_empty()) {
                        if matches!(kl, "vendorid" | "deviceid") && val.trim() == "0" {
                            continue;
                        }
                        overrides.push((kl.to_owned(), display.to_owned(), val.clone()));
                    }
                }
            }
        }

        if preserve_resolution {
            for (kl, display) in RESOLUTION_KEYS {
                if let Some(val) = existing_map.get(kl).filter(|v| !v.trim().is_empty()) {
                    overrides.push((kl.to_owned(), display.to_owned(), val.clone()));
                }
            }
        }
    }

    let override_map: HashMap<&str, &str> = overrides
        .iter()
        .map(|(kl, _, v)| (kl.as_str(), v.as_str()))
        .collect();

    let mut applied = HashSet::new();
    let mut lines: Vec<String> = normalized
        .lines()
        .map(|line| {
            if let Some((k, _)) = extract_quoted_kv(line) {
                let kl = k.to_ascii_lowercase();
                if let Some(new_val) = override_map.get(kl.as_str()) {
                    if let Some(replaced) = replace_quoted_kv_value(line, &kl, new_val) {
                        applied.insert(kl);
                        return replaced;
                    }
                }
            }
            line.to_owned()
        })
        .collect();

    let missing: Vec<&(String, String, String)> = overrides
        .iter()
        .filter(|(kl, _, _)| !applied.contains(kl.as_str()))
        .collect();

    if !missing.is_empty() {
        let insert_idx = lines
            .iter()
            .rposition(|l| l.trim() == "}")
            .unwrap_or(lines.len());
        for (offset, (_, display, val)) in missing.into_iter().enumerate() {
            lines.insert(insert_idx + offset, format!("\t\"{display}\"\t\t\"{val}\""));
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.replace('\n', "\r\n")
}

// ============================================================ запись, бэкап и сброс

fn set_readonly(path: &Path, readonly: bool) -> Result<(), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let mut perms = meta.permissions();
    perms.set_readonly(readonly);
    fs::set_permissions(path, perms).map_err(|e| e.to_string())
}

fn save_timestamped_video_backup(backups_dir: &Path, src: &Path) -> Result<Option<PathBuf>, String> {
    if !src.is_file() {
        return Ok(None);
    }
    let bytes = crate::fs_safe::read_bytes_with_retry(src)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", src.display()))?;
    fs::create_dir_all(backups_dir)
        .map_err(|e| format!("Не удалось создать папку backups: {e}"))?;

    let stamp = format_timestamp_now();
    let mut dst = backups_dir.join(format!("video_{stamp}.txt"));
    let mut idx = 1usize;
    while dst.exists() {
        dst = backups_dir.join(format!("video_{stamp}_{idx}.txt"));
        idx += 1;
    }
    crate::fs_safe::atomic_write_with_retry(&dst, &bytes)
        .map_err(|e| format!("Не удалось создать бэкап video.txt: {e}"))?;
    Ok(Some(dst))
}

pub fn apply_video_preset_in(
    presets_dir: &Path,
    backups_dir: &Path,
    s: &Settings,
    preset_name: &str,
    active_user: Option<&str>,
) -> Result<String, String> {
    let preset_text = read_preset_in(presets_dir, preset_name)?;
    let targets = resolve_target_paths(
        Path::new(&s.steam_path),
        Path::new(&s.game_path),
        active_user,
        s.video_apply_all_accounts,
    );

    if targets.is_empty() {
        return Err(
            "Не найдены пути для video.txt — укажите папку Steam или Deadlock во вкладке «Пути»"
                .into(),
        );
    }

    if let Some(first_existing) = targets.iter().find(|p| p.is_file()) {
        save_timestamped_video_backup(backups_dir, first_existing)?;
    }

    let mut written = 0usize;
    for path in &targets {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Не удалось создать {}: {e}", parent.display()))?;
        }
        let existing_text = crate::fs_safe::read_text_lossy_with_retry(path).ok();
        let adapted = adapt_video_cfg_for_machine(
            &preset_text,
            existing_text.as_deref(),
            s.video_preserve_gpu_ids,
            s.video_preserve_resolution,
        );

        crate::fs_safe::atomic_write_with_retry(path, adapted.as_bytes()).map_err(|e| {
            if e.kind() == ErrorKind::PermissionDenied {
                format!(
                    "Нет прав на запись {}: {e}. Снимите «Только чтение», проверьте антивирус или запустите ShaurMeaw.exe от имени администратора.",
                    path.display()
                )
            } else {
                format!("Не удалось записать {}: {e}", path.display())
            }
        })?;

        if s.video_set_readonly {
            let _ = set_readonly(path, true);
        }
        written += 1;
    }

    Ok(format!(
        "video.txt [{preset_name}] применён (обновлено файлов: {written})"
    ))
}

pub fn reset_video_for_valve_autogen_in(
    backups_dir: &Path,
    s: &Settings,
    active_user: Option<&str>,
) -> Result<String, String> {
    let targets = resolve_target_paths(
        Path::new(&s.steam_path),
        Path::new(&s.game_path),
        active_user,
        true,
    );

    if targets.is_empty() {
        return Err(
            "Не найдены пути для video.txt — укажите папку Steam или Deadlock во вкладке «Пути»"
                .into(),
        );
    }

    if let Some(first_existing) = targets.iter().find(|p| p.is_file()) {
        save_timestamped_video_backup(backups_dir, first_existing)?;
    }

    let mut deleted = 0usize;
    for path in &targets {
        if path.exists() {
            crate::fs_safe::clear_readonly(path);
            fs::remove_file(path)
                .map_err(|e| format!("Не удалось удалить {}: {e}", path.display()))?;
            deleted += 1;
        }
    }

    if deleted == 0 {
        Ok("Файлы video.txt уже отсутствуют — при следующем запуске Deadlock создаст новый автоматически".into())
    } else {
        Ok(format!(
            "Сброшено файлов video.txt: {deleted}. При запуске Deadlock создаст чистые настройки Valve"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn test_video_presets_defaults_and_crud() {
        let root = unique_temp("shaurmeaw_video_crud");
        let dir = root.join("video_presets");
        ensure_default_presets_in(&dir).unwrap();

        let list = list_presets_in(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, PRESET_SHAURMEAW);
        assert_eq!(list[1].name, PRESET_VALVE);

        let shaur = read_preset_in(&dir, PRESET_SHAURMEAW).unwrap();
        assert!(shaur.contains("\"setting.defaultres\"\t\t\"1280\""));
        assert!(shaur.contains("\"setting.r_low_latency\"\t\t\"2\""));

        let valve = read_preset_in(&dir, PRESET_VALVE).unwrap();
        assert!(valve.contains("\"setting.r_citadel_ssao_quality\"\t\t\"2\""));

        // Создание, импорт и удаление
        let custom_src = root.join("friend_video.txt");
        fs::write(&custom_src, crate::embedded::shaurmeaw_video()).unwrap();
        let imported = import_from_file_in(&dir, &custom_src).unwrap();
        assert_eq!(imported, "friend_video");
        assert!(read_preset_in(&dir, "friend_video").is_ok());

        delete_preset_in(&dir, "friend_video").unwrap();
        assert!(read_preset_in(&dir, "friend_video").is_err());

        // Невалидный файл отклоняется
        let bad_src = root.join("bad.txt");
        fs::write(&bad_src, "not a video config").unwrap();
        assert!(import_from_file_in(&dir, &bad_src).is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_adapt_video_cfg_preserves_gpu_and_resolution() {
        let existing_pc = "\"video.cfg\"\r\n{\r\n\t\"Version\"\t\t\"20\"\r\n\t\"VendorID\"\t\t\"4098\"\r\n\t\"DeviceID\"\t\t\"29822\"\r\n\t\"setting.knowndevice\"\t\t\"1\"\r\n\t\"setting.defaultres\"\t\t\"2560\"\r\n\t\"setting.defaultresheight\"\t\t\"1440\"\r\n\t\"setting.aspectratiomode\"\t\t\"1\"\r\n\t\"setting.monitor_index\"\t\t\"1\"\r\n}\r\n";

        // Сохраняем GPU ID, но меняем разрешение согласно пресету
        let gpu_only = adapt_video_cfg_for_machine(
            crate::embedded::shaurmeaw_video(),
            Some(existing_pc),
            true,
            false,
        );
        assert!(gpu_only.contains("\"VendorID\"\t\t\"4098\""));
        assert!(gpu_only.contains("\"DeviceID\"\t\t\"29822\""));
        assert!(gpu_only.contains("\"setting.defaultres\"\t\t\"1280\""));
        assert!(gpu_only.contains("\"setting.defaultresheight\"\t\t\"1080\""));

        // Сохраняем и GPU ID, и разрешение текущего ПК (включая недостающие ключи monitor_index / aspectratiomode)
        let both = adapt_video_cfg_for_machine(
            crate::embedded::shaurmeaw_video(),
            Some(existing_pc),
            true,
            true,
        );
        assert!(both.contains("\"VendorID\"\t\t\"4098\""));
        assert!(both.contains("\"DeviceID\"\t\t\"29822\""));
        assert!(both.contains("\"setting.defaultres\"\t\t\"2560\""));
        assert!(both.contains("\"setting.defaultresheight\"\t\t\"1440\""));
        assert!(both.contains("\"setting.aspectratiomode\"\t\t\"1\""));
        assert!(both.contains("\"setting.monitor_index\"\t\t\"1\""));
    }

    #[test]
    fn test_apply_and_reset_video_preset_handles_readonly_and_backups() {
        let root = unique_temp("shaurmeaw_video_apply");
        let presets_dir = root.join("video_presets");
        let backups_dir = root.join("backups");
        let steam_dir = root.join("Steam");
        let game_dir = steam_dir.join("steamapps").join("common").join("Deadlock");
        let citadel_cfg = game_dir.join("game").join("citadel").join("cfg");
        let user1_cfg = steam_dir
            .join("userdata")
            .join("111111")
            .join("1422450")
            .join("local")
            .join("cfg");
        let user2_cfg = steam_dir
            .join("userdata")
            .join("222222")
            .join("1422450")
            .join("local")
            .join("cfg");

        fs::create_dir_all(&citadel_cfg).unwrap();
        fs::create_dir_all(&user1_cfg).unwrap();
        fs::create_dir_all(&user2_cfg).unwrap();
        ensure_default_presets_in(&presets_dir).unwrap();

        let existing_u1 = "\"video.cfg\"\n{\n\t\"VendorID\"\t\t\"4098\"\n\t\"DeviceID\"\t\t\"9999\"\n\t\"setting.knowndevice\"\t\t\"1\"\n}\n";
        let u1_video = user1_cfg.join("video.txt");
        let game_video = citadel_cfg.join("video.txt");
        fs::write(&u1_video, existing_u1).unwrap();
        fs::write(&game_video, existing_u1).unwrap();

        // Помечаем оба файла Read-Only
        for p in [&u1_video, &game_video] {
            let mut perms = fs::metadata(p).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(p, perms).unwrap();
        }

        let mut s = Settings::default();
        s.steam_path = steam_dir.display().to_string();
        s.game_path = game_dir.display().to_string();
        s.video_preserve_gpu_ids = true;
        s.video_preserve_resolution = false;
        s.video_apply_all_accounts = false;
        s.video_set_readonly = true;

        // Применяем для активного аккаунта 111111 -> обновятся user1 и game_cfg, но не user2
        let msg = apply_video_preset_in(
            &presets_dir,
            &backups_dir,
            &s,
            PRESET_SHAURMEAW,
            Some("111111"),
        )
        .unwrap();
        assert!(msg.contains("обновлено файлов: 2"));
        assert!(fs::read_to_string(&u1_video).unwrap().contains("\"VendorID\"\t\t\"4098\""));
        assert!(fs::read_to_string(&game_video).unwrap().contains("\"setting.r_low_latency\"\t\t\"2\""));
        assert!(!user2_cfg.join("video.txt").exists());
        assert!(fs::metadata(&u1_video).unwrap().permissions().readonly());

        // Проверим создание бэкапа video_*.txt
        let backups_count = fs::read_dir(&backups_dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("video_")
            })
            .count();
        assert!(backups_count >= 1);

        // Переключаемся на официальный Valve + все аккаунты
        s.video_apply_all_accounts = true;
        s.video_set_readonly = false;
        let msg2 = apply_video_preset_in(
            &presets_dir,
            &backups_dir,
            &s,
            PRESET_VALVE,
            Some("111111"),
        )
        .unwrap();
        assert!(msg2.contains("обновлено файлов: 3"));
        assert!(user2_cfg.join("video.txt").exists());
        assert!(fs::read_to_string(&u1_video)
            .unwrap()
            .contains("\"setting.r_citadel_ssao_quality\"\t\t\"2\""));

        // Сброс для автогенерации Valve удаляет все 3 файла (даже если были readonly)
        set_readonly(&u1_video, true).unwrap();
        let reset_msg = reset_video_for_valve_autogen_in(&backups_dir, &s, Some("111111")).unwrap();
        assert!(reset_msg.contains("Сброшено файлов video.txt: 3"));
        assert!(!u1_video.exists());
        assert!(!user2_cfg.join("video.txt").exists());
        assert!(!game_video.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
