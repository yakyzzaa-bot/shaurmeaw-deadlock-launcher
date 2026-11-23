//! Модуль 2: Автоматическое отслеживание обновлений (SHA-256 / mtime / Steam buildid)
//! и умное слияние (Smart Merge) KeyValues для `gameinfo.gi`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fs_safe::{atomic_write_with_retry, read_bytes_with_retry, read_text_lossy_with_retry};
use crate::game_booster::{
    wait_event_or_dirs, DirChangeHandle, WaitOutcome, WinEvent, NOTIFY_FILE_AND_WRITE,
    WAIT_INFINITE,
};
use crate::gameinfo_presets::{self, CLEAN_VANILLA_MARKER};
use crate::settings::{GameInfoConfig, Settings};
use crate::steam;

pub const SMART_MARKER_PREFIX: &str = "// [ShaurMeaw SmartMerge]";
pub const LEGACY_AUTHOR_HEADER: &str = "//  discord : ShaurMeaw";

const SCENE_START: &str = "// [ShaurMeaw SceneSystem - Start]";
const SCENE_END: &str = "// [ShaurMeaw SceneSystem - End]";
const CONVARS_START: &str = "// [ShaurMeaw ConVars - Start]";
const CONVARS_END: &str = "// [ShaurMeaw ConVars - End]";
const LEGACY_CONVARS_START: &str = "// ================ CORE QUALITY ================";

const DMM_START: &str = "// Deadlock Mod Manager - Start";
const DMM_END: &str = "// Deadlock Mod Manager - End";

/// Ключи `SceneSystem`, отвечающие за отключение теней/тумана/tonemapping даже при 2-way merge.
const SCENE_SYSTEM_GRAPHICS_KEYS: [&str; 13] = [
    "FogCachedShadowAtlasWidth",
    "FogCachedShadowAtlasHeight",
    "FogCachedShadowTileSize",
    "CSMCascadeResolution",
    "SunLightManagerCount",
    "SunLightManagerCountTools",
    "DefaultShadowTextureWidth",
    "DefaultShadowTextureHeight",
    "DynamicShadowResolution",
    "NonTexturedGradientFog",
    "CubemapFog",
    "VolumetricFog",
    "Tonemapping",
];

/// Служебные поля верхнего уровня `GameInfo`, которые никогда не перетираются из пресета.
const PROTECTED_GAMEINFO_KEYS: [&str; 6] = [
    "PGIVersion",
    "DisallowGameInfoConditionals",
    "game",
    "title",
    "type",
    "GameData",
];

// ============================================================ пути и состояние

pub fn gameinfo_path(game_path: &Path) -> PathBuf {
    game_path.join("game").join("citadel").join("gameinfo.gi")
}

pub fn vanilla_backup_path(game_path: &Path) -> PathBuf {
    game_path
        .join("game")
        .join("citadel")
        .join("gameinfo_vanilla_backup.gi")
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct GameInfoState {
    pub active_preset: String,
    pub last_applied_sha256: Option<String>,
    pub last_applied_mtime_secs: Option<u64>,
    pub last_applied_build_id: Option<String>,
    pub last_vanilla_sha256: Option<String>,
}

impl GameInfoState {
    pub fn load() -> Self {
        Self::load_from(&Settings::gameinfo_state_path())
    }

    pub fn load_from(path: &Path) -> Self {
        read_text_lossy_with_retry(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Settings::gameinfo_state_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        atomic_write_with_retry(path, json.as_bytes()).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct SyncReport {
    pub message: String,
    pub is_patch_adaptation: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SyncStatus {
    GameNotFound,
    UpToDate {
        active_preset: String,
        sha256_short: String,
        build_id: Option<String>,
        has_vanilla_backup: bool,
    },
    ExternalChangeDetected {
        reason: String,
        build_id: Option<String>,
        has_vanilla_backup: bool,
    },
}

// ============================================================ мониторинг и применение

/// Быстрый поиск `appmanifest_1422450.acf` без разбора `libraryfolders.vdf`, если манифест рядом.
fn steam_manifest_path(steam_path: &Path, game_path: &Path) -> Option<PathBuf> {
    let manifest_name = format!("appmanifest_{}.acf", steam::APP_ID);

    if let Some(steamapps) = game_path.parent().and_then(|p| p.parent()) {
        let direct = steamapps.join(&manifest_name);
        if direct.is_file() {
            return Some(direct);
        }
    }

    if steam_path.exists() {
        for lib in steam::library_folders(steam_path) {
            let candidate = lib.join("steamapps").join(&manifest_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn read_steam_build_id(steam_path: &Path, game_path: &Path) -> Option<String> {
    let manifest = steam_manifest_path(steam_path, game_path)?;
    let file = crate::fs_safe::open_file_with_retry(&manifest).ok()?;
    let mut reader = BufReader::with_capacity(16 * 1024, file);
    let mut line_buf = Vec::with_capacity(256);

    loop {
        line_buf.clear();
        let n = reader.read_until(b'\n', &mut line_buf).ok()?;
        if n == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&line_buf);
        if let Some(after_key) = strip_quoted_key(line.trim(), "buildid") {
            if let Some(val) = parse_quoted_value(after_key.trim_start()) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

pub fn inspect_status(steam_path: &Path, game_path: &Path, state: &GameInfoState) -> SyncStatus {
    let gi_path = gameinfo_path(game_path);
    if !gi_path.exists() {
        return SyncStatus::GameNotFound;
    }

    let has_vanilla_backup = vanilla_backup_path(game_path).exists();
    let build_id = read_steam_build_id(steam_path, game_path);

    let Ok(bytes) = read_bytes_with_retry(&gi_path) else {
        return SyncStatus::GameNotFound;
    };
    let cur_sha = sha256_hex(&bytes);
    let sha256_short = cur_sha.chars().take(12).collect::<String>();
    let text = String::from_utf8_lossy(&bytes);
    let first_line = text.trim_start_matches('\u{feff}').lines().next().unwrap_or("").trim();

    if let Some(ref expected) = state.last_applied_sha256 {
        if expected != &cur_sha {
            return SyncStatus::ExternalChangeDetected {
                reason: "Файл gameinfo.gi изменён извне (вероятно, патч Valve или проверка файлов)"
                    .into(),
                build_id,
                has_vanilla_backup,
            };
        }
    } else if !first_line.starts_with(SMART_MARKER_PREFIX) {
        return SyncStatus::ExternalChangeDetected {
            reason: "Профиль ещё не адаптирован через Smart Merge".into(),
            build_id,
            has_vanilla_backup,
        };
    }

    if let Some(ref cur_build) = build_id {
        if state.last_applied_build_id.as_deref() != Some(cur_build.as_str()) {
            return SyncStatus::ExternalChangeDetected {
                reason: format!("Обнаружен новый билд Steam ({cur_build})"),
                build_id,
                has_vanilla_backup,
            };
        }
    }

    let active = if state.active_preset.is_empty() {
        gameinfo_presets::DEFAULT_PRESET_SHAURMEAW.to_owned()
    } else {
        state.active_preset.clone()
    };

    SyncStatus::UpToDate {
        active_preset: active,
        sha256_short,
        build_id,
        has_vanilla_backup,
    }
}

pub fn apply_preset(s: &Settings, preset_name: &str) -> Result<SyncReport, String> {
    apply_preset_in_dirs(
        &Settings::gameinfo_presets_dir(),
        &Settings::gameinfo_backups_dir(),
        &Settings::gameinfo_state_path(),
        Path::new(&s.steam_path),
        Path::new(&s.game_path),
        &s.gameinfo,
        preset_name,
    )
}

pub fn check_and_adapt_if_patched(s: &Settings) -> Result<Option<SyncReport>, String> {
    check_and_adapt_in_dirs(
        &Settings::gameinfo_presets_dir(),
        &Settings::gameinfo_backups_dir(),
        &Settings::gameinfo_state_path(),
        Path::new(&s.steam_path),
        Path::new(&s.game_path),
        &s.gameinfo,
    )
}

pub fn apply_preset_in_dirs(
    presets_dir: &Path,
    backups_dir: &Path,
    state_path: &Path,
    steam_path: &Path,
    game_path: &Path,
    cfg: &GameInfoConfig,
    preset_name: &str,
) -> Result<SyncReport, String> {
    let gi_path = gameinfo_path(game_path);
    if !gi_path.exists() {
        return Err("Файл game\\citadel\\gameinfo.gi не найден — проверьте путь к Deadlock".into());
    }

    let preset_text = gameinfo_presets::read_preset_in(presets_dir, preset_name)?;

    // Перед любой перезаписью создаём бэкап с датой и временем в backups/
    let _ = gameinfo_presets::create_timestamped_backup_in(backups_dir, game_path)?;

    let current = read_text_lossy_with_retry(&gi_path)
        .map_err(|e| format!("Не удалось прочитать gameinfo.gi: {e}"))?;
    let bak_path = vanilla_backup_path(game_path);

    let (new_vanilla, old_vanilla) = if !is_custom_file(&current) {
        // Текущий файл в игре — свежая официальная ваниль Valve!
        let old = read_text_lossy_with_retry(&bak_path)
            .ok()
            .filter(|b| !is_custom_file(b));
        atomic_write_with_retry(&bak_path, current.as_bytes())
            .map_err(|e| format!("Не удалось сохранить gameinfo_vanilla_backup.gi: {e}"))?;
        (current, old)
    } else if bak_path.exists() {
        let bak = read_text_lossy_with_retry(&bak_path)
            .map_err(|e| format!("Не удалось прочитать gameinfo_vanilla_backup.gi: {e}"))?;
        if !is_custom_file(&bak) {
            (bak.clone(), Some(bak))
        } else {
            // Бэкап был повреждён кастомным файлом — берём встроенный официальный оригинал Valve
            let builtin = crate::embedded::valve_vanilla_gi().to_owned();
            let _ = atomic_write_with_retry(&bak_path, builtin.as_bytes());
            (builtin, None)
        }
    } else {
        // У пользователя уже стоял кастомный/модовый gameinfo.gi, а ванильного бэкапа ещё не было:
        // сохраняем встроенный официальный оригинал Valve как чистую базу
        let builtin = crate::embedded::valve_vanilla_gi().to_owned();
        let _ = atomic_write_with_retry(&bak_path, builtin.as_bytes());
        (builtin, None)
    };

    let build_id = read_steam_build_id(steam_path, game_path);
    let clean_vanilla_mode = is_clean_vanilla_preset(preset_name, &preset_text);
    let merged = smart_merge(
        &new_vanilla,
        &preset_text,
        old_vanilla.as_deref(),
        cfg,
        preset_name,
        build_id.as_deref(),
    )?;

    atomic_write_with_retry(&gi_path, merged.as_bytes())
        .map_err(|e| format!("Не удалось записать gameinfo.gi: {e}"))?;

    let applied_sha = sha256_hex(merged.as_bytes());
    let vanilla_sha = sha256_hex(new_vanilla.as_bytes());
    let mtime = file_mtime_secs(&gi_path);

    let state = GameInfoState {
        active_preset: preset_name.to_owned(),
        last_applied_sha256: Some(applied_sha),
        last_applied_mtime_secs: mtime,
        last_applied_build_id: build_id,
        last_vanilla_sha256: Some(vanilla_sha),
    };
    state.save_to(state_path)?;

    let message = if clean_vanilla_mode {
        format!("Записан официальный gameinfo.gi от Valve (без модов, профиль [{preset_name}])")
    } else {
        format!("Профиль [{preset_name}] адаптирован и применён к gameinfo.gi")
    };

    Ok(SyncReport {
        message,
        is_patch_adaptation: false,
    })
}

pub fn check_and_adapt_in_dirs(
    presets_dir: &Path,
    backups_dir: &Path,
    state_path: &Path,
    steam_path: &Path,
    game_path: &Path,
    cfg: &GameInfoConfig,
) -> Result<Option<SyncReport>, String> {
    if !cfg.enabled {
        return Ok(None);
    }

    let state = GameInfoState::load_from(state_path);
    let status = inspect_status(steam_path, game_path, &state);
    let SyncStatus::ExternalChangeDetected { .. } = status else {
        return Ok(None);
    };

    let gi_path = gameinfo_path(game_path);
    if !gi_path.exists() {
        return Ok(None);
    }

    // Определяем активный профиль
    let preset_name = resolve_active_preset_name(presets_dir, cfg, &state)?;
    let preset_text = gameinfo_presets::read_preset_in(presets_dir, &preset_name)?;

    // 1. Бэкап текущего файла с датой и временем в backups/
    let _ = gameinfo_presets::create_timestamped_backup_in(backups_dir, game_path)?;

    // 2. Сравниваем с предыдущей ванилью (old_vanilla) и сохраняем новую официальную в gameinfo_vanilla_backup.gi
    let bak_path = vanilla_backup_path(game_path);
    let old_vanilla = read_text_lossy_with_retry(&bak_path)
        .ok()
        .filter(|b| !is_custom_file(b));
    let current = read_text_lossy_with_retry(&gi_path)
        .map_err(|e| format!("Не удалось прочитать gameinfo.gi: {e}"))?;

    let new_vanilla = if !is_custom_file(&current) {
        atomic_write_with_retry(&bak_path, current.as_bytes())
            .map_err(|e| format!("Не удалось сохранить gameinfo_vanilla_backup.gi: {e}"))?;
        if is_clean_vanilla_preset(&preset_name, &preset_text) {
            let _ = gameinfo_presets::save_preset_in(
                presets_dir,
                gameinfo_presets::DEFAULT_PRESET_VALVE,
                &current,
                None,
            );
        }
        current
    } else if let Some(ref bak) = old_vanilla {
        bak.clone()
    } else {
        let builtin = crate::embedded::valve_vanilla_gi().to_owned();
        let _ = atomic_write_with_retry(&bak_path, builtin.as_bytes());
        builtin
    };

    let build_id = read_steam_build_id(steam_path, game_path);
    let merged = smart_merge(
        &new_vanilla,
        &preset_text,
        old_vanilla.as_deref(),
        cfg,
        &preset_name,
        build_id.as_deref(),
    )?;

    atomic_write_with_retry(&gi_path, merged.as_bytes())
        .map_err(|e| format!("Не удалось записать gameinfo.gi: {e}"))?;

    let new_state = GameInfoState {
        active_preset: preset_name.clone(),
        last_applied_sha256: Some(sha256_hex(merged.as_bytes())),
        last_applied_mtime_secs: file_mtime_secs(&gi_path),
        last_applied_build_id: build_id,
        last_vanilla_sha256: Some(sha256_hex(new_vanilla.as_bytes())),
    };
    new_state.save_to(state_path)?;

    Ok(Some(SyncReport {
        message: format!(
            "Обнаружен патч Deadlock. Ваш профиль [{preset_name}] успешно адаптирован и применен"
        ),
        is_patch_adaptation: true,
    }))
}

pub fn restore_vanilla(game_path: &Path) -> Result<String, String> {
    let bak = vanilla_backup_path(game_path);
    let dst = gameinfo_path(game_path);
    if !bak.exists() {
        return Err("Официальная копия gameinfo_vanilla_backup.gi пока не создана".into());
    }

    let _ = gameinfo_presets::create_timestamped_backup(game_path)?;
    let bytes = read_bytes_with_retry(&bak)
        .map_err(|e| format!("Не удалось прочитать gameinfo_vanilla_backup.gi: {e}"))?;
    atomic_write_with_retry(&dst, &bytes)
        .map_err(|e| format!("Не удалось восстановить ванильный gameinfo.gi: {e}"))?;

    // Обновляем состояние под чистый оригинал без лишнего чтения с диска
    let mut state = GameInfoState::load();
    state.last_applied_sha256 = Some(sha256_hex(&bytes));
    state.last_applied_mtime_secs = file_mtime_secs(&dst);
    let _ = state.save();
    Ok("Восстановлен оригинальный официальный gameinfo.gi (из gameinfo_vanilla_backup.gi)".into())
}

/// Скачивает свежий официальный `gameinfo.gi` Valve со SteamDB, обновляет
/// `gameinfo_vanilla_backup.gi` и пресет «Официальный от Valve (без модов)».
pub fn refresh_official_vanilla_from_steamdb(game_path: &Path) -> Result<String, String> {
    let official = gameinfo_presets::fetch_official_valve_from_steamdb()?;
    if game_path.join("game").join("citadel").exists() {
        let bak = vanilla_backup_path(game_path);
        atomic_write_with_retry(&bak, official.as_bytes())
            .map_err(|e| format!("Не удалось обновить gameinfo_vanilla_backup.gi: {e}"))?;
    }
    Ok("Свежий официальный gameinfo.gi Valve скачан со SteamDB".into())
}

fn resolve_active_preset_name(
    presets_dir: &Path,
    cfg: &GameInfoConfig,
    state: &GameInfoState,
) -> Result<String, String> {
    let migrated_cfg = if cfg.active_preset == gameinfo_presets::LEGACY_PRESET_VANILLA {
        gameinfo_presets::DEFAULT_PRESET_VALVE
    } else {
        cfg.active_preset.as_str()
    };
    let migrated_state = if state.active_preset == gameinfo_presets::LEGACY_PRESET_VANILLA {
        gameinfo_presets::DEFAULT_PRESET_VALVE
    } else {
        state.active_preset.as_str()
    };

    let candidates = [
        migrated_cfg,
        migrated_state,
        gameinfo_presets::DEFAULT_PRESET_SHAURMEAW,
    ];
    for c in candidates {
        if !c.trim().is_empty() && gameinfo_presets::read_preset_in(presets_dir, c).is_ok() {
            return Ok(c.to_owned());
        }
    }
    gameinfo_presets::list_presets_in(presets_dir)
        .into_iter()
        .next()
        .map(|e| e.name)
        .ok_or_else(|| "В папке presets/ нет ни одного профиля gameinfo.gi".to_owned())
}

pub fn is_clean_vanilla_preset(preset_name: &str, preset_text: &str) -> bool {
    preset_name == gameinfo_presets::DEFAULT_PRESET_VALVE
        || preset_name == gameinfo_presets::LEGACY_PRESET_VANILLA
        || preset_text.contains(CLEAN_VANILLA_MARKER)
}

fn is_custom_file(content: &str) -> bool {
    let clean = content.trim_start_matches('\u{feff}');
    let first = clean.lines().next().unwrap_or("").trim();
    if first.starts_with(SMART_MARKER_PREFIX)
        || first.starts_with("// [ShaurMeaw GameInfo Patcher]")
        || first.eq_ignore_ascii_case(LEGACY_AUTHOR_HEADER)
    {
        return true;
    }

    if clean.contains(LEGACY_CONVARS_START) {
        return true;
    }

    // Если в SearchPaths (не в комментариях) вписаны пути модов
    for line in clean.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let code = trimmed.split("//").next().unwrap_or("").to_ascii_lowercase();
        if code.contains("citadel/addons") || code.contains("citadel/dmm_localization_default") {
            return true;
        }
    }

    false
}

// ============================================================ фоновый наблюдатель

struct WatcherState {
    steam_path: String,
    game_path: String,
    gameinfo: GameInfoConfig,
    game_running: bool,
    revision: u64,
    shutdown: bool,
}

pub struct GameInfoWatcher {
    shared: Arc<Mutex<WatcherState>>,
    notify: Arc<Mutex<Option<Result<SyncReport, String>>>>,
    wake: Option<Arc<WinEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl GameInfoWatcher {
    pub fn spawn(
        initial: &Settings,
        ctx: eframe::egui::Context,
        window_minimized: Arc<AtomicBool>,
        game_running_shared: Arc<AtomicBool>,
    ) -> Self {
        let shared = Arc::new(Mutex::new(WatcherState {
            steam_path: initial.steam_path.clone(),
            game_path: initial.game_path.clone(),
            gameinfo: initial.gameinfo.clone(),
            game_running: game_running_shared.load(Ordering::Acquire),
            revision: 1,
            shutdown: false,
        }));
        let notify = Arc::new(Mutex::new(None));
        let wake = WinEvent::new().map(Arc::new);

        let shared_bg = Arc::clone(&shared);
        let notify_bg = Arc::clone(&notify);
        let min_flag = Arc::clone(&window_minimized);
        let game_bg = Arc::clone(&game_running_shared);
        let wake_bg = wake.as_ref().map(Arc::clone);

        let worker = std::thread::Builder::new()
            .name("shaurmeaw-gameinfo".to_owned())
            .spawn(move || {
                let mut handled_rev = 0u64;
                let mut steam_path = String::new();
                let mut game_path = String::new();
                let mut cfg_snapshot = GameInfoConfig::default();

                let mut cached_key: (String, String) = (String::new(), String::new());
                let mut cached_manifest_path: Option<PathBuf> = None;
                let mut gi_dir_watch: Option<DirChangeHandle> = None;
                let mut manifest_dir_watch: Option<DirChangeHandle> = None;
                let mut last_seen_gi_mtime: Option<u64> = None;
                let mut last_seen_manifest_mtime: Option<u64> = None;
                let mut retry_after_error = false;

                loop {
                    let sleep_forever = {
                        let guard = shared_bg.lock().unwrap_or_else(|e| e.into_inner());
                        if guard.shutdown {
                            break;
                        }
                        if !guard.gameinfo.enabled
                            || !guard.gameinfo.auto_watch_updates
                            || guard.game_running
                            || game_bg.load(Ordering::Acquire)
                        {
                            true
                        } else {
                            if guard.revision != handled_rev {
                                steam_path.clone_from(&guard.steam_path);
                                game_path.clone_from(&guard.game_path);
                                cfg_snapshot = guard.gameinfo.clone();
                                handled_rev = guard.revision;
                            }
                            false
                        }
                    };

                    if sleep_forever {
                        // Во время игры или при отключённом вотчере закрываем хэндлы каталогов и спим в ядре
                        gi_dir_watch = None;
                        manifest_dir_watch = None;
                        if let Some(ref ev) = wake_bg {
                            let _ = wait_event_or_dirs(ev, &[], WAIT_INFINITE);
                        } else {
                            std::thread::sleep(Duration::from_secs(5));
                        }
                        continue;
                    }

                    if cached_key.0 != steam_path || cached_key.1 != game_path {
                        cached_key = (steam_path.clone(), game_path.clone());
                        cached_manifest_path =
                            steam_manifest_path(Path::new(&steam_path), Path::new(&game_path));
                        gi_dir_watch = None;
                        manifest_dir_watch = None;
                        last_seen_gi_mtime = None;
                        last_seen_manifest_mtime = None;
                    } else if cached_manifest_path.is_none() {
                        cached_manifest_path =
                            steam_manifest_path(Path::new(&steam_path), Path::new(&game_path));
                    }

                    let gi = gameinfo_path(Path::new(&game_path));
                    if gi_dir_watch.is_none() {
                        if let Some(parent) = gi.parent() {
                            gi_dir_watch =
                                DirChangeHandle::open(parent, false, NOTIFY_FILE_AND_WRITE);
                        }
                    }
                    if manifest_dir_watch.is_none() {
                        if let Some(m_parent) =
                            cached_manifest_path.as_deref().and_then(Path::parent)
                        {
                            if gi.parent() != Some(m_parent) {
                                manifest_dir_watch =
                                    DirChangeHandle::open(m_parent, false, NOTIFY_FILE_AND_WRITE);
                            }
                        }
                    }

                    if gi.exists() {
                        // Сравниваем только дешёвые mtime без чтения файлов
                        let cur_gi_mtime = file_mtime_secs(&gi);
                        let cur_manifest_mtime =
                            cached_manifest_path.as_deref().and_then(file_mtime_secs);
                        let unchanged = last_seen_gi_mtime.is_some()
                            && cur_gi_mtime == last_seen_gi_mtime
                            && cur_manifest_mtime == last_seen_manifest_mtime;

                        if !unchanged {
                            match check_and_adapt_in_dirs(
                                &Settings::gameinfo_presets_dir(),
                                &Settings::gameinfo_backups_dir(),
                                &Settings::gameinfo_state_path(),
                                Path::new(&steam_path),
                                Path::new(&game_path),
                                &cfg_snapshot,
                            ) {
                                Ok(Some(report)) => {
                                    last_seen_gi_mtime = file_mtime_secs(&gi);
                                    last_seen_manifest_mtime =
                                        cached_manifest_path.as_deref().and_then(file_mtime_secs);
                                    retry_after_error = false;
                                    *notify_bg.lock().unwrap_or_else(|e| e.into_inner()) =
                                        Some(Ok(report));
                                    if !min_flag.load(Ordering::Relaxed) {
                                        ctx.request_repaint();
                                    }
                                }
                                Ok(None) => {
                                    last_seen_gi_mtime = cur_gi_mtime;
                                    last_seen_manifest_mtime = cur_manifest_mtime;
                                    retry_after_error = false;
                                }
                                Err(e) => {
                                    // Сбрасываем кэш mtime, чтобы вотчер не пропустил патч после снятия блокировки Steam
                                    last_seen_gi_mtime = None;
                                    last_seen_manifest_mtime = None;
                                    retry_after_error = true;
                                    *notify_bg.lock().unwrap_or_else(|e| e.into_inner()) =
                                        Some(Err(e));
                                    if !min_flag.load(Ordering::Relaxed) {
                                        ctx.request_repaint();
                                    }
                                }
                            }
                        }
                    }

                    if let Some(ref ev) = wake_bg {
                        let timeout_ms = if retry_after_error {
                            2_000
                        } else if gi_dir_watch.is_some() {
                            60_000
                        } else {
                            15_000
                        };
                        let outcome = match (gi_dir_watch.as_ref(), manifest_dir_watch.as_ref()) {
                            (Some(a), Some(b)) => wait_event_or_dirs(ev, &[a, b], timeout_ms),
                            (Some(a), None) => wait_event_or_dirs(ev, &[a], timeout_ms),
                            (None, Some(b)) => wait_event_or_dirs(ev, &[b], timeout_ms),
                            (None, None) => wait_event_or_dirs(ev, &[], 15_000),
                        };

                        match outcome {
                            WaitOutcome::DirChanged(_) => {
                                // Короткий дебаунс, чтобы Steam успел завершить запись файла
                                let _ = wait_event_or_dirs(ev, &[], 150);
                            }
                            WaitOutcome::DirClosed(idx) => {
                                if idx == 0 && gi_dir_watch.is_some() {
                                    gi_dir_watch = None;
                                } else {
                                    manifest_dir_watch = None;
                                }
                            }
                            WaitOutcome::Woken | WaitOutcome::Timeout | WaitOutcome::Error => {}
                        }
                    } else {
                        std::thread::sleep(Duration::from_secs(8));
                    }
                }
            })
            .ok();

        Self {
            shared,
            notify,
            wake,
            worker,
        }
    }

    pub fn wake_handle(&self) -> Option<Arc<WinEvent>> {
        self.wake.as_ref().map(Arc::clone)
    }

    pub fn sync_config(&self, settings: &Settings, game_running: bool) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.steam_path.clone_from(&settings.steam_path);
            guard.game_path.clone_from(&settings.game_path);
            guard.gameinfo = settings.gameinfo.clone();
            guard.game_running = game_running;
            guard.revision = guard.revision.wrapping_add(1);
        }
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    pub fn take_notification(&self) -> Option<Result<SyncReport, String>> {
        self.notify.lock().ok().and_then(|mut g| g.take())
    }

    pub fn shutdown(&mut self) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.shutdown = true;
        }
        if let Some(ref w) = self.wake {
            w.signal();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for GameInfoWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================ Smart Merge (KeyValues / VDF)

/// Умное слияние: берёт за основу свежий официальный `new_vanilla` и переносит
/// кастомные отличия из `preset_text` (SearchPaths, SceneSystem, ConVars, параметры GameInfo).
pub fn smart_merge(
    new_vanilla: &str,
    preset_text: &str,
    old_vanilla: Option<&str>,
    cfg: &GameInfoConfig,
    preset_name: &str,
    build_id: Option<&str>,
) -> Result<String, String> {
    let clean_vanilla = strip_header_lines(&normalize_nl(new_vanilla));
    let clean_preset = strip_header_lines(&normalize_nl(preset_text));
    let clean_old = old_vanilla.map(|s| strip_header_lines(&normalize_nl(s)));

    // Чистый официальный режим Valve: отдаём чистый официальный файл без шапок SmartMerge и модов
    if is_clean_vanilla_preset(preset_name, &clean_preset) || clean_preset.trim().is_empty() {
        let mut out = clean_vanilla.trim_start_matches('\n').to_owned();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        return Ok(out.replace('\n', "\r\n"));
    }

    let build_label = build_id.unwrap_or("unknown");
    let header = format!("{SMART_MARKER_PREFIX} preset=\"{preset_name}\" build={build_label}\n");

    let mut result = clean_vanilla;

    // 1. PGIVersion (только если пользователь явно попросил комментировать)
    if cfg.comment_pgi_version {
        result = comment_out_gameinfo_pgi(&result);
    }

    // 2. Верхний уровень GameInfo (например, tonemapping 0)
    result = merge_gameinfo_top_level(&result, &clean_preset, clean_old.as_deref());

    // 3. Слой SearchPaths (переносим добавленные пути модов из пресета)
    result = merge_search_paths(&result, &clean_preset)?;

    // 4. Слой SceneSystem (3-way или 2-way перенос обнулённых/изменённых параметров)
    result = merge_scene_system(&result, &clean_preset, clean_old.as_deref())?;

    // 4.1. Дополнительные FPS-секции движка (RenderSystem, WorldRenderer, AnimationSystem, Particles)
    for (section_name, start_marker, end_marker) in [
        (
            "RenderSystem",
            "// [ShaurMeaw RenderSystem - Start]",
            "// [ShaurMeaw RenderSystem - End]",
        ),
        (
            "WorldRenderer",
            "// [ShaurMeaw WorldRenderer - Start]",
            "// [ShaurMeaw WorldRenderer - End]",
        ),
        (
            "AnimationSystem",
            "// [ShaurMeaw AnimationSystem - Start]",
            "// [ShaurMeaw AnimationSystem - End]",
        ),
        (
            "Particles",
            "// [ShaurMeaw Particles - Start]",
            "// [ShaurMeaw Particles - End]",
        ),
    ] {
        result = merge_optional_kv_section(
            &result,
            &clean_preset,
            clean_old.as_deref(),
            section_name,
            start_marker,
            end_marker,
        );
    }

    // 5. Слой ConVars (перенос добавленных/изменённых ConVars с сохранением новых Valve-полей)
    result = merge_convars(&result, &clean_preset, clean_old.as_deref(), cfg)?;

    let mut final_text = format!(
        "{header}{LEGACY_AUTHOR_HEADER}\n\n{}",
        result.trim_start_matches('\n')
    );
    if !final_text.ends_with('\n') {
        final_text.push('\n');
    }
    Ok(final_text.replace('\n', "\r\n"))
}

fn normalize_nl(s: &str) -> String {
    s.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn strip_header_lines(text: &str) -> String {
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.peek() {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with(SMART_MARKER_PREFIX)
            || t.starts_with("// [ShaurMeaw GameInfo Patcher]")
            || t.eq_ignore_ascii_case(LEGACY_AUTHOR_HEADER)
        {
            lines.next();
        } else {
            break;
        }
    }
    lines.collect::<Vec<_>>().join("\n") + if text.ends_with('\n') { "\n" } else { "" }
}

fn comment_out_gameinfo_pgi(text: &str) -> String {
    let Some((gi_s, gi_e)) = find_section_body(text, "GameInfo") else {
        return text.to_owned();
    };
    let body = map_top_level_lines(&text[gi_s..gi_e], |line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            return line.to_owned();
        }
        if starts_with_key_token(trimmed, "PGIVersion")
            || starts_with_key_token(trimmed, "DisallowGameInfoConditionals")
        {
            let indent_len = line.len() - trimmed.len();
            format!("{}//{}", &line[..indent_len], trimmed)
        } else {
            line.to_owned()
        }
    });
    format!("{}{}{}", &text[..gi_s], body, &text[gi_e..])
}

fn merge_gameinfo_top_level(vanilla: &str, preset: &str, old_vanilla: Option<&str>) -> String {
    let Some((v_s, v_e)) = find_section_body(vanilla, "GameInfo") else {
        return vanilla.to_owned();
    };
    let Some((p_s, p_e)) = find_section_body(preset, "GameInfo") else {
        return vanilla.to_owned();
    };

    let p_kvs = extract_top_level_kvs(&preset[p_s..p_e]);
    let v_kvs = extract_top_level_kvs_map(&vanilla[v_s..v_e]);
    let o_kvs = old_vanilla
        .and_then(|o| find_section_body(o, "GameInfo").map(|(s, e)| extract_top_level_kvs_map(&o[s..e])));

    let mut overrides: HashMap<String, (String, String)> = HashMap::new();
    for (key, val, _) in p_kvs {
        if PROTECTED_GAMEINFO_KEYS
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&key))
        {
            continue;
        }
        let kl = key.to_ascii_lowercase();
        let should_apply = match &o_kvs {
            Some(old) => old.get(&kl).map(String::as_str) != Some(val.as_str()),
            None => {
                v_kvs.get(&kl).map(String::as_str) != Some(val.as_str())
                    && (val == "0" || val.eq_ignore_ascii_case("false"))
            }
        };
        if should_apply {
            overrides.insert(kl, (key, val));
        }
    }

    if overrides.is_empty() {
        return vanilla.to_owned();
    }

    let new_body = map_top_level_lines(&vanilla[v_s..v_e], |line| {
        if let Some((k, _)) = parse_kv_pair(line) {
            if let Some((_, new_val)) = overrides.get(&k.to_ascii_lowercase()) {
                if let Some(replaced) = replace_kv_line(line, &k, new_val) {
                    return replaced;
                }
            }
        }
        line.to_owned()
    });

    format!("{}{}{}", &vanilla[..v_s], new_body, &vanilla[v_e..])
}

fn merge_search_paths(vanilla: &str, preset: &str) -> Result<String, String> {
    let Some((p_gi_s, p_gi_e)) = find_section_body(preset, "GameInfo") else {
        return Ok(vanilla.to_owned());
    };
    let Some((p_fs_s, p_fs_e)) = find_section_body(&preset[p_gi_s..p_gi_e], "FileSystem") else {
        return Ok(vanilla.to_owned());
    };
    let Some((p_sp_s, p_sp_e)) =
        find_section_body(&preset[p_gi_s + p_fs_s..p_gi_s + p_fs_e], "SearchPaths")
    else {
        return Ok(vanilla.to_owned());
    };
    let preset_sp = &preset[p_gi_s + p_fs_s + p_sp_s..p_gi_s + p_fs_s + p_sp_e];
    let preset_paths = extract_top_level_kvs(preset_sp);
    if preset_paths.is_empty() {
        return Ok(vanilla.to_owned());
    }

    let (v_gi_s, v_gi_e) = find_section_body(vanilla, "GameInfo")
        .ok_or_else(|| "В официальном gameinfo.gi не найдена секция GameInfo".to_owned())?;
    let v_gi_body = &vanilla[v_gi_s..v_gi_e];

    let (v_fs_s, v_fs_e) = find_section_body(v_gi_body, "FileSystem")
        .ok_or_else(|| "В официальном gameinfo.gi не найдена секция FileSystem".to_owned())?;
    let fs_abs_s = v_gi_s + v_fs_s;
    let fs_abs_e = v_gi_s + v_fs_e;
    let v_fs_body = &vanilla[fs_abs_s..fs_abs_e];

    let (v_sp_s, v_sp_e) = find_section_body(v_fs_body, "SearchPaths")
        .ok_or_else(|| "В FileSystem не найдена секция SearchPaths".to_owned())?;
    let v_sp_body = &v_fs_body[v_sp_s..v_sp_e];

    let existing_set: HashSet<(String, String)> = extract_top_level_kvs(v_sp_body)
        .into_iter()
        .map(|(k, v, _)| (k.to_ascii_lowercase(), v.to_ascii_lowercase()))
        .collect();

    let indent = detect_line_indent(v_sp_body, "Mod").unwrap_or("            ");
    let mut to_insert = String::new();
    for (k, v, _) in preset_paths {
        let pair = (k.to_ascii_lowercase(), v.to_ascii_lowercase());
        if !existing_set.contains(&pair) {
            let _ = writeln!(to_insert, "{indent}{k:<20}{v}");
        }
    }

    if to_insert.is_empty() {
        return Ok(vanilla.to_owned());
    }

    let new_sp_body = if let Some(mod_pos) = find_top_level_key_line_start(v_sp_body, "Mod") {
        format!("{}{}{}", &v_sp_body[..mod_pos], to_insert, &v_sp_body[mod_pos..])
    } else if let Some(nl) = v_sp_body.find('\n') {
        format!("{}\n{}{}", &v_sp_body[..nl], to_insert, &v_sp_body[nl + 1..])
    } else {
        format!("\n{to_insert}{v_sp_body}")
    };

    let mut new_fs_body = format!("{}{}{}", &v_fs_body[..v_sp_s], new_sp_body, &v_fs_body[v_sp_e..]);

    if !new_fs_body.contains(DMM_START) {
        if let Some(sp_key_pos) = find_top_level_key_line_start(&new_fs_body, "SearchPaths") {
            if let Some((_, sp_end)) = find_section_body(&new_fs_body, "SearchPaths") {
                let after_brace = new_fs_body[sp_end..]
                    .find('\n')
                    .map(|off| sp_end + off + 1)
                    .unwrap_or(new_fs_body.len());
                new_fs_body = format!(
                    "{}{DMM_START}\n{}{DMM_END}\n{}",
                    &new_fs_body[..sp_key_pos],
                    &new_fs_body[sp_key_pos..after_brace],
                    &new_fs_body[after_brace..]
                );
            }
        }
    }

    Ok(format!(
        "{}{}{}",
        &vanilla[..fs_abs_s],
        new_fs_body,
        &vanilla[fs_abs_e..]
    ))
}

fn merge_scene_system(
    vanilla: &str,
    preset: &str,
    old_vanilla: Option<&str>,
) -> Result<String, String> {
    let Some((p_gi_s, p_gi_e)) = find_section_body(preset, "GameInfo") else {
        return Ok(vanilla.to_owned());
    };
    let Some((p_ss_s, p_ss_e)) = find_section_body(&preset[p_gi_s..p_gi_e], "SceneSystem") else {
        return Ok(vanilla.to_owned());
    };
    let p_kvs = extract_top_level_kvs(&preset[p_gi_s + p_ss_s..p_gi_s + p_ss_e]);
    if p_kvs.is_empty() {
        return Ok(vanilla.to_owned());
    }

    let (v_gi_s, v_gi_e) = find_section_body(vanilla, "GameInfo")
        .ok_or_else(|| "В официальном gameinfo.gi не найдена секция GameInfo".to_owned())?;
    let Some((v_ss_s, v_ss_e)) = find_section_body(&vanilla[v_gi_s..v_gi_e], "SceneSystem") else {
        return Ok(vanilla.to_owned());
    };
    let ss_abs_s = v_gi_s + v_ss_s;
    let ss_abs_e = v_gi_s + v_ss_e;

    let mut ss_body = remove_marker_block(&vanilla[ss_abs_s..ss_abs_e], SCENE_START, SCENE_END);
    let v_kvs = extract_top_level_kvs_map(&ss_body);
    let o_kvs = old_vanilla.and_then(|o| {
        let (gs, ge) = find_section_body(o, "GameInfo")?;
        let (ss, se) = find_section_body(&o[gs..ge], "SceneSystem")?;
        Some(extract_top_level_kvs_map(&o[gs + ss..gs + se]))
    });

    // Собираем отличия, которые нужно применить к ванили
    let mut desired: Vec<(String, String, String)> = Vec::new();
    let mut desired_map: HashMap<String, String> = HashMap::new();
    for (k, val, raw_line) in p_kvs {
        let kl = k.to_ascii_lowercase();
        let take = match &o_kvs {
            Some(old) => old.get(&kl).map(String::as_str) != Some(val.as_str()),
            None => {
                let in_gfx_list = SCENE_SYSTEM_GRAPHICS_KEYS
                    .iter()
                    .any(|g| g.eq_ignore_ascii_case(&k));
                let zero_or_false = val == "0" || val.eq_ignore_ascii_case("false");
                let differs_from_vanilla = v_kvs.get(&kl).map(String::as_str) != Some(val.as_str());
                differs_from_vanilla && (in_gfx_list || zero_or_false || !v_kvs.contains_key(&kl))
            }
        };
        if take {
            if !desired_map.contains_key(&kl) {
                desired.push((k.clone(), val.clone(), raw_line));
            }
            desired_map.insert(kl, val);
        }
    }

    if desired.is_empty() {
        return Ok(vanilla.to_owned());
    }

    let mut applied_in_place = HashSet::new();
    ss_body = map_top_level_lines(&ss_body, |line| {
        if let Some((k, _)) = parse_kv_pair(line) {
            let kl = k.to_ascii_lowercase();
            if let Some(new_val) = desired_map.get(&kl) {
                if let Some(replaced) = replace_kv_line(line, &k, new_val) {
                    applied_in_place.insert(kl);
                    return replaced;
                }
            }
        }
        line.to_owned()
    });

    let missing: Vec<(String, String, String)> = desired
        .into_iter()
        .filter(|(k, _, _)| !applied_in_place.contains(&k.to_ascii_lowercase()))
        .collect();

    if !missing.is_empty() {
        let indent = detect_any_kv_indent(&ss_body).unwrap_or("        ");
        let mut block = String::new();
        let _ = writeln!(block, "{indent}{SCENE_START}");
        for (_, _, raw) in missing {
            let _ = writeln!(block, "{indent}{}", raw.trim());
        }
        let _ = writeln!(block, "{indent}{SCENE_END}");

        let trimmed_end = ss_body.trim_end_matches([' ', '\t', '\n']);
        let trailing_ws = &ss_body[trimmed_end.len()..];
        let closing_indent = trailing_ws.rsplit('\n').next().unwrap_or("    ");
        ss_body = format!("{trimmed_end}\n\n{block}{closing_indent}");
    }

    Ok(format!(
        "{}{}{}",
        &vanilla[..ss_abs_s],
        ss_body,
        &vanilla[ss_abs_e..]
    ))
}

fn merge_optional_kv_section(
    vanilla: &str,
    preset: &str,
    old_vanilla: Option<&str>,
    section_name: &str,
    start_marker: &str,
    end_marker: &str,
) -> String {
    let Some((p_gi_s, p_gi_e)) = find_section_body(preset, "GameInfo") else {
        return vanilla.to_owned();
    };
    let Some((p_sec_s, p_sec_e)) = find_section_body(&preset[p_gi_s..p_gi_e], section_name) else {
        return vanilla.to_owned();
    };
    let p_kvs = extract_top_level_kvs(&preset[p_gi_s + p_sec_s..p_gi_s + p_sec_e]);
    if p_kvs.is_empty() {
        return vanilla.to_owned();
    }

    let Some((v_gi_s, v_gi_e)) = find_section_body(vanilla, "GameInfo") else {
        return vanilla.to_owned();
    };
    let Some((v_sec_s, v_sec_e)) = find_section_body(&vanilla[v_gi_s..v_gi_e], section_name)
    else {
        return vanilla.to_owned();
    };
    let sec_abs_s = v_gi_s + v_sec_s;
    let sec_abs_e = v_gi_s + v_sec_e;

    let mut sec_body = remove_marker_block(&vanilla[sec_abs_s..sec_abs_e], start_marker, end_marker);
    let v_kvs = extract_top_level_kvs_map(&sec_body);
    let o_kvs = old_vanilla.and_then(|o| {
        let (gs, ge) = find_section_body(o, "GameInfo")?;
        let (ss, se) = find_section_body(&o[gs..ge], section_name)?;
        Some(extract_top_level_kvs_map(&o[gs + ss..gs + se]))
    });

    let mut desired: Vec<(String, String, String)> = Vec::new();
    let mut desired_map: HashMap<String, String> = HashMap::new();
    for (k, val, raw_line) in p_kvs {
        let kl = k.to_ascii_lowercase();
        let take = match &o_kvs {
            Some(old) => old.get(&kl).map(String::as_str) != Some(val.as_str()),
            None => {
                let differs_from_vanilla = v_kvs.get(&kl).map(String::as_str) != Some(val.as_str());
                let is_zero_or_false = val == "0" || val.eq_ignore_ascii_case("false");
                differs_from_vanilla && (!v_kvs.contains_key(&kl) || is_zero_or_false)
            }
        };
        if take {
            if !desired_map.contains_key(&kl) {
                desired.push((k.clone(), val.clone(), raw_line));
            }
            desired_map.insert(kl, val);
        }
    }

    if desired.is_empty() {
        return vanilla.to_owned();
    }

    let mut applied_in_place = HashSet::new();
    sec_body = map_top_level_lines(&sec_body, |line| {
        if let Some((k, _)) = parse_kv_pair(line) {
            let kl = k.to_ascii_lowercase();
            if let Some(new_val) = desired_map.get(&kl) {
                if let Some(replaced) = replace_kv_line(line, &k, new_val) {
                    applied_in_place.insert(kl);
                    return replaced;
                }
            }
        }
        line.to_owned()
    });

    let missing: Vec<(String, String, String)> = desired
        .into_iter()
        .filter(|(k, _, _)| !applied_in_place.contains(&k.to_ascii_lowercase()))
        .collect();

    if !missing.is_empty() {
        let indent = detect_any_kv_indent(&sec_body).unwrap_or("        ");
        let mut block = String::new();
        let _ = writeln!(block, "{indent}{start_marker}");
        for (_, _, raw) in missing {
            let _ = writeln!(block, "{indent}{}", raw.trim());
        }
        let _ = writeln!(block, "{indent}{end_marker}");

        let trimmed_end = sec_body.trim_end_matches([' ', '\t', '\n']);
        let trailing_ws = &sec_body[trimmed_end.len()..];
        let closing_indent = trailing_ws.rsplit('\n').next().unwrap_or("    ");
        sec_body = format!("{trimmed_end}\n\n{block}{closing_indent}");
    }

    format!(
        "{}{}{}",
        &vanilla[..sec_abs_s],
        sec_body,
        &vanilla[sec_abs_e..]
    )
}

fn collect_extra_convar_overrides(cfg: &GameInfoConfig) -> Vec<(&'static str, String)> {
    let mut extra = Vec::new();

    if !cfg.use_quick_tweaks {
        if let Some(fov) = cfg.fov_preset.aspect_ratio() {
            extra.push(("r_aspectratio", fov.to_owned()));
        }
    }
    if cfg.hide_hp_names {
        extra.push(("citadel_unit_status_hide_names", "true".to_owned()));
    }
    if cfg.bright_outlines {
        extra.push(("cl_glow_brightness", "1.5".to_owned()));
    }
    if cfg.no_flash_lights {
        extra.push(("r_muzzleflashbrightness", "0".to_owned()));
        extra.push(("r_citadel_disable_particle_lights", "true".to_owned()));
    }
    if cfg.no_camera_shake {
        extra.push(("citadel_camera_wobble_disable", "true".to_owned()));
        extra.push(("citadel_melee_shake_amplitude", "0".to_owned()));
        extra.push(("citadel_melee_shake_duration", "0".to_owned()));
    }
    if cfg.cpu_threads_boost {
        extra.push(("r_threaded_particles", "1".to_owned()));
        extra.push(("cl_threaded_bone_setup", "1".to_owned()));
        extra.push(("anim_allow_asynchronous_animation", "1".to_owned()));
        extra.push(("host_thread_mode", "2".to_owned()));
        extra.push(("r_shader_execute_only_pre_compiled", "1".to_owned()));
    }

    extra
}

fn merge_convars(
    vanilla: &str,
    preset: &str,
    old_vanilla: Option<&str>,
    cfg: &GameInfoConfig,
) -> Result<String, String> {
    let extra_overrides = collect_extra_convar_overrides(cfg);
    let p_cv_body = find_section_body(preset, "GameInfo")
        .and_then(|(p_gi_s, p_gi_e)| {
            find_section_body(&preset[p_gi_s..p_gi_e], "ConVars")
                .map(|(p_cv_s, p_cv_e)| &preset[p_gi_s + p_cv_s..p_gi_s + p_cv_e])
        })
        .unwrap_or("");

    if p_cv_body.is_empty() && !cfg.use_quick_tweaks && extra_overrides.is_empty() {
        return Ok(vanilla.to_owned());
    }

    let (v_gi_s, v_gi_e) = find_section_body(vanilla, "GameInfo")
        .ok_or_else(|| "В официальном gameinfo.gi не найдена секция GameInfo".to_owned())?;
    let Some((v_cv_s, v_cv_e)) = find_section_body(&vanilla[v_gi_s..v_gi_e], "ConVars") else {
        return Ok(vanilla.to_owned());
    };
    let cv_abs_s = v_gi_s + v_cv_s;
    let cv_abs_e = v_gi_s + v_cv_e;

    // Очищаем базу ConVars от старых вставленных блоков ShaurMeaw
    let mut cv_body = remove_marker_block(&vanilla[cv_abs_s..cv_abs_e], CONVARS_START, CONVARS_END);
    cv_body = remove_legacy_convars_block(&cv_body);

    let v_kvs = extract_top_level_kvs_map(&cv_body);
    let o_kvs = old_vanilla.and_then(|o| {
        let (gs, ge) = find_section_body(o, "GameInfo")?;
        let (cs, ce) = find_section_body(&o[gs..ge], "ConVars")?;
        Some(extract_top_level_kvs_map(&o[gs + cs..gs + ce]))
    });

    // Подготавливаем карту быстрых ползунков (если включены)
    let fov_s = format!("{:.2}", cfg.fov_aspect_ratio);
    let gamma_s = format!("{:.2}", cfg.gamma);
    let pool_s = cfg.texture_pool_mb.to_string();
    let mip_s = cfg.texture_mip_bias.to_string();
    let sh_s = if cfg.shadows_enabled { "1" } else { "0" };
    let quick_pairs: [(&str, &str); 6] = [
        ("r_aspectratio", &fov_s),
        ("r_fullscreen_gamma", &gamma_s),
        ("r_texture_pool_size", &pool_s),
        ("r_texture_stream_mip_bias", &mip_s),
        ("r_shadows", sh_s),
        ("lb_enable_shadow_casting", sh_s),
    ];

    // Определяем, какие однострочные ConVars пресета являются кастомными (отличаются от ванили)
    let mut selected_keys: HashMap<String, String> = HashMap::new();
    for (k, val, _) in extract_top_level_kvs(p_cv_body) {
        let kl = k.to_ascii_lowercase();
        let take = match &o_kvs {
            Some(old) => old.get(&kl).map(String::as_str) != Some(val.as_str()),
            None => v_kvs.get(&kl).map(String::as_str) != Some(val.as_str()),
        };
        if take {
            selected_keys.insert(kl, val);
        }
    }

    let mut override_keys: HashSet<String> = HashSet::new();
    for (ek, ev) in &extra_overrides {
        let kl = ek.to_ascii_lowercase();
        selected_keys.insert(kl.clone(), ev.clone());
        override_keys.insert(kl);
    }

    if cfg.use_quick_tweaks {
        for (qk, qv) in quick_pairs {
            let kl = qk.to_ascii_lowercase();
            selected_keys.insert(kl.clone(), qv.to_owned());
            override_keys.insert(kl);
        }
    }

    if selected_keys.is_empty() {
        return Ok(format!(
            "{}{}{}",
            &vanilla[..cv_abs_s],
            cv_body,
            &vanilla[cv_abs_e..]
        ));
    }

    // 1. Те кастомные ConVars, которые уже есть в новом ванильном ConVars — обновляем in-place
    let mut updated_in_place = HashSet::new();
    cv_body = map_top_level_lines(&cv_body, |line| {
        if let Some((k, _)) = parse_kv_pair(line) {
            let kl = k.to_ascii_lowercase();
            if let Some(new_val) = selected_keys.get(&kl) {
                if let Some(replaced) = replace_kv_line(line, &k, new_val) {
                    updated_in_place.insert(kl);
                    return replaced;
                }
            }
        }
        line.to_owned()
    });

    // 2. Все остальные ConVars собираем из тела пресета (сохраняя красивые заголовки // === и комментарии)
    let mut block_lines: Vec<String> = Vec::new();
    let mut appended_keys = HashSet::new();
    let mut pending_headers: Vec<String> = Vec::new();
    let mut tracker = KvLineTracker::default();

    for raw_line in p_cv_body.lines() {
        let (prev_depth, next_depth, started_in_quotes) = tracker.advance(raw_line);
        let only_top = prev_depth == 0 && next_depth == 0 && !started_in_quotes;

        if !only_top {
            continue;
        }

        let trimmed = raw_line.trim();
        if trimmed == CONVARS_START || trimmed == CONVARS_END {
            continue;
        }
        if trimmed.is_empty() {
            if !block_lines.is_empty() && !block_lines.last().is_some_and(String::is_empty) {
                pending_headers.push(String::new());
            }
            continue;
        }
        if trimmed.starts_with("//") {
            // Заголовки секций вроде // ================ FOV ================
            if trimmed.starts_with("// ===") {
                pending_headers.push(trimmed.to_owned());
            }
            continue;
        }

        if let Some((k, _)) = parse_kv_pair(raw_line) {
            let kl = k.to_ascii_lowercase();
            if selected_keys.contains_key(&kl)
                && !updated_in_place.contains(&kl)
                && !appended_keys.contains(&kl)
            {
                for h in pending_headers.drain(..) {
                    block_lines.push(h);
                }
                let mut line_out = raw_line.trim_end().to_owned();
                if override_keys.contains(&kl) {
                    if let Some(qv) = selected_keys.get(&kl) {
                        if let Some(replaced) = replace_kv_line(&line_out, &k, qv) {
                            line_out = replaced;
                        }
                    }
                }
                block_lines.push(line_out);
                appended_keys.insert(kl);
            } else {
                pending_headers.clear();
            }
        }
    }

    // 3. Дописываем активные FPS/HUD-твики и ползунки, которых не было ни в ванили, ни в пресете
    for (ek, ev) in &extra_overrides {
        let kl = ek.to_ascii_lowercase();
        let val_to_write = selected_keys.get(&kl).map(String::as_str).unwrap_or(ev);
        if !updated_in_place.contains(&kl) && !appended_keys.contains(&kl) {
            block_lines.push(format!("\"{ek}\" \"{val_to_write}\""));
            appended_keys.insert(kl);
        }
    }

    if cfg.use_quick_tweaks {
        for (qk, qv) in quick_pairs {
            let kl = qk.to_ascii_lowercase();
            if !updated_in_place.contains(&kl) && !appended_keys.contains(&kl) {
                block_lines.push(format!("\"{qk}\" \"{qv}\""));
                appended_keys.insert(kl);
            }
        }
    }

    if !block_lines.is_empty() {
        let trimmed_end = cv_body.trim_end_matches([' ', '\t', '\n']);
        let trailing_ws = &cv_body[trimmed_end.len()..];
        let closing_indent = trailing_ws.rsplit('\n').next().unwrap_or("    ");

        let joined = block_lines.join("\n");
        cv_body = format!(
            "{trimmed_end}\n\n{CONVARS_START}\n{}\n{CONVARS_END}\n{closing_indent}",
            joined.trim()
        );
    }

    Ok(format!(
        "{}{}{}",
        &vanilla[..cv_abs_s],
        cv_body,
        &vanilla[cv_abs_e..]
    ))
}

fn remove_legacy_convars_block(cv_body: &str) -> String {
    if let Some(legacy_pos) = cv_body.find(LEGACY_CONVARS_START) {
        let line_start = cv_body[..legacy_pos]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let tail = &cv_body[line_start..];
        if let Some(rate_rel) = find_top_level_key_line_start(tail, "rate") {
            return format!("{}{}", &cv_body[..line_start], &tail[rate_rel..]);
        }
        let trailing = cv_body.rsplit('\n').next().unwrap_or("    ");
        return format!("{}\n{trailing}", cv_body[..line_start].trim_end());
    }
    cv_body.to_owned()
}

// ============================================================ KeyValues-парсер

/// Ищет тело `{ ... }` секции `name` на глубине 0 в `text`.
pub fn find_section_body(text: &str, name: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut last_token: Option<&str> = None;

    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'"' {
            let start = i + 1;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    let char_len = text[i + 1..].chars().next().map_or(0, char::len_utf8);
                    i += 1 + char_len;
                    continue;
                }
                if bytes[i] == b'"' {
                    break;
                }
                i += 1;
            }
            let end = i.min(len);
            if depth == 0 {
                last_token = Some(&text[start..end]);
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        if b == b'{' {
            if depth == 0 && last_token.is_some_and(|t| t.eq_ignore_ascii_case(name)) {
                let body_start = i + 1;
                let mut inner_depth = 1usize;
                let mut j = body_start;
                while j < len {
                    let jb = bytes[j];
                    if jb == b'/' && j + 1 < len && bytes[j + 1] == b'/' {
                        j += 2;
                        while j < len && bytes[j] != b'\n' {
                            j += 1;
                        }
                        continue;
                    }
                    if jb == b'"' {
                        j += 1;
                        while j < len {
                            if bytes[j] == b'\\' {
                                let char_len = text[j + 1..].chars().next().map_or(0, char::len_utf8);
                                j += 1 + char_len;
                                continue;
                            }
                            if bytes[j] == b'"' {
                                break;
                            }
                            j += 1;
                        }
                        if j < len {
                            j += 1;
                        }
                        continue;
                    }
                    if jb == b'{' {
                        inner_depth += 1;
                    } else if jb == b'}' {
                        inner_depth -= 1;
                        if inner_depth == 0 {
                            return Some((body_start, j));
                        }
                    }
                    j += 1;
                }
                return None;
            }
            depth += 1;
            last_token = None;
            i += 1;
            continue;
        }

        if b == b'}' {
            depth = depth.saturating_sub(1);
            last_token = None;
            i += 1;
            continue;
        }

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        let start = i;
        while i < len
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'{' | b'}' | b'"')
        {
            if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                break;
            }
            i += 1;
        }
        if depth == 0 && start < i {
            last_token = Some(&text[start..i]);
        }
    }

    None
}

/// Находит байтовый индекс закрывающей кавычки `"` после открывающей,
/// пропуская `\"` и `\\` с сохранением границ UTF-8 символов.
pub fn find_closing_quote(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let char_len = s[i + 1..].chars().next().map_or(0, char::len_utf8);
            i += 1 + char_len;
            continue;
        }
        if bytes[i] == b'"' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[derive(Default)]
struct KvLineTracker {
    depth: usize,
    in_quotes: bool,
}

impl KvLineTracker {
    /// Обновляет глубину скобок и состояние кавычек на строке VDF, возвращая
    /// `(prev_depth, next_depth, started_in_quotes)`.
    fn advance(&mut self, line: &str) -> (usize, usize, bool) {
        let prev_depth = self.depth;
        let started_in_quotes = self.in_quotes;
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if self.in_quotes {
                if bytes[i] == b'\\' {
                    let char_len = line[i + 1..].chars().next().map_or(0, char::len_utf8);
                    i += 1 + char_len;
                    continue;
                }
                if bytes[i] == b'"' {
                    self.in_quotes = false;
                }
                i += 1;
                continue;
            }
            if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                break;
            }
            match bytes[i] {
                b'"' => self.in_quotes = true,
                b'{' => self.depth += 1,
                b'}' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
            i += 1;
        }
        (prev_depth, self.depth, started_in_quotes)
    }
}

fn extract_top_level_kvs(body: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut tracker = KvLineTracker::default();
    for line in body.lines() {
        let (prev_depth, next_depth, started_in_quotes) = tracker.advance(line);
        if prev_depth == 0 && next_depth == 0 && !started_in_quotes {
            if let Some((k, v)) = parse_kv_pair(line) {
                out.push((k, v, line.to_owned()));
            }
        }
    }
    out
}

fn extract_top_level_kvs_map(body: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (k, v, _) in extract_top_level_kvs(body) {
        map.insert(k.to_ascii_lowercase(), v);
    }
    map
}

fn parse_kv_pair(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return None;
    }

    let (key, after_key) = if let Some(inner) = trimmed.strip_prefix('"') {
        let end = find_closing_quote(inner)?;
        (&inner[..end], &inner[end + 1..])
    } else {
        let mut end = 0usize;
        let bytes = trimmed.as_bytes();
        while end < bytes.len()
            && !bytes[end].is_ascii_whitespace()
            && !matches!(bytes[end], b'{' | b'}' | b'"')
        {
            if bytes[end] == b'/' && end + 1 < bytes.len() && bytes[end + 1] == b'/' {
                break;
            }
            end += 1;
        }
        if end == 0 {
            return None;
        }
        (&trimmed[..end], &trimmed[end..])
    };

    let after_ws = after_key.trim_start();
    if after_ws.is_empty() || after_ws.starts_with("//") || after_ws.starts_with('{') {
        return None;
    }

    let val = if let Some(inner) = after_ws.strip_prefix('"') {
        let end = find_closing_quote(inner)?;
        inner[..end].trim()
    } else {
        let mut end = 0usize;
        let bytes = after_ws.as_bytes();
        while end < bytes.len()
            && !bytes[end].is_ascii_whitespace()
            && !matches!(bytes[end], b'{' | b'}' | b'"')
        {
            if bytes[end] == b'/' && end + 1 < bytes.len() && bytes[end + 1] == b'/' {
                break;
            }
            end += 1;
        }
        if end == 0 {
            return None;
        }
        after_ws[..end].trim()
    };

    Some((key.to_owned(), val.to_owned()))
}

fn map_top_level_lines<F>(body: &str, mut f: F) -> String
where
    F: FnMut(&str) -> String,
{
    let mut out = String::with_capacity(body.len());
    let mut tracker = KvLineTracker::default();
    for chunk in body.split_inclusive('\n') {
        let (line, nl) = if let Some(stripped) = chunk.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (chunk, "")
        };
        let (prev_depth, next_depth, started_in_quotes) = tracker.advance(line);
        if prev_depth == 0 && next_depth == 0 && !started_in_quotes {
            out.push_str(&f(line));
        } else {
            out.push_str(line);
        }
        out.push_str(nl);
    }
    out
}

fn replace_kv_line(line: &str, key: &str, new_val: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }
    let prefix_len = line.len() - trimmed.len();

    let after_key = if let Some(rest) = strip_quoted_key(trimmed, key) {
        rest
    } else if let Some(rest) = strip_bare_key(trimmed, key) {
        rest
    } else {
        return None;
    };

    let ws_len = after_key.len() - after_key.trim_start().len();
    if ws_len == 0 {
        return None;
    }
    let val_start = &after_key[ws_len..];
    if val_start.starts_with('{') || val_start.starts_with("//") || val_start.is_empty() {
        return None;
    }

    let key_part_len = trimmed.len() - after_key.len();
    let before_val = &line[..prefix_len + key_part_len + ws_len];

    if let Some(after_open) = val_start.strip_prefix('"') {
        let close_idx = find_closing_quote(after_open)?;
        let after_val = &after_open[close_idx + 1..];
        Some(format!("{before_val}\"{new_val}\"{after_val}"))
    } else {
        let mut end = 0usize;
        let vb = val_start.as_bytes();
        while end < vb.len() && !vb[end].is_ascii_whitespace() {
            if vb[end] == b'/' && end + 1 < vb.len() && vb[end + 1] == b'/' {
                break;
            }
            end += 1;
        }
        if end == 0 {
            return None;
        }
        let after_val = &val_start[end..];
        let need_quotes = new_val.is_empty()
            || new_val
                .bytes()
                .any(|b| b.is_ascii_whitespace() || matches!(b, b'{' | b'}' | b'"'));
        if need_quotes {
            Some(format!("{before_val}\"{new_val}\"{after_val}"))
        } else {
            Some(format!("{before_val}{new_val}{after_val}"))
        }
    }
}

fn strip_quoted_key<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let inner = s.strip_prefix('"')?;
    let end = find_closing_quote(inner)?;
    let tok = &inner[..end];
    tok.eq_ignore_ascii_case(key).then_some(&inner[end + 1..])
}

fn strip_bare_key<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    if s.len() < key.len() {
        return None;
    }
    let head = s.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let tail = &s[key.len()..];
    if tail.is_empty() || tail.starts_with(|c: char| c.is_ascii_whitespace() || c == '{') {
        Some(tail)
    } else {
        None
    }
}

fn starts_with_key_token(trimmed: &str, key: &str) -> bool {
    strip_quoted_key(trimmed, key).is_some() || strip_bare_key(trimmed, key).is_some()
}

fn parse_quoted_value(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?;
    let end = find_closing_quote(inner)?;
    Some(inner[..end].to_owned())
}

fn find_top_level_key_line_start(body: &str, key: &str) -> Option<usize> {
    let mut offset = 0usize;
    let mut tracker = KvLineTracker::default();
    for chunk in body.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let (prev_depth, _, started_in_quotes) = tracker.advance(line);
        let trimmed = line.trim_start();
        if prev_depth == 0
            && !started_in_quotes
            && !trimmed.starts_with("//")
            && starts_with_key_token(trimmed, key)
        {
            return Some(offset);
        }
        offset += chunk.len();
    }
    None
}

fn detect_line_indent<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    for line in body.lines() {
        let t = line.trim_start();
        if !t.starts_with("//") && starts_with_key_token(t, key) {
            return Some(&line[..line.len() - t.len()]);
        }
    }
    None
}

fn detect_any_kv_indent(body: &str) -> Option<&str> {
    for line in body.lines() {
        let t = line.trim_start();
        if !t.is_empty() && !t.starts_with("//") && !t.starts_with('{') && !t.starts_with('}') {
            return Some(&line[..line.len() - t.len()]);
        }
    }
    None
}

fn find_marker_block(text: &str, start_marker: &str, end_marker: &str) -> Option<(usize, usize)> {
    let s_pos = text.find(start_marker)?;
    let line_start = text[..s_pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let after_start = s_pos + start_marker.len();
    let e_rel = text[after_start..].find(end_marker)?;
    let e_pos = after_start + e_rel + end_marker.len();
    let block_end = text[e_pos..]
        .find('\n')
        .map(|off| e_pos + off + 1)
        .unwrap_or(text.len());
    Some((line_start, block_end))
}

fn remove_marker_block(text: &str, start_marker: &str, end_marker: &str) -> String {
    let mut res = text.to_owned();
    while let Some((s, e)) = find_marker_block(&res, start_marker, end_marker) {
        let trimmed_before = res[..s].trim_end_matches('\n');
        let keep_nl = if trimmed_before.is_empty() { "" } else { "\n" };
        res = format!("{trimmed_before}{keep_nl}{}", &res[e..]);
    }
    res
}

fn file_mtime_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

// ============================================================ Pure-Rust SHA-256

#[inline]
fn sha256_compress_block(h: &mut [u32; 8], block: &[u8]) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

/// Инкрементальный построитель SHA-256 (FIPS 180-4) без динамических аллокаций в процессе хеширования.
pub struct Sha256Hasher {
    h: [u32; 8],
    tail: [u8; 64],
    tail_len: usize,
    total_bytes: u64,
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            tail: [0u8; 64],
            tail_len: 0,
            total_bytes: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);

        if self.tail_len > 0 {
            let need = 64 - self.tail_len;
            let take = need.min(data.len());
            self.tail[self.tail_len..self.tail_len + take].copy_from_slice(&data[..take]);
            self.tail_len += take;
            data = &data[take..];
            if self.tail_len == 64 {
                sha256_compress_block(&mut self.h, &self.tail);
                self.tail_len = 0;
            }
        }

        let mut chunks = data.chunks_exact(64);
        for block in &mut chunks {
            sha256_compress_block(&mut self.h, block);
        }

        let rem = chunks.remainder();
        if !rem.is_empty() {
            self.tail[..rem.len()].copy_from_slice(rem);
            self.tail_len = rem.len();
        }
    }

    pub fn finalize_hex(mut self) -> String {
        let rem_len = self.tail_len;
        self.tail[rem_len] = 0x80;
        self.tail[rem_len + 1..64].fill(0);

        let bit_len = self.total_bytes.wrapping_mul(8);
        if rem_len + 1 + 8 <= 64 {
            self.tail[56..64].copy_from_slice(&bit_len.to_be_bytes());
            sha256_compress_block(&mut self.h, &self.tail);
        } else {
            sha256_compress_block(&mut self.h, &self.tail);
            let mut last = [0u8; 64];
            last[56..64].copy_from_slice(&bit_len.to_be_bytes());
            sha256_compress_block(&mut self.h, &last);
        }

        let mut hex = String::with_capacity(64);
        for word in self.h {
            let _ = write!(hex, "{word:08x}");
        }
        hex
    }
}

/// Вычисляет SHA-256 (FIPS 180-4) в виде lowercase hex-строки (64 символа) без промежуточных аллокаций.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256Hasher::new();
    hasher.update(data);
    hasher.finalize_hex()
}

/// Потоково вычисляет SHA-256 файла чанками по 64 КБ без загрузки файла в RAM.
pub fn sha256_file_streaming(path: &Path) -> std::io::Result<String> {
    let file = crate::fs_safe::open_file_with_retry(path)?;
    let mut reader = BufReader::with_capacity(crate::fs_safe::STREAM_CHUNK_SIZE, file);
    let mut chunk = [0u8; crate::fs_safe::STREAM_CHUNK_SIZE];
    let mut hasher = Sha256Hasher::new();

    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => hasher.update(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(hasher.finalize_hex())
}

// ============================================================ автозапуск синхронизации

#[cfg(windows)]
const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "ShaurMeawGameInfo";

pub fn is_autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(RUN_SUBKEY) {
            return key.get_value::<String, _>(RUN_VALUE).is_ok();
        }
    }
    false
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(RUN_SUBKEY)
            .map_err(|e| format!("Не удалось открыть автозагрузку Windows: {e}"))?;

        if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| format!("Не удалось определить путь к exe: {e}"))?;
            let cmd = format!("\"{}\" --sync-gameinfo", exe.display());
            key.set_value(RUN_VALUE, &cmd)
                .map_err(|e| format!("Не удалось добавить в автозагрузку: {e}"))?;
        } else {
            let _ = key.delete_value(RUN_VALUE);
        }
    }
    Ok(())
}

// ============================================================ юнит-тесты

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_NEW_VANILLA: &str = r#""GameInfo"
{
    game        "citadel"
    title       "Citadel"
    tonemapping 1
    PGIVersion  "NEW_VALVE_PGI_HASH_2026"

    FileSystem
    {
        SearchPaths
        {
            Game_UILanguage     citadel_*LANGUAGE*
            Game_LowViolence    citadel_lv
            Game                citadel/new_valve_dlc
            Mod                 citadel
            Write               citadel
            Game                citadel
            Mod                 core
            Write               core
            Game                core
        }
    }

    RenderSystem
    {
        "NewValveRenderFlag" "1"
    }

    SceneSystem
    {
        GpuLightBinner 1
        FogCachedShadowAtlasWidth 2048
        FogCachedShadowAtlasHeight 2048
        FogCachedShadowTileSize 256
        CSMCascadeResolution 1024
        SunLightManagerCount 4
        SunLightManagerCountTools 4
        DefaultShadowTextureWidth 1024
        DefaultShadowTextureHeight 1024
        DynamicShadowResolution 1024
        TransformTextureRowCount 2048
        NonTexturedGradientFog 1
        CubemapFog 1
        VolumetricFog 1
        "NewValveSceneSetting" "42"
        WellKnownLightCookies
        {
            "blank" "materials/effects/lightcookies/blank.vtex"
        }
    }

    ConVars
    {
        "rate"
        {
            "min"       "98304"
            "default"   "786432"
        }
        "fps_max" "400"
        "new_valve_convar" "123"
    }
}
"#;

    fn count_braces(s: &str) -> (usize, usize) {
        let mut open = 0usize;
        let mut close = 0usize;
        for line in s.lines() {
            let mut in_str = false;
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if !in_str && bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    break;
                }
                if bytes[i] == b'"' {
                    in_str = !in_str;
                } else if !in_str && bytes[i] == b'{' {
                    open += 1;
                } else if !in_str && bytes[i] == b'}' {
                    close += 1;
                }
                i += 1;
            }
        }
        (open, close)
    }

    #[test]
    fn test_sha256_nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Двухблочный NIST-вектор (длина 56 байт — граница padding)
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn test_smart_merge_shaurmeaw_preset_onto_new_valve_patch() {
        let shaurmeaw_preset = crate::embedded::shaurmeaw_gi();
        let cfg = GameInfoConfig::default();
        let out = smart_merge(
            SAMPLE_NEW_VANILLA,
            shaurmeaw_preset,
            None,
            &cfg,
            "ShaurMeaw (Мой конфиг)",
            Some("99887766"),
        )
        .unwrap();

        assert!(out.starts_with("// [ShaurMeaw SmartMerge] preset=\"ShaurMeaw (Мой конфиг)\" build=99887766"));
        // Сохранены свежие ключи Valve
        assert!(out.contains("PGIVersion  \"NEW_VALVE_PGI_HASH_2026\""));
        assert!(out.contains("citadel/new_valve_dlc"));
        assert!(out.contains("\"NewValveRenderFlag\" \"1\""));
        assert!(out.contains("TransformTextureRowCount 2048"));
        assert!(out.contains("\"NewValveSceneSetting\" \"42\""));
        assert!(out.contains("\"new_valve_convar\" \"123\""));

        // Перенесены модовые пути и обнулён SceneSystem
        assert!(out.contains("citadel/dmm_localization_default"));
        assert!(out.contains("citadel/addons"));
        assert!(out.contains("tonemapping 0"));
        assert!(out.contains("FogCachedShadowAtlasWidth 0"));
        assert!(out.contains("DynamicShadowResolution 0"));
        assert!(out.contains("VolumetricFog 0"));
        assert!(out.contains("Tonemapping 0"));

        // Перенесены ConVars ShaurMeaw
        assert!(out.contains("\"r_aspectratio\" \"2.3\""));
        assert!(out.contains(CONVARS_START));

        let (o, c) = count_braces(&out);
        assert_eq!(o, c, "Нарушен баланс фигурных скобок: open={o}, close={c}");
    }

    #[test]
    fn test_smart_merge_three_way_friend_preset() {
        let old_vanilla = r#""GameInfo"
{
    tonemapping 1
    FileSystem
    {
        SearchPaths
        {
            Mod citadel
            Game citadel
        }
    }
    SceneSystem
    {
        TransformTextureRowCount 1024
        DynamicShadowResolution 512
    }
    ConVars
    {
        "fps_max" "400"
    }
}"#;
        let friend_preset = r#""GameInfo"
{
    tonemapping 0
    FileSystem
    {
        SearchPaths
        {
            Game citadel/friend_mods
            Mod citadel
            Game citadel
        }
    }
    SceneSystem
    {
        TransformTextureRowCount 1024
        DynamicShadowResolution 0
    }
    ConVars
    {
        "fps_max" "144"
        "friend_custom_cvar" "42"
    }
}"#;

        let cfg = GameInfoConfig::default();
        let out = smart_merge(
            SAMPLE_NEW_VANILLA,
            friend_preset,
            Some(old_vanilla),
            &cfg,
            "Конфиг друга",
            Some("555"),
        )
        .unwrap();

        // Апдейт Valve TransformTextureRowCount (1024 -> 2048) сохранился!
        assert!(out.contains("TransformTextureRowCount 2048"));
        // Новый путь Valve сохранился + добавлен путь друга
        assert!(out.contains("citadel/new_valve_dlc"));
        assert!(out.contains("citadel/friend_mods"));
        // Отличия друга перенеслись
        assert!(out.contains("tonemapping 0"));
        assert!(out.contains("DynamicShadowResolution 0"));
        assert!(out.contains("\"fps_max\" \"144\""));
        assert!(out.contains("\"friend_custom_cvar\" \"42\""));
        assert!(out.contains("\"new_valve_convar\" \"123\""));

        let (o, c) = count_braces(&out);
        assert_eq!(o, c);
    }

    #[test]
    fn test_smart_merge_mods_only_and_clean_vanilla() {
        let cfg = GameInfoConfig::default();
        let mods_preset = r#""GameInfo"
{
    FileSystem
    {
        SearchPaths
        {
            Game citadel/addons
            Mod citadel
        }
    }
}"#;
        let mods_out =
            smart_merge(SAMPLE_NEW_VANILLA, mods_preset, None, &cfg, "Для модов", Some("1")).unwrap();
        assert!(mods_out.contains("citadel/addons"));
        assert!(mods_out.contains("DynamicShadowResolution 1024"));

        let clean_out = smart_merge(
            SAMPLE_NEW_VANILLA,
            "// [ShaurMeaw Clean Vanilla Preset]\n",
            None,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_VALVE,
            Some("1"),
        )
        .unwrap();
        assert!(!clean_out.contains("citadel/addons"));
        assert!(!clean_out.contains(SMART_MARKER_PREFIX));
        assert!(clean_out.contains("PGIVersion  \"NEW_VALVE_PGI_HASH_2026\""));
    }

    #[test]
    fn test_switch_from_custom_shaurmeaw_to_official_valve_without_prior_backup() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_switch_valve_test_{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let presets_dir = root.join("presets");
        let backups_dir = root.join("backups");
        let state_path = root.join("gameinfo_state.json");
        let steam_dir = root.join("Steam");
        let game_dir = steam_dir.join("steamapps").join("common").join("Deadlock");
        let citadel_dir = game_dir.join("game").join("citadel");
        fs::create_dir_all(&citadel_dir).unwrap();
        fs::create_dir_all(steam_dir.join("steamapps")).unwrap();

        fs::write(
            steam_dir.join("steamapps").join("appmanifest_1422450.acf"),
            "\"AppState\"\n{\n    \"buildid\" \"26770241\"\n}\n",
        )
        .unwrap();

        // Ситуация как у пользователя: в игре изначально стоит кастомный gameinfo.gi, а бэкапа ванили ещё нет!
        fs::write(
            citadel_dir.join("gameinfo.gi"),
            crate::embedded::shaurmeaw_gi(),
        )
        .unwrap();

        gameinfo_presets::ensure_default_presets_in(&presets_dir).unwrap();
        let cfg = GameInfoConfig::default();

        let report = apply_preset_in_dirs(
            &presets_dir,
            &backups_dir,
            &state_path,
            &steam_dir,
            &game_dir,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_VALVE,
        )
        .unwrap();
        assert!(report.message.contains("официальный gameinfo.gi"));

        let gi_after = fs::read_to_string(citadel_dir.join("gameinfo.gi")).unwrap();
        assert!(!gi_after.contains(SMART_MARKER_PREFIX));
        assert!(!gi_after.contains(LEGACY_AUTHOR_HEADER));
        assert!(!gi_after.contains("citadel/addons"));
        assert!(!gi_after.contains("r_aspectratio"));
        assert!(gi_after.contains("PGIVersion \"48C04ECA"));

        // Статус должен показать UpToDate с активным пресетом Valve
        let state = GameInfoState::load_from(&state_path);
        match inspect_status(&steam_dir, &game_dir, &state) {
            SyncStatus::UpToDate { active_preset, .. } => {
                assert_eq!(active_preset, gameinfo_presets::DEFAULT_PRESET_VALVE);
            }
            other => panic!("Ожидался UpToDate, получено: {other:?}"),
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_switch_from_custom_shaurmeaw_to_mods_only_without_prior_backup() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_switch_mods_test_{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let presets_dir = root.join("presets");
        let backups_dir = root.join("backups");
        let state_path = root.join("gameinfo_state.json");
        let steam_dir = root.join("Steam");
        let game_dir = steam_dir.join("steamapps").join("common").join("Deadlock");
        let citadel_dir = game_dir.join("game").join("citadel");
        fs::create_dir_all(&citadel_dir).unwrap();
        fs::create_dir_all(steam_dir.join("steamapps")).unwrap();

        fs::write(
            citadel_dir.join("gameinfo.gi"),
            crate::embedded::shaurmeaw_gi(),
        )
        .unwrap();

        gameinfo_presets::ensure_default_presets_in(&presets_dir).unwrap();
        let cfg = GameInfoConfig::default();

        apply_preset_in_dirs(
            &presets_dir,
            &backups_dir,
            &state_path,
            &steam_dir,
            &game_dir,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_MODS,
        )
        .unwrap();

        let gi_after = fs::read_to_string(citadel_dir.join("gameinfo.gi")).unwrap();
        assert!(gi_after.contains("citadel/addons"));
        assert!(!gi_after.contains("r_aspectratio"));
        assert!(!gi_after.contains("CORE QUALITY"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_patch_detection_workflow_with_backups() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_sync_test_{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let presets_dir = root.join("presets");
        let backups_dir = root.join("backups");
        let state_path = root.join("gameinfo_state.json");
        let steam_dir = root.join("Steam");
        let game_dir = steam_dir.join("steamapps").join("common").join("Deadlock");
        let citadel_dir = game_dir.join("game").join("citadel");
        fs::create_dir_all(&citadel_dir).unwrap();
        fs::create_dir_all(steam_dir.join("steamapps")).unwrap();

        fs::write(
            steam_dir.join("steamapps").join("appmanifest_1422450.acf"),
            "\"AppState\"\n{\n    \"buildid\" \"10001\"\n}\n",
        )
        .unwrap();
        fs::write(citadel_dir.join("gameinfo.gi"), SAMPLE_NEW_VANILLA).unwrap();

        gameinfo_presets::ensure_default_presets_in(&presets_dir).unwrap();
        let mut cfg = GameInfoConfig::default();
        cfg.active_preset = gameinfo_presets::DEFAULT_PRESET_SHAURMEAW.to_owned();

        // 1. Первичное применение пресета
        let first_report = apply_preset_in_dirs(
            &presets_dir,
            &backups_dir,
            &state_path,
            &steam_dir,
            &game_dir,
            &cfg,
            &cfg.active_preset,
        )
        .unwrap();
        assert!(!first_report.is_patch_adaptation);
        assert!(vanilla_backup_path(&game_dir).exists());

        // Проверим, что повторная проверка без изменений ничего не делает
        assert!(check_and_adapt_in_dirs(
            &presets_dir,
            &backups_dir,
            &state_path,
            &steam_dir,
            &game_dir,
            &cfg,
        )
        .unwrap()
        .is_none());

        // 2. Имитируем патч Valve: Steam перезаписывает gameinfo.gi и обновляет buildid
        let patched_by_valve = SAMPLE_NEW_VANILLA.replace(
            "NEW_VALVE_PGI_HASH_2026",
            "VALVE_PATCH_2026_NOV_HASH",
        );
        fs::write(citadel_dir.join("gameinfo.gi"), patched_by_valve).unwrap();
        fs::write(
            steam_dir.join("steamapps").join("appmanifest_1422450.acf"),
            "\"AppState\"\n{\n    \"buildid\" \"10002\"\n}\n",
        )
        .unwrap();

        // 3. Проверяем автодетект и Smart Merge
        let adapt_report = check_and_adapt_in_dirs(
            &presets_dir,
            &backups_dir,
            &state_path,
            &steam_dir,
            &game_dir,
            &cfg,
        )
        .unwrap()
        .expect("Патч должен быть обнаружен");

        assert!(adapt_report.is_patch_adaptation);
        assert_eq!(
            adapt_report.message,
            "Обнаружен патч Deadlock. Ваш профиль [ShaurMeaw (Мой конфиг)] успешно адаптирован и применен"
        );

        let result_gi = fs::read_to_string(citadel_dir.join("gameinfo.gi")).unwrap();
        assert!(result_gi.contains("VALVE_PATCH_2026_NOV_HASH"));
        assert!(result_gi.contains("citadel/addons"));
        assert!(result_gi.contains("\"r_aspectratio\" \"2.3\""));
        assert!(gameinfo_presets::list_backups_in(&backups_dir).len() >= 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_smart_merge_performance_fps_preset_onto_new_valve_patch() {
        let perf_preset = crate::embedded::performance_fps_gi();
        let patched_valve = crate::embedded::valve_vanilla_gi()
            .replace(
                "48C04ECAFC6448F8D3F41F0AA3A15D065501751CF45FBE32DF6CB951BC4F7F6E",
                "NEW_VALVE_PATCH_2026_OPT",
            )
            .replace(
                "ComputeShaderSkinning 1",
                "ComputeShaderSkinning 1\n\t\tNewValvePatchKey 777",
            );

        let cfg = GameInfoConfig::default();
        let out = smart_merge(
            &patched_valve,
            perf_preset,
            None,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_PERFORMANCE_FPS,
            Some("33445566"),
        )
        .unwrap();

        assert!(out.starts_with(
            "// [ShaurMeaw SmartMerge] preset=\"Перформанс FPS (OptimizationLock 4.0)\" build=33445566"
        ));
        // Сохранены свежие ключи Valve
        assert!(out.contains("NEW_VALVE_PATCH_2026_OPT"));
        assert!(out.contains("NewValvePatchKey 777"));

        // Перенесены SearchPaths (custom + addons)
        assert!(out.contains("citadel/custom"));
        assert!(out.contains("citadel/addons"));

        // SceneSystem FPS-оптимизации
        assert!(out.contains("HDRFrameBuffer"));
        assert!(out.contains("ParticleBufferSize"));

        // RenderSystem / WorldRenderer / Particles FPS-оптимизации
        assert!(out.contains("ExecuteQueuedCommandBuffers"));
        assert!(out.contains("MaxPreloadTextureResolution"));
        assert!(out.contains("GrassCastsShadows\t\t\t\t0"));
        assert!(out.contains("ParticlesFoggedByDefault"));

        // ConVars FPS-оптимизации
        assert!(out.contains("sc_clutter_enable"));
        assert!(out.contains("r_citadel_shadow_quality"));
        assert!(out.contains("r_farz"));

        let (o, c) = count_braces(&out);
        assert_eq!(o, c, "Нарушен баланс скобок: open={o}, close={c}");
    }

    #[test]
    fn test_smart_merge_ultra_fps_preset_and_tweak_overrides() {
        let ultra_fps_preset = crate::embedded::ultra_fps_gi();
        let patched_valve = crate::embedded::valve_vanilla_gi().replace(
            "48C04ECAFC6448F8D3F41F0AA3A15D065501751CF45FBE32DF6CB951BC4F7F6E",
            "VALVE_ULTRA_FPS_TEST_HASH",
        );

        let mut cfg = GameInfoConfig::default();
        cfg.fov_preset = crate::settings::FovPreset::Fov90;
        cfg.hide_hp_names = true;
        cfg.bright_outlines = true;
        cfg.no_flash_lights = true;
        cfg.no_camera_shake = true;
        cfg.cpu_threads_boost = true;

        let out = smart_merge(
            &patched_valve,
            ultra_fps_preset,
            None,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_ULTRA_FPS,
            Some("44556677"),
        )
        .unwrap();

        assert!(out.starts_with(
            "// [ShaurMeaw SmartMerge] preset=\"Ультра FPS (Без теней и тумана)\" build=44556677"
        ));
        assert!(out.contains("VALVE_ULTRA_FPS_TEST_HASH"));
        assert!(out.contains("citadel/addons"));
        assert!(out.contains("CSMCascadeResolution 0"));
        assert!(out.contains("VolumetricFog 0"));
        assert!(out.contains("r_aspectratio"));
        assert!(out.contains("\"2.15\""));
        assert!(out.contains("cl_glow_brightness"));
        assert!(out.contains("\"1.5\""));
        assert!(out.contains("citadel_unit_status_hide_names"));
        assert!(out.contains("r_muzzleflashbrightness"));
        assert!(out.contains("r_threaded_particles"));

        let (o, c) = count_braces(&out);
        assert_eq!(o, c, "Нарушен баланс скобок: open={o}, close={c}");
    }

    #[test]
    fn test_smart_merge_preserves_november_2026_build_6864_sections() {
        let mut nov_vanilla = crate::embedded::valve_vanilla_gi()
            .replace(
                "48C04ECAFC6448F8D3F41F0AA3A15D065501751CF45FBE32DF6CB951BC4F7F6E",
                "NOV_2026_BUILD_6864_HASH",
            )
            .replace(
                "Game\t\t\t\tcitadel",
                "Game\t\t\t\tcitadel_patch\r\n\t\t\tGame\t\t\t\tcitadel",
            )
            .replace(
                "\"net_gather_child_fields_only\" \"1\"",
                "\"net_gather_child_fields_only\" \"1\"\r\n\t\t\"r_nov2026_new_cvar\" \"1\"",
            );
        if let Some(pos) = nov_vanilla.rfind('}') {
            nov_vanilla.insert_str(
                pos,
                "\n    ShaderSystem\n    {\n        VulkanPipelineCache 1\n    }\n    NetworkSystem\n    {\n        NetChannelMaxFragments 64\n    }\n",
            );
        }
        assert!(nov_vanilla.contains("citadel_patch"));
        assert!(nov_vanilla.contains("r_nov2026_new_cvar"));

        let cfg = GameInfoConfig::default();
        let out = smart_merge(
            &nov_vanilla,
            crate::embedded::shaurmeaw_gi(),
            None,
            &cfg,
            gameinfo_presets::DEFAULT_PRESET_SHAURMEAW,
            Some("26764805"),
        )
        .unwrap();

        assert!(out.contains("NOV_2026_BUILD_6864_HASH"));
        assert!(out.contains("ShaderSystem"));
        assert!(out.contains("VulkanPipelineCache 1"));
        assert!(out.contains("NetworkSystem"));
        assert!(out.contains("NetChannelMaxFragments 64"));
        assert!(out.contains("citadel_patch"));
        assert!(out.contains("r_nov2026_new_cvar"));
        assert!(out.contains("citadel/addons"));

        let (o, c) = count_braces(&out);
        assert_eq!(o, c, "Нарушен баланс скобок: open={o}, close={c}");
    }

    #[test]
    fn test_kv_parser_handles_escaped_quotes_multiline_and_cyrillic() {
        let vdf = concat!(
            "\"GameInfo\"\n",
            "{\n",
            "    title \"Deadlock \\\"Я\\\" {тест}\"\n",
            "    \"multiline\" \"строка 1\n",
            "    { внутри строки } // не секция\n",
            "    строка 3\"\n",
            "    tonemapping 1 // комментарий // в конце\n",
            "    FileSystem\n",
            "    {\n",
            "        SearchPaths\n",
            "        {\n",
            "            Game citadel\n",
            "        }\n",
            "    }\n",
            "    ConVars\n",
            "    {\n",
            "        fps_max 400\n",
            "    }\n",
            "}\n",
        );

        let (gs, ge) = find_section_body(vdf, "GameInfo").expect("секция GameInfo должна находиться");
        let kvs = extract_top_level_kvs_map(&vdf[gs..ge]);
        assert_eq!(kvs.get("tonemapping").map(String::as_str), Some("1"));
        assert!(kvs.get("title").is_some_and(|t| t.contains("\\\"Я\\\"")));

        let line_bare = "    fps_max 400 // max";
        let replaced = replace_kv_line(line_bare, "fps_max", "240 0").unwrap();
        assert_eq!(replaced, "    fps_max \"240 0\" // max");
    }

    #[test]
    fn test_sha256_streaming_matches_in_memory_across_chunk_boundaries() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_sha256_stream_{}_{}",
            std::process::id(),
            UNIX_EPOCH.elapsed().map(|d| d.as_nanos()).unwrap_or(1)
        ));
        fs::create_dir_all(&root).unwrap();

        let payload: Vec<u8> = (0..(150 * 1024 + 317))
            .map(|i| ((i * 73 + 19) % 256) as u8)
            .collect();
        let path = root.join("large_mod.vpk");
        fs::write(&path, &payload).unwrap();

        let expected = sha256_hex(&payload);
        let streamed = sha256_file_streaming(&path).unwrap();
        assert_eq!(streamed, expected);

        let empty_path = root.join("empty.vpk");
        fs::write(&empty_path, b"").unwrap();
        assert_eq!(sha256_file_streaming(&empty_path).unwrap(), sha256_hex(b""));

        let _ = fs::remove_dir_all(&root);
    }
}
