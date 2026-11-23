//! In-memory кэш состояния лаунчера, нативный файловый watcher Windows (`FindFirstChangeNotificationW`)
//! и асинхронный двухканальный воркер I/O (`crossbeam-channel`) для 0ms UI Latency.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::autoexec_presets::{self, AutoexecPresetEntry, MODE_BUILDER};
use crate::config_gen;
use crate::game_booster::{
    self, wait_event_or_dirs, DirChangeHandle, WaitOutcome, WinEvent, WindowsTweaksStatus,
    NOTIFY_FILE_AND_WRITE, WAIT_INFINITE,
};
use crate::gameinfo_presets::{self, BackupEntry, PresetEntry};
use crate::gameinfo_sync::{self, GameInfoState, SyncStatus};
use crate::launcher;
use crate::settings::Settings;
use crate::steam;
use crate::tools::{self, AddonModEntry, CacheStats};
use crate::video_presets::{self, VideoPresetEntry};

const NOTIFY_DEBOUNCE_MS: u32 = 120;
const NOTIFY_MAX_BATCH_MS: u64 = 300;
const SELF_WRITE_GRACE_MS: u64 = 650;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    GameInfo,
    Autoexec,
    Video,
}

/// Полный снимок файлового/системного состояния лаунчера в памяти (без чтения диска в UI).
#[derive(Clone, Debug)]
pub struct AppCache {
    pub gi_presets: Vec<PresetEntry>,
    pub gi_backups: Vec<BackupEntry>,
    pub gi_texts: HashMap<String, String>,

    pub ae_presets: Vec<AutoexecPresetEntry>,
    pub ae_texts: HashMap<String, String>,

    pub vid_presets: Vec<VideoPresetEntry>,
    pub vid_texts: HashMap<String, String>,

    pub profiles: Vec<String>,

    pub addon_mods: Vec<AddonModEntry>,
    pub addon_hashes: HashMap<PathBuf, String>,
    pub dmm_conflicts: usize,

    pub vid_active_uid: Option<String>,
    pub vid_accounts_count: usize,
    pub vid_targets: Vec<PathBuf>,

    pub gi_status: SyncStatus,
    pub has_vanilla_bak: bool,
    pub has_autoexec_bak: bool,

    pub paths_steam_ok: bool,
    pub paths_game_ok: bool,
    pub is_portable: bool,
    pub cached_app_dir: PathBuf,
    pub cached_settings_file: PathBuf,

    pub autostart_sync: bool,
    pub has_shortcut: bool,
    pub console_log_bytes: u64,

    pub win_tweaks: Option<WindowsTweaksStatus>,
    pub cache_stats: Option<CacheStats>,

    pub hydrated: bool,
    gi_mtimes: HashMap<String, (u64, Option<SystemTime>)>,
    ae_mtimes: HashMap<String, (u64, Option<SystemTime>)>,
    vid_mtimes: HashMap<String, (u64, Option<SystemTime>)>,
    last_self_writes: HashMap<PathBuf, Instant>,
}

impl Default for AppCache {
    fn default() -> Self {
        Self {
            gi_presets: Vec::new(),
            gi_backups: Vec::new(),
            gi_texts: HashMap::new(),
            ae_presets: Vec::new(),
            ae_texts: HashMap::new(),
            vid_presets: Vec::new(),
            vid_texts: HashMap::new(),
            profiles: Vec::new(),
            addon_mods: Vec::new(),
            addon_hashes: HashMap::new(),
            dmm_conflicts: 0,
            vid_active_uid: None,
            vid_accounts_count: 0,
            vid_targets: Vec::new(),
            gi_status: SyncStatus::GameNotFound,
            has_vanilla_bak: false,
            has_autoexec_bak: false,
            paths_steam_ok: false,
            paths_game_ok: false,
            is_portable: false,
            cached_app_dir: Settings::app_dir(),
            cached_settings_file: Settings::file_path(),
            autostart_sync: false,
            has_shortcut: false,
            console_log_bytes: 0,
            win_tweaks: None,
            cache_stats: None,
            hydrated: false,
            gi_mtimes: HashMap::new(),
            ae_mtimes: HashMap::new(),
            vid_mtimes: HashMap::new(),
            last_self_writes: HashMap::new(),
        }
    }
}

/// Потокобезопасная обёртка над `AppCache` с O(1) проверкой ревизии в UI-цикле.
#[derive(Clone)]
pub struct SharedAppCache {
    inner: Arc<RwLock<AppCache>>,
    rev: Arc<AtomicU64>,
}

impl SharedAppCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(AppCache::default())),
            rev: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn revision(&self) -> u64 {
        self.rev.load(Ordering::Acquire)
    }

    pub fn bump_revision(&self) {
        self.rev.fetch_add(1, Ordering::Release);
    }

    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, AppCache> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, AppCache> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn mark_self_write(&self, path: PathBuf) {
        let now = Instant::now();
        let mut w = self.write();
        w.last_self_writes
            .retain(|_, t| now.duration_since(*t) < Duration::from_secs(3));
        w.last_self_writes.insert(path, now);
    }

    fn is_recent_self_write(&self, path: &Path) -> bool {
        let now = Instant::now();
        let r = self.read();
        r.last_self_writes
            .get(path)
            .is_some_and(|t| now.duration_since(*t) < Duration::from_millis(SELF_WRITE_GRACE_MS))
    }

    /// Перезагружает домен `gameinfo.gi` (пресеты, бэкапы, статус печати) без удержания блокировки во время I/O.
    pub fn reload_gameinfo_domain(
        &self,
        steam_path: &Path,
        game_path: &Path,
        invalidate_names: Option<&[String]>,
    ) {
        self.reload_gameinfo_domain_in(
            &Settings::gameinfo_presets_dir(),
            &Settings::gameinfo_backups_dir(),
            steam_path,
            game_path,
            invalidate_names,
            false,
        );
    }

    pub fn reload_gameinfo_domain_in(
        &self,
        presets_dir: &Path,
        backups_dir: &Path,
        steam_path: &Path,
        game_path: &Path,
        invalidate_names: Option<&[String]>,
        from_watcher: bool,
    ) {
        let presets = gameinfo_presets::list_presets_in(presets_dir);
        let backups = gameinfo_presets::list_backups_in(backups_dir);
        let state = GameInfoState::load();
        let gi_status = gameinfo_sync::inspect_status(steam_path, game_path, &state);
        let has_vanilla_bak = gameinfo_sync::vanilla_backup_path(game_path).exists();

        let new_mtimes: Vec<(String, (u64, Option<SystemTime>), bool)> = presets
            .iter()
            .map(|p| {
                let mtime = std::fs::metadata(&p.path).and_then(|m| m.modified()).ok();
                let self_write = from_watcher && self.is_recent_self_write(&p.path);
                (p.name.clone(), (p.size_bytes, mtime), self_write)
            })
            .collect();

        let existing_names: HashSet<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        let mut w = self.write();
        w.gi_texts.retain(|k, _| existing_names.contains(k.as_str()));
        if from_watcher {
            for (name, stamp, self_write) in &new_mtimes {
                if !self_write && w.gi_mtimes.get(name) != Some(stamp) {
                    w.gi_texts.remove(name);
                }
            }
        }
        w.gi_mtimes.clear();
        for (name, stamp, _) in new_mtimes {
            w.gi_mtimes.insert(name, stamp);
        }
        w.gi_presets = presets;
        w.gi_backups = backups;
        w.gi_status = gi_status;
        w.has_vanilla_bak = has_vanilla_bak;
        if let Some(names) = invalidate_names {
            if names.is_empty() {
                w.gi_texts.clear();
            } else {
                for n in names {
                    w.gi_texts.remove(n);
                }
            }
        }
        drop(w);
        self.bump_revision();
    }

    pub fn reload_autoexec_domain(&self, game_path: &Path, invalidate_names: Option<&[String]>) {
        self.reload_autoexec_domain_in(
            &Settings::autoexec_presets_dir(),
            game_path,
            invalidate_names,
            false,
        );
    }

    pub fn reload_autoexec_domain_in(
        &self,
        presets_dir: &Path,
        game_path: &Path,
        invalidate_names: Option<&[String]>,
        from_watcher: bool,
    ) {
        let presets = autoexec_presets::list_presets_in(presets_dir);
        let has_bak = tools::has_autoexec_backup(game_path);

        let new_mtimes: Vec<(String, (u64, Option<SystemTime>), bool)> = presets
            .iter()
            .map(|p| {
                let mtime = std::fs::metadata(&p.path).and_then(|m| m.modified()).ok();
                let self_write = from_watcher && self.is_recent_self_write(&p.path);
                (p.name.clone(), (p.size_bytes, mtime), self_write)
            })
            .collect();

        let existing_names: HashSet<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        let mut w = self.write();
        w.ae_texts.retain(|k, _| existing_names.contains(k.as_str()));
        if from_watcher {
            for (name, stamp, self_write) in &new_mtimes {
                if !self_write && w.ae_mtimes.get(name) != Some(stamp) {
                    w.ae_texts.remove(name);
                }
            }
        }
        w.ae_mtimes.clear();
        for (name, stamp, _) in new_mtimes {
            w.ae_mtimes.insert(name, stamp);
        }
        w.ae_presets = presets;
        w.has_autoexec_bak = has_bak;
        if let Some(names) = invalidate_names {
            if names.is_empty() {
                w.ae_texts.clear();
            } else {
                for n in names {
                    w.ae_texts.remove(n);
                }
            }
        }
        drop(w);
        self.bump_revision();
    }

    pub fn reload_video_domain(
        &self,
        steam_path: &Path,
        game_path: &Path,
        apply_all: bool,
        invalidate_names: Option<&[String]>,
    ) {
        self.reload_video_domain_in(
            &Settings::video_presets_dir(),
            steam_path,
            game_path,
            apply_all,
            invalidate_names,
            false,
        );
    }

    pub fn reload_video_domain_in(
        &self,
        presets_dir: &Path,
        steam_path: &Path,
        game_path: &Path,
        apply_all: bool,
        invalidate_names: Option<&[String]>,
        from_watcher: bool,
    ) {
        let presets = video_presets::list_presets_in(presets_dir);
        let active_uid = video_presets::active_steam_user_id();
        let accounts_count = video_presets::steam_userdata_video_paths(steam_path).len();
        let targets = video_presets::resolve_target_paths(
            steam_path,
            game_path,
            active_uid.as_deref(),
            apply_all,
        );

        let new_mtimes: Vec<(String, (u64, Option<SystemTime>), bool)> = presets
            .iter()
            .map(|p| {
                let mtime = std::fs::metadata(&p.path).and_then(|m| m.modified()).ok();
                let self_write = from_watcher && self.is_recent_self_write(&p.path);
                (p.name.clone(), (p.size_bytes, mtime), self_write)
            })
            .collect();

        let existing_names: HashSet<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        let mut w = self.write();
        w.vid_texts.retain(|k, _| existing_names.contains(k.as_str()));
        if from_watcher {
            for (name, stamp, self_write) in &new_mtimes {
                if !self_write && w.vid_mtimes.get(name) != Some(stamp) {
                    w.vid_texts.remove(name);
                }
            }
        }
        w.vid_mtimes.clear();
        for (name, stamp, _) in new_mtimes {
            w.vid_mtimes.insert(name, stamp);
        }
        w.vid_presets = presets;
        w.vid_active_uid = active_uid;
        w.vid_accounts_count = accounts_count;
        w.vid_targets = targets;
        if let Some(names) = invalidate_names {
            if names.is_empty() {
                w.vid_texts.clear();
            } else {
                for n in names {
                    w.vid_texts.remove(n);
                }
            }
        }
        drop(w);
        self.bump_revision();
    }

    pub fn reload_profiles_domain(&self) {
        let profiles = tools::list_profiles();
        let mut w = self.write();
        w.profiles = profiles;
        drop(w);
        self.bump_revision();
    }

    pub fn reload_addons_domain(&self, game_path: &Path) {
        let mods = tools::list_addon_mods(game_path);
        let conflicts = tools::read_dmm_conflicts_count(game_path);
        let paths: HashSet<PathBuf> = mods.iter().map(|m| m.path.clone()).collect();
        let mut w = self.write();
        w.addon_mods = mods;
        w.dmm_conflicts = conflicts;
        w.addon_hashes.retain(|p, _| paths.contains(p));
        drop(w);
        self.bump_revision();
    }

    pub fn reload_paths_domain(&self, steam_path: &Path, game_path: &Path) {
        let steam_ok = steam_path.join("steam.exe").exists();
        let game_ok = steam::is_valid_game_dir(game_path);
        let is_portable = Settings::is_portable_mode();
        let app_dir = Settings::app_dir();
        let settings_file = Settings::file_path();
        let mut w = self.write();
        w.paths_steam_ok = steam_ok;
        w.paths_game_ok = game_ok;
        w.is_portable = is_portable;
        w.cached_app_dir = app_dir;
        w.cached_settings_file = settings_file;
        drop(w);
        self.bump_revision();
    }
}

// ============================================================ Нативный Win32 Directory Watcher

#[derive(Clone, Debug)]
struct WatchRoots {
    gi_presets: PathBuf,
    gi_backups: PathBuf,
    ae_presets: PathBuf,
    vid_presets: PathBuf,
    profiles: PathBuf,
    addons: Option<PathBuf>,
    steam_path: PathBuf,
    game_path: PathBuf,
    video_apply_all: bool,
}

impl WatchRoots {
    fn from_settings(s: &Settings) -> Self {
        let game_path = PathBuf::from(&s.game_path);
        let addons_dir = tools::addons_dir(&game_path);
        Self {
            gi_presets: Settings::gameinfo_presets_dir(),
            gi_backups: Settings::gameinfo_backups_dir(),
            ae_presets: Settings::autoexec_presets_dir(),
            vid_presets: Settings::video_presets_dir(),
            profiles: Settings::profiles_dir(),
            addons: addons_dir.is_dir().then_some(addons_dir),
            steam_path: PathBuf::from(&s.steam_path),
            game_path,
            video_apply_all: s.video_apply_all_accounts,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchDomain {
    GiPresets,
    GiBackups,
    AePresets,
    VidPresets,
    Profiles,
    Addons,
}

enum WatcherCtrl {
    UpdateRoots(WatchRoots),
    Shutdown,
}

/// Асинхронный наблюдатель папок пресетов, профилей и модов на нативном Win32 `DirChangeHandle`.
/// При простое поток спит в ядре (`WAIT_INFINITE`, 0% CPU); при изменении файлов с дебаунсом обновляет `SharedAppCache`.
pub struct PresetWatcher {
    ctrl_tx: Sender<WatcherCtrl>,
    wake: Option<Arc<WinEvent>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl PresetWatcher {
    pub fn spawn(
        cache: SharedAppCache,
        initial: &Settings,
        ctx: egui::Context,
        window_minimized: Arc<AtomicBool>,
    ) -> Self {
        Self::spawn_with_roots(
            cache,
            WatchRoots::from_settings(initial),
            Some(ctx),
            window_minimized,
        )
    }

    fn spawn_with_roots(
        cache: SharedAppCache,
        initial_roots: WatchRoots,
        ctx: Option<egui::Context>,
        window_minimized: Arc<AtomicBool>,
    ) -> Self {
        let (ctrl_tx, ctrl_rx) = crossbeam_channel::unbounded::<WatcherCtrl>();
        let wake = WinEvent::new().map(Arc::new);
        let wake_bg = wake.as_ref().map(Arc::clone);

        let worker = thread::Builder::new()
            .name("shaurmeaw-watch".to_owned())
            .spawn(move || {
                let mut roots = initial_roots;
                let mut active: Vec<(WatchDomain, PathBuf, DirChangeHandle)> = Vec::new();
                sync_dir_watches(&roots, &mut active);

                loop {
                    // Обработка ожидающих управляющих команд
                    let mut should_stop = false;
                    while let Ok(msg) = ctrl_rx.try_recv() {
                        match msg {
                            WatcherCtrl::UpdateRoots(next_roots) => {
                                roots = next_roots;
                                sync_dir_watches(&roots, &mut active);
                            }
                            WatcherCtrl::Shutdown => {
                                should_stop = true;
                                break;
                            }
                        }
                    }
                    if should_stop {
                        break;
                    }

                    let Some(ref ev) = wake_bg else {
                        match ctrl_rx.recv_timeout(Duration::from_secs(2)) {
                            Ok(WatcherCtrl::UpdateRoots(next_roots)) => {
                                roots = next_roots;
                                sync_dir_watches(&roots, &mut active);
                            }
                            Ok(WatcherCtrl::Shutdown)
                            | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        }
                        continue;
                    };

                    let handle_refs: Vec<&DirChangeHandle> =
                        active.iter().map(|(_, _, h)| h).collect();
                    let outcome = wait_event_or_dirs(ev, &handle_refs, WAIT_INFINITE);
                    match outcome {
                        WaitOutcome::Woken => continue,
                        WaitOutcome::DirClosed(idx) => {
                            if idx < active.len() {
                                active.remove(idx);
                            }
                            sync_dir_watches(&roots, &mut active);
                            continue;
                        }
                        WaitOutcome::DirChanged(first_idx) => {
                            let mut dirty = DirtyDomains::default();
                            if let Some((dom, _, _)) = active.get(first_idx) {
                                dirty.mark(*dom);
                            }

                            let started = Instant::now();
                            let mut stop_now = false;
                            while started.elapsed() < Duration::from_millis(NOTIFY_MAX_BATCH_MS) {
                                let refs: Vec<&DirChangeHandle> =
                                    active.iter().map(|(_, _, h)| h).collect();
                                match wait_event_or_dirs(ev, &refs, NOTIFY_DEBOUNCE_MS) {
                                    WaitOutcome::DirChanged(idx2) => {
                                        if let Some((dom, _, _)) = active.get(idx2) {
                                            dirty.mark(*dom);
                                        }
                                    }
                                    WaitOutcome::DirClosed(idx2) => {
                                        if idx2 < active.len() {
                                            active.remove(idx2);
                                        }
                                        sync_dir_watches(&roots, &mut active);
                                    }
                                    WaitOutcome::Woken => {
                                        while let Ok(msg) = ctrl_rx.try_recv() {
                                            match msg {
                                                WatcherCtrl::UpdateRoots(next_roots) => {
                                                    roots = next_roots;
                                                    sync_dir_watches(&roots, &mut active);
                                                }
                                                WatcherCtrl::Shutdown => {
                                                    stop_now = true;
                                                    break;
                                                }
                                            }
                                        }
                                        if stop_now {
                                            break;
                                        }
                                    }
                                    WaitOutcome::Timeout => break,
                                    WaitOutcome::Error => {
                                        thread::sleep(Duration::from_millis(50));
                                        break;
                                    }
                                }
                            }
                            if stop_now {
                                break;
                            }

                            if apply_watcher_dirty(&cache, &roots, dirty)
                                && !window_minimized.load(Ordering::Relaxed)
                            {
                                if let Some(ref c) = ctx {
                                    c.request_repaint();
                                }
                            }
                        }
                        WaitOutcome::Timeout => {}
                        WaitOutcome::Error => {
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            })
            .ok();

        Self {
            ctrl_tx,
            wake,
            worker: Mutex::new(worker),
        }
    }

    pub fn update_roots(&self, s: &Settings) {
        let _ = self
            .ctrl_tx
            .send(WatcherCtrl::UpdateRoots(WatchRoots::from_settings(s)));
        if let Some(ref ev) = self.wake {
            ev.signal();
        }
    }

    pub fn shutdown(&self) {
        let _ = self.ctrl_tx.send(WatcherCtrl::Shutdown);
        if let Some(ref ev) = self.wake {
            ev.signal();
        }
        if let Ok(mut guard) = self.worker.lock() {
            if let Some(h) = guard.take() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for PresetWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default, Clone, Copy)]
struct DirtyDomains {
    gi: bool,
    ae: bool,
    vid: bool,
    profiles: bool,
    addons: bool,
}

impl DirtyDomains {
    fn mark(&mut self, dom: WatchDomain) {
        match dom {
            WatchDomain::GiPresets | WatchDomain::GiBackups => self.gi = true,
            WatchDomain::AePresets => self.ae = true,
            WatchDomain::VidPresets => self.vid = true,
            WatchDomain::Profiles => self.profiles = true,
            WatchDomain::Addons => self.addons = true,
        }
    }

    fn any(self) -> bool {
        self.gi || self.ae || self.vid || self.profiles || self.addons
    }
}

fn sync_dir_watches(
    roots: &WatchRoots,
    active: &mut Vec<(WatchDomain, PathBuf, DirChangeHandle)>,
) {
    let desired: Vec<(WatchDomain, PathBuf)> = [
        (WatchDomain::GiPresets, Some(&roots.gi_presets)),
        (WatchDomain::GiBackups, Some(&roots.gi_backups)),
        (WatchDomain::AePresets, Some(&roots.ae_presets)),
        (WatchDomain::VidPresets, Some(&roots.vid_presets)),
        (WatchDomain::Profiles, Some(&roots.profiles)),
        (WatchDomain::Addons, roots.addons.as_ref()),
    ]
    .into_iter()
    .filter_map(|(dom, opt)| opt.filter(|p| p.is_dir()).map(|p| (dom, p.clone())))
    .collect();

    active.retain(|(dom, path, _)| {
        desired
            .iter()
            .any(|(d_dom, d_path)| d_dom == dom && d_path == path)
    });

    for (dom, dir) in desired {
        if active.iter().any(|(a_dom, _, _)| *a_dom == dom) {
            continue;
        }
        if let Some(h) = DirChangeHandle::open(&dir, false, NOTIFY_FILE_AND_WRITE) {
            active.push((dom, dir, h));
        }
    }
}

fn apply_watcher_dirty(cache: &SharedAppCache, roots: &WatchRoots, dirty: DirtyDomains) -> bool {
    if !dirty.any() {
        return false;
    }

    if dirty.gi {
        cache.reload_gameinfo_domain_in(
            &roots.gi_presets,
            &roots.gi_backups,
            &roots.steam_path,
            &roots.game_path,
            None,
            true,
        );
    }
    if dirty.ae {
        cache.reload_autoexec_domain_in(&roots.ae_presets, &roots.game_path, None, true);
    }
    if dirty.vid {
        cache.reload_video_domain_in(
            &roots.vid_presets,
            &roots.steam_path,
            &roots.game_path,
            roots.video_apply_all,
            None,
            true,
        );
    }
    if dirty.profiles {
        cache.reload_profiles_domain();
    }
    if dirty.addons {
        cache.reload_addons_domain(&roots.game_path);
    }
    true
}

// ============================================================ Двухканальный асинхронный IoWorker

pub enum DiskCmd {
    StartupHydrate {
        initial: Box<Settings>,
    },
    SaveSettings {
        settings: Box<Settings>,
    },
    ReadPresetText {
        kind: PresetKind,
        name: String,
    },
    SavePresetText {
        kind: PresetKind,
        name: String,
        content: String,
        source_url: Option<String>,
        and_apply: bool,
        settings: Option<Box<Settings>>,
    },
    ApplyGiPreset {
        settings: Box<Settings>,
        name: String,
    },
    DeleteGiPreset {
        name: String,
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    RenameGiPreset {
        old_name: String,
        new_name: String,
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    CreateGiFromCurrent {
        game_path: PathBuf,
        steam_path: PathBuf,
        name: String,
    },
    ImportGiFile {
        src: PathBuf,
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    RestoreGiBackup {
        backup_path: PathBuf,
        game_path: PathBuf,
        steam_path: PathBuf,
    },
    RestoreGiVanilla {
        game_path: PathBuf,
        steam_path: PathBuf,
    },
    CheckGiPatch {
        settings: Box<Settings>,
        explicit: bool,
    },
    RefreshGiState {
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    ApplyAePreset {
        settings: Box<Settings>,
        name: String,
    },
    DeleteAePreset {
        name: String,
        game_path: PathBuf,
    },
    CreateAeFromCurrent {
        game_path: PathBuf,
        name: String,
    },
    ImportAeFile {
        src: PathBuf,
        game_path: PathBuf,
    },
    WriteAutoexec {
        settings: Box<Settings>,
    },
    RestoreAutoexecBak {
        game_path: PathBuf,
    },
    ApplyVidPreset {
        settings: Box<Settings>,
        name: String,
    },
    DeleteVidPreset {
        name: String,
        steam_path: PathBuf,
        game_path: PathBuf,
        apply_all: bool,
    },
    CreateVidFromCurrent {
        steam_path: PathBuf,
        game_path: PathBuf,
        name: String,
        apply_all: bool,
    },
    ImportVidFile {
        src: PathBuf,
        steam_path: PathBuf,
        game_path: PathBuf,
        apply_all: bool,
    },
    ResetVidAutogen {
        settings: Box<Settings>,
    },
    RefreshVidTargets {
        steam_path: PathBuf,
        game_path: PathBuf,
        apply_all: bool,
    },
    SaveProfile {
        name: String,
        settings: Box<Settings>,
    },
    LoadProfile {
        name: String,
        current: Box<Settings>,
    },
    DeleteProfile {
        name: String,
    },
    ExportSettings {
        path: PathBuf,
        settings: Box<Settings>,
    },
    ImportSettings {
        path: PathBuf,
        current: Box<Settings>,
    },
    RefreshAddons {
        game_path: PathBuf,
    },
    ToggleAddonMod {
        path: PathBuf,
        game_path: PathBuf,
    },
    InstallDroppedFiles {
        paths: Vec<PathBuf>,
        game_path: PathBuf,
    },
    ResetAllToValve {
        settings: Box<Settings>,
    },
    AutoDetectPaths {
        settings: Box<Settings>,
        force: bool,
    },
    RefreshPathsState {
        steam_path: PathBuf,
        game_path: PathBuf,
        apply_all: bool,
    },
    InspectWindowsTweaks {
        game_path: PathBuf,
    },
    SetWindowsGameMode {
        enable: bool,
        game_path: PathBuf,
    },
    SetDisableGameDvr {
        disable: bool,
        game_path: PathBuf,
    },
    SetDeadlockHighPerfGpu {
        enable: bool,
        game_path: PathBuf,
    },
    SetDeadlockDisableFso {
        disable: bool,
        game_path: PathBuf,
    },
    ApplyAllWindowsTweaks {
        game_path: PathBuf,
    },
    RevertAllWindowsTweaks {
        game_path: PathBuf,
    },
    SetAutostart {
        enable: bool,
    },
    ToggleDesktopShortcut {
        create: bool,
    },
    TogglePortableMode {
        enable: bool,
        settings: Box<Settings>,
    },
    OpenDir {
        path: PathBuf,
        ensure_exists: bool,
    },
    PlayLaunch {
        settings: Box<Settings>,
        game_running: bool,
        sandbox: bool,
    },
    Shutdown,
}

pub enum HeavyCmd {
    ScanCache {
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    ClearShaderCache {
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    ClearCrashDumps {
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    ManualOptimize {
        steam_path: PathBuf,
        game_path: PathBuf,
        game_running: bool,
        before_ram: u64,
    },
    ImportGiUrl {
        url: String,
        name: String,
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    RefreshGiUrl {
        name: String,
        steam_path: PathBuf,
        game_path: PathBuf,
    },
    RefreshSteamDb {
        game_path: PathBuf,
        steam_path: PathBuf,
    },
    HashAddonsParallel {
        game_path: PathBuf,
    },
    Shutdown,
}

pub enum IoEvent {
    StartupHydrated {
        settings: Box<Settings>,
        selected_gi_preset: String,
        gi_text: String,
        selected_ae_preset: String,
        ae_text: String,
        selected_vid_preset: String,
        vid_text: String,
        status_ok: bool,
        status_msg: String,
        patch_banner: Option<String>,
    },
    SettingsSaved(Result<Box<Settings>, String>),
    PresetTextLoaded {
        kind: PresetKind,
        name: String,
        result: Result<String, String>,
    },
    PresetSaved {
        kind: PresetKind,
        name: String,
        and_apply: bool,
        result: Result<(), String>,
    },
    PresetCreatedOrImported {
        kind: PresetKind,
        select_name: Result<String, String>,
        status_msg: String,
    },
    GiPresetApplied {
        name: String,
        result: Result<String, String>,
    },
    GiPresetDeleted {
        deleted: String,
        result: Result<(), String>,
    },
    GiPresetRenamed {
        old_name: String,
        result: Result<String, String>,
    },
    GiPatchChecked {
        explicit: bool,
        report: Result<Option<gameinfo_sync::SyncReport>, String>,
    },
    AePresetApplied {
        name: String,
        result: Result<PathBuf, String>,
    },
    VidPresetApplied {
        name: String,
        result: Result<String, String>,
    },
    ProfileLoaded {
        result: Result<Box<Settings>, String>,
        status_msg: String,
    },
    ResetToValveCompleted {
        gi_res: Result<String, String>,
        ae_res: Result<PathBuf, String>,
        vid_res: Result<String, String>,
    },
    PathsDetected {
        settings: Box<Settings>,
        status_ok: bool,
        status_msg: String,
    },
    PortableToggled {
        enable: bool,
        result: Result<PathBuf, String>,
    },
    PlayCompleted {
        result: Result<String, String>,
        console_log_bytes: u64,
        updated_settings: Box<Settings>,
    },
    StatusOnly {
        ok: bool,
        msg: String,
    },
    CacheScanned,
    HeavyFinished {
        ok: bool,
        msg: String,
        select_gi: Option<String>,
        gi_text: Option<String>,
        clear_url_inputs: bool,
        refresh_valve_editor: bool,
    },
    ManualOptimized {
        before_ram: u64,
        console_log_bytes: u64,
        parts: Vec<String>,
    },
    AddonsHashed {
        ok: bool,
        msg: String,
    },
}

/// Двухканальный фоновый диспетчер дисковых и сетевых операций на `crossbeam-channel`.
pub struct IoWorker {
    disk_tx: Sender<DiskCmd>,
    heavy_tx: Sender<HeavyCmd>,
    event_rx: Receiver<IoEvent>,
    disk_thread: Option<JoinHandle<()>>,
    heavy_thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct UiNotifier {
    ctx: egui::Context,
    window_minimized: Arc<AtomicBool>,
}

impl UiNotifier {
    fn request_repaint(&self) {
        if !self.window_minimized.load(Ordering::Relaxed) {
            self.ctx.request_repaint();
        }
    }
}

impl IoWorker {
    pub fn spawn(
        cache: SharedAppCache,
        ctx: egui::Context,
        preset_watcher: PresetWatcher,
        window_minimized: Arc<AtomicBool>,
    ) -> (Self, Arc<PresetWatcher>) {
        let watcher = Arc::new(preset_watcher);
        let (disk_tx, disk_rx) = crossbeam_channel::unbounded::<DiskCmd>();
        let (heavy_tx, heavy_rx) = crossbeam_channel::unbounded::<HeavyCmd>();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<IoEvent>();
        let notifier = UiNotifier {
            ctx,
            window_minimized,
        };

        let cache_disk = cache.clone();
        let watcher_disk = Arc::clone(&watcher);
        let tx_disk = event_tx.clone();
        let ctx_disk = notifier.clone();

        let disk_thread = thread::Builder::new()
            .name("shaurmeaw-disk-io".to_owned())
            .spawn(move || {
                let mut pending: Option<DiskCmd> = None;
                loop {
                    let cmd = if let Some(c) = pending.take() {
                        c
                    } else {
                        match disk_rx.recv() {
                            Ok(c) => c,
                            Err(_) => break,
                        }
                    };

                    if matches!(cmd, DiskCmd::Shutdown) {
                        break;
                    }

                    // Дедуплицируем последовательные SaveSettings без потери других команд
                    let cmd = match cmd {
                        DiskCmd::SaveSettings { mut settings } => {
                            while let Ok(next) = disk_rx.try_recv() {
                                match next {
                                    DiskCmd::SaveSettings { settings: newer } => settings = newer,
                                    other => {
                                        pending = Some(other);
                                        break;
                                    }
                                }
                            }
                            DiskCmd::SaveSettings { settings }
                        }
                        other => other,
                    };

                    handle_disk_cmd(cmd, &cache_disk, &watcher_disk, &tx_disk, &ctx_disk);
                }
            })
            .ok();

        let cache_heavy = cache;
        let tx_heavy = event_tx;
        let ctx_heavy = notifier;
        let heavy_thread = thread::Builder::new()
            .name("shaurmeaw-heavy-io".to_owned())
            .spawn(move || {
                while let Ok(cmd) = heavy_rx.recv() {
                    if matches!(cmd, HeavyCmd::Shutdown) {
                        break;
                    }
                    handle_heavy_cmd(cmd, &cache_heavy, &tx_heavy, &ctx_heavy);
                }
            })
            .ok();

        (
            Self {
                disk_tx,
                heavy_tx,
                event_rx,
                disk_thread,
                heavy_thread,
            },
            watcher,
        )
    }

    pub fn send_disk(&self, cmd: DiskCmd) {
        let _ = self.disk_tx.send(cmd);
    }

    pub fn send_heavy(&self, cmd: HeavyCmd) {
        let _ = self.heavy_tx.send(cmd);
    }

    pub fn try_recv(&self) -> Option<IoEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn shutdown(&mut self) {
        let _ = self.disk_tx.send(DiskCmd::Shutdown);
        let _ = self.heavy_tx.send(HeavyCmd::Shutdown);
        if let Some(h) = self.disk_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.heavy_thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for IoWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn emit(tx: &Sender<IoEvent>, ctx: &UiNotifier, ev: IoEvent) {
    let _ = tx.send(ev);
    ctx.request_repaint();
}

fn auto_detect_paths_logic(s: &mut Settings, force: bool) -> (bool, String) {
    let need_steam = force || !steam::is_valid_steam_dir(Path::new(&s.steam_path));
    if need_steam {
        if let Some(p) = steam::find_steam_path() {
            s.steam_path = p.display().to_string();
        } else if force {
            s.steam_path.clear();
        }
    }

    let need_game = force || !steam::is_valid_game_dir(Path::new(&s.game_path));
    if need_game {
        let steam_dir = Path::new(&s.steam_path);
        let hint = (!s.steam_path.trim().is_empty() && steam_dir.exists()).then_some(steam_dir);
        match steam::find_deadlock_anywhere(hint) {
            Some(g) => {
                s.game_path = g.display().to_string();
                if !steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
                    if let Some(inferred) = steam::infer_steam_from_game(&g) {
                        s.steam_path = inferred.display().to_string();
                    }
                }
                if steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
                    (true, format!("Deadlock найден: {}", s.game_path))
                } else {
                    (
                        true,
                        "Deadlock найден (Steam не найден, запуск через steam://)".to_owned(),
                    )
                }
            }
            None => {
                let msg = if steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
                    "Deadlock не найден автоматически — укажите папку во вкладке «Пути»"
                } else {
                    "Steam и Deadlock не найдены автоматически — укажите пути во вкладке «Пути»"
                };
                (false, msg.to_owned())
            }
        }
    } else if steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
        (true, "Архив готов к запуску Deadlock".to_owned())
    } else {
        (
            true,
            "Deadlock найден (Steam не найден, запуск через steam://)".to_owned(),
        )
    }
}

fn handle_disk_cmd(
    cmd: DiskCmd,
    cache: &SharedAppCache,
    watcher: &Arc<PresetWatcher>,
    tx: &Sender<IoEvent>,
    ctx: &UiNotifier,
) {
    match cmd {
        DiskCmd::StartupHydrate { mut initial } => {
            gameinfo_presets::ensure_default_presets();
            autoexec_presets::ensure_default_presets();
            video_presets::ensure_default_presets();

            let (mut status_ok, mut status_msg) = auto_detect_paths_logic(&mut initial, false);
            let steam_path = PathBuf::from(&initial.steam_path);
            let game_path = PathBuf::from(&initial.game_path);

            let mut patch_banner = None;
            if initial.gameinfo.enabled && initial.gameinfo.auto_watch_updates {
                match gameinfo_sync::check_and_adapt_if_patched(&initial) {
                    Ok(Some(report)) => {
                        if report.is_patch_adaptation {
                            patch_banner = Some(report.message.clone());
                        }
                        status_ok = true;
                        status_msg = report.message;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        status_ok = false;
                        status_msg = e;
                    }
                }
            }

            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            cache.reload_autoexec_domain(&game_path, None);
            cache.reload_video_domain(
                &steam_path,
                &game_path,
                initial.video_apply_all_accounts,
                None,
            );
            cache.reload_profiles_domain();
            cache.reload_addons_domain(&game_path);
            cache.reload_paths_domain(&steam_path, &game_path);

            let (selected_gi_preset, selected_ae_preset, selected_vid_preset) = {
                let r = cache.read();
                let gi = if r
                    .gi_presets
                    .iter()
                    .any(|p| p.name == initial.gameinfo.active_preset)
                {
                    initial.gameinfo.active_preset.clone()
                } else {
                    r.gi_presets
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| initial.gameinfo.active_preset.clone())
                };
                let ae = if initial.autoexec_preset == MODE_BUILDER
                    || r.ae_presets.iter().any(|p| p.name == initial.autoexec_preset)
                {
                    initial.autoexec_preset.clone()
                } else {
                    autoexec_presets::PRESET_SHAURMEAW.to_owned()
                };
                let vid = if r.vid_presets.iter().any(|p| p.name == initial.video_preset) {
                    initial.video_preset.clone()
                } else {
                    video_presets::PRESET_SHAURMEAW.to_owned()
                };
                (gi, ae, vid)
            };

            let gi_text = gameinfo_presets::read_preset(&selected_gi_preset).unwrap_or_default();
            let ae_text = if selected_ae_preset == MODE_BUILDER {
                config_gen::build_autoexec(&initial)
            } else {
                autoexec_presets::read_preset(&selected_ae_preset).unwrap_or_default()
            };
            let vid_text = video_presets::read_preset(&selected_vid_preset).unwrap_or_default();

            let autostart_sync = gameinfo_sync::is_autostart_enabled();
            let has_shortcut = tools::has_desktop_shortcut();
            let console_log_bytes = game_booster::console_log_size(&game_path);
            let win_tweaks = game_booster::inspect_windows_tweaks(&game_path);

            {
                let mut w = cache.write();
                w.gi_texts.insert(selected_gi_preset.clone(), gi_text.clone());
                if selected_ae_preset != MODE_BUILDER {
                    w.ae_texts.insert(selected_ae_preset.clone(), ae_text.clone());
                }
                w.vid_texts.insert(selected_vid_preset.clone(), vid_text.clone());
                w.autostart_sync = autostart_sync;
                w.has_shortcut = has_shortcut;
                w.console_log_bytes = console_log_bytes;
                w.win_tweaks = Some(win_tweaks);
                w.hydrated = true;
            }
            cache.bump_revision();
            watcher.update_roots(&initial);

            emit(
                tx,
                ctx,
                IoEvent::StartupHydrated {
                    settings: initial,
                    selected_gi_preset,
                    gi_text,
                    selected_ae_preset,
                    ae_text,
                    selected_vid_preset,
                    vid_text,
                    status_ok,
                    status_msg,
                    patch_banner,
                },
            );
        }
        DiskCmd::SaveSettings { settings } => {
            let res = settings.save().map(|()| settings);
            emit(tx, ctx, IoEvent::SettingsSaved(res));
        }
        DiskCmd::ReadPresetText { kind, name } => {
            let result = match kind {
                PresetKind::GameInfo => gameinfo_presets::read_preset(&name),
                PresetKind::Autoexec => autoexec_presets::read_preset(&name),
                PresetKind::Video => video_presets::read_preset(&name),
            };
            if let Ok(ref text) = result {
                let mut w = cache.write();
                match kind {
                    PresetKind::GameInfo => {
                        w.gi_texts.insert(name.clone(), text.clone());
                    }
                    PresetKind::Autoexec => {
                        w.ae_texts.insert(name.clone(), text.clone());
                    }
                    PresetKind::Video => {
                        w.vid_texts.insert(name.clone(), text.clone());
                    }
                }
            }
            emit(tx, ctx, IoEvent::PresetTextLoaded { kind, name, result });
        }
        DiskCmd::SavePresetText {
            kind,
            name,
            content,
            source_url,
            and_apply,
            settings,
        } => {
            let save_res = match kind {
                PresetKind::GameInfo => {
                    let path = Settings::gameinfo_presets_dir().join(format!("{name}.gi"));
                    cache.mark_self_write(path);
                    gameinfo_presets::save_preset(&name, &content, source_url).map(|_| ())
                }
                PresetKind::Autoexec => {
                    let path = Settings::autoexec_presets_dir().join(format!("{name}.cfg"));
                    cache.mark_self_write(path);
                    autoexec_presets::save_preset(&name, &content).map(|_| ())
                }
                PresetKind::Video => {
                    let path = Settings::video_presets_dir().join(format!("{name}.txt"));
                    cache.mark_self_write(path);
                    video_presets::save_preset(&name, &content).map(|_| ())
                }
            };
            if save_res.is_ok() {
                {
                    let mut w = cache.write();
                    match kind {
                        PresetKind::GameInfo => {
                            w.gi_texts.insert(name.clone(), content.clone());
                        }
                        PresetKind::Autoexec => {
                            w.ae_texts.insert(name.clone(), content.clone());
                        }
                        PresetKind::Video => {
                            w.vid_texts.insert(name.clone(), content.clone());
                        }
                    }
                }
                if let Some(ref s) = settings {
                    let sp = PathBuf::from(&s.steam_path);
                    let gp = PathBuf::from(&s.game_path);
                    match kind {
                        PresetKind::GameInfo => cache.reload_gameinfo_domain(&sp, &gp, None),
                        PresetKind::Autoexec => cache.reload_autoexec_domain(&gp, None),
                        PresetKind::Video => {
                            cache.reload_video_domain(&sp, &gp, s.video_apply_all_accounts, None);
                        }
                    }
                } else {
                    cache.bump_revision();
                }
            }
            emit(
                tx,
                ctx,
                IoEvent::PresetSaved {
                    kind,
                    name: name.clone(),
                    and_apply,
                    result: save_res.clone(),
                },
            );
            if save_res.is_ok() && and_apply {
                if let Some(s) = settings {
                    match kind {
                        PresetKind::GameInfo => {
                            apply_gi_internal(&s, &name, cache, tx, ctx);
                        }
                        PresetKind::Autoexec => {
                            apply_ae_internal(&s, &name, cache, tx, ctx);
                        }
                        PresetKind::Video => {
                            apply_vid_internal(&s, &name, cache, tx, ctx);
                        }
                    }
                }
            }
        }
        DiskCmd::ApplyGiPreset { settings, name } => {
            apply_gi_internal(&settings, &name, cache, tx, ctx);
        }
        DiskCmd::DeleteGiPreset {
            name,
            steam_path,
            game_path,
        } => {
            let result = gameinfo_presets::delete_preset(&name);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            emit(
                tx,
                ctx,
                IoEvent::GiPresetDeleted {
                    deleted: name,
                    result,
                },
            );
        }
        DiskCmd::RenameGiPreset {
            old_name,
            new_name,
            steam_path,
            game_path,
        } => {
            let result = gameinfo_presets::rename_preset(&old_name, &new_name);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            emit(
                tx,
                ctx,
                IoEvent::GiPresetRenamed { old_name, result },
            );
        }
        DiskCmd::CreateGiFromCurrent {
            game_path,
            steam_path,
            name,
        } => {
            let res = gameinfo_presets::create_from_current(&game_path, &name);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            let status_msg = match &res {
                Ok(created) => format!("Профиль «{created}» создан из текущего файла игры"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::GameInfo,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::ImportGiFile {
            src,
            steam_path,
            game_path,
        } => {
            let res = gameinfo_presets::import_from_file(&src, None);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            let status_msg = match &res {
                Ok(created) => format!("Импортирован профиль «{created}»"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::GameInfo,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::RestoreGiBackup {
            backup_path,
            game_path,
            steam_path,
        } => {
            let res = gameinfo_presets::restore_backup(&backup_path, &game_path);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            match res {
                Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::RestoreGiVanilla {
            game_path,
            steam_path,
        } => {
            let res = gameinfo_sync::restore_vanilla(&game_path);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            match res {
                Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::CheckGiPatch { settings, explicit } => {
            let res = gameinfo_sync::check_and_adapt_if_patched(&settings);
            let sp = Path::new(&settings.steam_path);
            let gp = Path::new(&settings.game_path);
            cache.reload_gameinfo_domain(sp, gp, None);
            emit(
                tx,
                ctx,
                IoEvent::GiPatchChecked {
                    explicit,
                    report: res,
                },
            );
        }
        DiskCmd::RefreshGiState {
            steam_path,
            game_path,
        } => {
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            emit(
                tx,
                ctx,
                IoEvent::StatusOnly {
                    ok: true,
                    msg: "Статус gameinfo.gi и список пресетов обновлены".to_owned(),
                },
            );
        }
        DiskCmd::ApplyAePreset { settings, name } => {
            apply_ae_internal(&settings, &name, cache, tx, ctx);
        }
        DiskCmd::DeleteAePreset { name, game_path } => {
            let res = autoexec_presets::delete_preset(&name);
            cache.reload_autoexec_domain(&game_path, None);
            match res {
                Ok(()) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("Пресет «{name}» удалён"),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::CreateAeFromCurrent { game_path, name } => {
            let res = autoexec_presets::create_from_current(&game_path, &name);
            cache.reload_autoexec_domain(&game_path, None);
            let status_msg = match &res {
                Ok(created) => format!("Создан пресет autoexec.cfg «{created}»"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::Autoexec,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::ImportAeFile { src, game_path } => {
            let res = autoexec_presets::import_from_file(&src);
            cache.reload_autoexec_domain(&game_path, None);
            let status_msg = match &res {
                Ok(created) => format!("Импортирован пресет «{created}»"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::Autoexec,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::WriteAutoexec { settings } => {
            let res = launcher::write_autoexec(&settings);
            cache.reload_autoexec_domain(Path::new(&settings.game_path), None);
            match res {
                Ok(p) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("autoexec.cfg записан: {}", p.display()),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::RestoreAutoexecBak { game_path } => {
            let res = tools::restore_autoexec_backup(&game_path);
            cache.reload_autoexec_domain(&game_path, None);
            match res {
                Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::ApplyVidPreset { settings, name } => {
            apply_vid_internal(&settings, &name, cache, tx, ctx);
        }
        DiskCmd::DeleteVidPreset {
            name,
            steam_path,
            game_path,
            apply_all,
        } => {
            let res = video_presets::delete_preset(&name);
            cache.reload_video_domain(&steam_path, &game_path, apply_all, None);
            match res {
                Ok(()) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("Пресет «{name}» удалён"),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::CreateVidFromCurrent {
            steam_path,
            game_path,
            name,
            apply_all,
        } => {
            let res = video_presets::create_from_current(&steam_path, &game_path, &name);
            cache.reload_video_domain(&steam_path, &game_path, apply_all, None);
            let status_msg = match &res {
                Ok(created) => format!("Создан пресет video.txt «{created}»"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::Video,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::ImportVidFile {
            src,
            steam_path,
            game_path,
            apply_all,
        } => {
            let res = video_presets::import_from_file(&src);
            cache.reload_video_domain(&steam_path, &game_path, apply_all, None);
            let status_msg = match &res {
                Ok(created) => format!("Импортирован пресет «{created}»"),
                Err(e) => e.clone(),
            };
            emit(
                tx,
                ctx,
                IoEvent::PresetCreatedOrImported {
                    kind: PresetKind::Video,
                    select_name: res,
                    status_msg,
                },
            );
        }
        DiskCmd::ResetVidAutogen { settings } => {
            let res = video_presets::reset_video_for_valve_autogen(&settings);
            cache.reload_video_domain(
                Path::new(&settings.steam_path),
                Path::new(&settings.game_path),
                settings.video_apply_all_accounts,
                None,
            );
            match res {
                Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::RefreshVidTargets {
            steam_path,
            game_path,
            apply_all,
        } => {
            cache.reload_video_domain(&steam_path, &game_path, apply_all, None);
            ctx.request_repaint();
        }
        DiskCmd::SaveProfile { name, settings } => {
            let res = tools::save_profile(&name, &settings);
            cache.reload_profiles_domain();
            match res {
                Ok(saved_name) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("Профиль «{saved_name}» сохранён"),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::LoadProfile { name, current } => {
            let res = tools::load_profile(&name, &current);
            match res {
                Ok(loaded) => {
                    let _ = loaded.save();
                    let sp = PathBuf::from(&loaded.steam_path);
                    let gp = PathBuf::from(&loaded.game_path);
                    watcher.update_roots(&loaded);
                    cache.reload_paths_domain(&sp, &gp);
                    cache.reload_video_domain(&sp, &gp, loaded.video_apply_all_accounts, None);
                    cache.reload_autoexec_domain(&gp, None);
                    cache.reload_gameinfo_domain(&sp, &gp, None);
                    cache.reload_addons_domain(&gp);
                    emit(
                        tx,
                        ctx,
                        IoEvent::ProfileLoaded {
                            result: Ok(Box::new(loaded)),
                            status_msg: format!("Загружен профиль «{name}»"),
                        },
                    );
                }
                Err(e) => {
                    emit(
                        tx,
                        ctx,
                        IoEvent::ProfileLoaded {
                            result: Err(e.clone()),
                            status_msg: e,
                        },
                    );
                }
            }
        }
        DiskCmd::DeleteProfile { name } => {
            let res = tools::delete_profile(&name);
            cache.reload_profiles_domain();
            match res {
                Ok(()) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("Профиль «{name}» удалён"),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::ExportSettings { path, settings } => {
            let res = tools::export_settings(&path, &settings);
            match res {
                Ok(()) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: format!("Настройки экспортированы: {}", path.display()),
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::ImportSettings { path, current } => {
            let res = tools::import_settings(&path, &current);
            match res {
                Ok(loaded) => {
                    let _ = loaded.save();
                    let sp = PathBuf::from(&loaded.steam_path);
                    let gp = PathBuf::from(&loaded.game_path);
                    watcher.update_roots(&loaded);
                    cache.reload_paths_domain(&sp, &gp);
                    cache.reload_video_domain(&sp, &gp, loaded.video_apply_all_accounts, None);
                    cache.reload_autoexec_domain(&gp, None);
                    cache.reload_gameinfo_domain(&sp, &gp, None);
                    cache.reload_addons_domain(&gp);
                    emit(
                        tx,
                        ctx,
                        IoEvent::ProfileLoaded {
                            result: Ok(Box::new(loaded)),
                            status_msg: "Настройки импортированы из JSON".to_owned(),
                        },
                    );
                }
                Err(e) => {
                    emit(
                        tx,
                        ctx,
                        IoEvent::ProfileLoaded {
                            result: Err(e.clone()),
                            status_msg: e,
                        },
                    );
                }
            }
        }
        DiskCmd::RefreshAddons { game_path } => {
            cache.reload_addons_domain(&game_path);
            emit(
                tx,
                ctx,
                IoEvent::StatusOnly {
                    ok: true,
                    msg: "Список модов в addons/ обновлён".to_owned(),
                },
            );
        }
        DiskCmd::ToggleAddonMod { path, game_path } => {
            let res = tools::toggle_addon_mod(&path);
            cache.reload_addons_domain(&game_path);
            let msg = match res {
                Ok(true) => "Мод включён".to_owned(),
                Ok(false) => "Мод отключён".to_owned(),
                Err(e) => {
                    emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg: e });
                    return;
                }
            };
            emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg });
        }
        DiskCmd::InstallDroppedFiles { paths, game_path } => {
            let res = tools::install_dropped_files(&paths, &game_path);
            cache.reload_addons_domain(&game_path);
            match res {
                Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::ResetAllToValve { settings } => {
            let mut gi_settings = (*settings).clone();
            gi_settings.gameinfo.active_preset = gameinfo_presets::DEFAULT_PRESET_VALVE.to_owned();
            let gi_res = gameinfo_sync::apply_preset(&gi_settings, gameinfo_presets::DEFAULT_PRESET_VALVE)
                .map(|r| r.message);
            let mut ae_settings = (*settings).clone();
            ae_settings.autoexec_preset = autoexec_presets::PRESET_VALVE.to_owned();
            let ae_res = launcher::write_autoexec(&ae_settings);
            let vid_res = video_presets::reset_video_for_valve_autogen(&settings);

            let sp = Path::new(&settings.steam_path);
            let gp = Path::new(&settings.game_path);
            cache.reload_gameinfo_domain(sp, gp, None);
            cache.reload_autoexec_domain(gp, None);
            cache.reload_video_domain(sp, gp, settings.video_apply_all_accounts, None);

            emit(
                tx,
                ctx,
                IoEvent::ResetToValveCompleted {
                    gi_res,
                    ae_res,
                    vid_res,
                },
            );
        }
        DiskCmd::AutoDetectPaths { mut settings, force } => {
            let (status_ok, status_msg) = auto_detect_paths_logic(&mut settings, force);
            let sp = PathBuf::from(&settings.steam_path);
            let gp = PathBuf::from(&settings.game_path);
            watcher.update_roots(&settings);
            cache.reload_paths_domain(&sp, &gp);
            cache.reload_video_domain(&sp, &gp, settings.video_apply_all_accounts, None);
            cache.reload_autoexec_domain(&gp, None);
            cache.reload_gameinfo_domain(&sp, &gp, None);
            cache.reload_addons_domain(&gp);
            emit(
                tx,
                ctx,
                IoEvent::PathsDetected {
                    settings,
                    status_ok,
                    status_msg,
                },
            );
        }
        DiskCmd::RefreshPathsState {
            steam_path,
            game_path,
            apply_all,
        } => {
            cache.reload_paths_domain(&steam_path, &game_path);
            cache.reload_video_domain(&steam_path, &game_path, apply_all, None);
            cache.reload_gameinfo_domain(&steam_path, &game_path, None);
            cache.reload_addons_domain(&game_path);
            let fake_s = Settings {
                steam_path: steam_path.display().to_string(),
                game_path: game_path.display().to_string(),
                video_apply_all_accounts: apply_all,
                ..Default::default()
            };
            watcher.update_roots(&fake_s);
            ctx.request_repaint();
        }
        DiskCmd::InspectWindowsTweaks { game_path } => {
            let tw = game_booster::inspect_windows_tweaks(&game_path);
            {
                let mut w = cache.write();
                w.win_tweaks = Some(tw);
            }
            cache.bump_revision();
            ctx.request_repaint();
        }
        DiskCmd::SetWindowsGameMode { enable, game_path } => {
            let res = game_booster::set_windows_game_mode(enable).map(|()| {
                if enable {
                    "Игровой режим Windows включён".to_owned()
                } else {
                    "Игровой режим Windows отключён".to_owned()
                }
            });
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::SetDisableGameDvr { disable, game_path } => {
            let res = game_booster::set_disable_game_dvr(disable).map(|()| {
                if disable {
                    "Фоновая запись Xbox Game DVR отключена".to_owned()
                } else {
                    "Настройки Xbox Game DVR возвращены".to_owned()
                }
            });
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::SetDeadlockHighPerfGpu { enable, game_path } => {
            let res = game_booster::set_deadlock_high_perf_gpu(&game_path, enable).map(|()| {
                if enable {
                    "Для deadlock.exe задана дискретная GPU (High Performance)".to_owned()
                } else {
                    "Приоритет GPU для deadlock.exe сброшен".to_owned()
                }
            });
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::SetDeadlockDisableFso { disable, game_path } => {
            let res = game_booster::set_deadlock_disable_fso(&game_path, disable).map(|()| {
                if disable {
                    "Fullscreen Optimizations отключены для deadlock.exe".to_owned()
                } else {
                    "Fullscreen Optimizations возвращены к стандарту".to_owned()
                }
            });
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::ApplyAllWindowsTweaks { game_path } => {
            let res = game_booster::apply_all_windows_tweaks(&game_path);
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::RevertAllWindowsTweaks { game_path } => {
            let res = game_booster::revert_all_windows_tweaks(&game_path);
            refresh_tweaks_cache(cache, &game_path);
            emit_res(tx, ctx, res);
        }
        DiskCmd::SetAutostart { enable } => {
            let res = gameinfo_sync::set_autostart(enable);
            let actual = gameinfo_sync::is_autostart_enabled();
            {
                let mut w = cache.write();
                w.autostart_sync = actual;
            }
            cache.bump_revision();
            match res {
                Ok(()) => emit(
                    tx,
                    ctx,
                    IoEvent::StatusOnly {
                        ok: true,
                        msg: if enable {
                            "Фоновая автопроверка при входе в Windows включена".to_owned()
                        } else {
                            "Автопроверка при старте Windows отключена".to_owned()
                        },
                    },
                ),
                Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
            }
        }
        DiskCmd::ToggleDesktopShortcut { create } => {
            let (ok, msg) = if create {
                match tools::create_desktop_shortcut() {
                    Ok(p) => (true, format!("Ярлык создан на Рабочем столе: {}", p.display())),
                    Err(e) => (false, e),
                }
            } else {
                match tools::remove_desktop_shortcut() {
                    Ok(()) => (true, "Ярлык с рабочего стола удалён".to_owned()),
                    Err(e) => (false, e),
                }
            };
            let actual = tools::has_desktop_shortcut();
            {
                let mut w = cache.write();
                w.has_shortcut = actual;
            }
            cache.bump_revision();
            emit(tx, ctx, IoEvent::StatusOnly { ok, msg });
        }
        DiskCmd::TogglePortableMode { enable, settings } => {
            let result = if enable {
                settings.enable_portable_mode()
            } else {
                settings.disable_portable_mode()
            };
            if result.is_ok() {
                gameinfo_presets::ensure_default_presets();
                autoexec_presets::ensure_default_presets();
                video_presets::ensure_default_presets();
                let sp = Path::new(&settings.steam_path);
                let gp = Path::new(&settings.game_path);
                watcher.update_roots(&settings);
                cache.reload_paths_domain(sp, gp);
                cache.reload_profiles_domain();
                cache.reload_autoexec_domain(gp, Some(&[]));
                cache.reload_video_domain(sp, gp, settings.video_apply_all_accounts, Some(&[]));
                cache.reload_gameinfo_domain(sp, gp, Some(&[]));
            }
            emit(tx, ctx, IoEvent::PortableToggled { enable, result });
        }
        DiskCmd::OpenDir {
            path,
            ensure_exists,
        } => {
            if ensure_exists {
                let _ = std::fs::create_dir_all(&path);
            }
            if let Err(e) = launcher::open_folder(&path) {
                emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg: e });
            }
        }
        DiskCmd::PlayLaunch {
            settings,
            game_running,
            sandbox,
        } => {
            let _ = settings.save();
            let sp = PathBuf::from(&settings.steam_path);
            let gp = PathBuf::from(&settings.game_path);

            if settings.gameinfo.enabled && settings.gameinfo.rebuild_before_launch {
                if let Err(e) = gameinfo_sync::check_and_adapt_if_patched(&settings) {
                    cache.reload_gameinfo_domain(&sp, &gp, None);
                    emit(
                        tx,
                        ctx,
                        IoEvent::PlayCompleted {
                            result: Err(e),
                            console_log_bytes: game_booster::console_log_size(&gp),
                            updated_settings: settings,
                        },
                    );
                    return;
                }
                cache.reload_gameinfo_domain(&sp, &gp, None);
            }
            if settings.write_autoexec_on_launch {
                if let Err(e) = launcher::write_autoexec(&settings) {
                    emit(
                        tx,
                        ctx,
                        IoEvent::PlayCompleted {
                            result: Err(e),
                            console_log_bytes: game_booster::console_log_size(&gp),
                            updated_settings: settings,
                        },
                    );
                    return;
                }
                cache.reload_autoexec_domain(&gp, None);
            }
            if settings.write_video_on_launch {
                let preset = settings.video_preset.clone();
                if let Err(e) = video_presets::apply_video_preset(&settings, &preset) {
                    emit(
                        tx,
                        ctx,
                        IoEvent::PlayCompleted {
                            result: Err(e),
                            console_log_bytes: game_booster::console_log_size(&gp),
                            updated_settings: settings,
                        },
                    );
                    return;
                }
                cache.reload_video_domain(&sp, &gp, settings.video_apply_all_accounts, None);
            }
            if settings.auto_clean_console_log {
                let clog = game_booster::console_log_path(&gp);
                let _ = game_booster::clean_console_log_if_oversized(
                    &clog,
                    game_booster::CONSOLE_LOG_MAX_BYTES,
                    game_running,
                );
            }
            let console_log_bytes = game_booster::console_log_size(&gp);
            let res = if sandbox {
                launcher::launch_with_extra(&settings, &["+map", "street_test"])
                    .map(|_| "Запуск Deadlock на полигоне (Sandbox)…".to_owned())
            } else {
                launcher::launch(&settings)
            };
            if res.is_ok() && settings.ram_trim_on_minimize {
                game_booster::trim_memory();
            }
            {
                let mut w = cache.write();
                w.console_log_bytes = console_log_bytes;
            }
            cache.bump_revision();
            emit(
                tx,
                ctx,
                IoEvent::PlayCompleted {
                    result: res,
                    console_log_bytes,
                    updated_settings: settings,
                },
            );
        }
        DiskCmd::Shutdown => {}
    }
}

fn handle_heavy_cmd(
    cmd: HeavyCmd,
    cache: &SharedAppCache,
    tx: &Sender<IoEvent>,
    ctx: &UiNotifier,
) {
    match cmd {
        HeavyCmd::ScanCache {
            steam_path,
            game_path,
        } => {
            let stats = tools::scan_cache(&steam_path, &game_path);
            {
                let mut w = cache.write();
                w.cache_stats = Some(stats);
            }
            cache.bump_revision();
            emit(tx, ctx, IoEvent::CacheScanned);
        }
        HeavyCmd::ClearShaderCache {
            steam_path,
            game_path,
        } => {
            let res = tools::clear_shader_cache(&steam_path, &game_path);
            let ok = res.is_ok();
            let msg = match res {
                Ok(m) => {
                    let stats = tools::scan_cache(&steam_path, &game_path);
                    let mut w = cache.write();
                    w.cache_stats = Some(stats);
                    drop(w);
                    cache.bump_revision();
                    m
                }
                Err(e) => e,
            };
            emit(
                tx,
                ctx,
                IoEvent::HeavyFinished {
                    ok,
                    msg,
                    select_gi: None,
                    gi_text: None,
                    clear_url_inputs: false,
                    refresh_valve_editor: false,
                },
            );
        }
        HeavyCmd::ClearCrashDumps {
            steam_path,
            game_path,
        } => {
            let res = tools::clear_crash_dumps(&game_path);
            let ok = res.is_ok();
            let msg = match res {
                Ok(m) => {
                    let stats = tools::scan_cache(&steam_path, &game_path);
                    let mut w = cache.write();
                    w.cache_stats = Some(stats);
                    drop(w);
                    cache.bump_revision();
                    m
                }
                Err(e) => e,
            };
            emit(
                tx,
                ctx,
                IoEvent::HeavyFinished {
                    ok,
                    msg,
                    select_gi: None,
                    gi_text: None,
                    clear_url_inputs: false,
                    refresh_valve_editor: false,
                },
            );
        }
        HeavyCmd::ManualOptimize {
            steam_path,
            game_path,
            game_running,
            before_ram,
        } => {
            let mut parts: Vec<String> = Vec::new();
            if !game_running {
                if let Ok(msg) = tools::clear_shader_cache(&steam_path, &game_path) {
                    if msg != "Кэш шейдеров уже пуст" {
                        parts.push(msg);
                    }
                }
                if let Ok(dumps) = tools::clear_crash_dumps(&game_path) {
                    if dumps != "Краш-дампы не найдены" {
                        parts.push(dumps);
                    }
                }
                let clog = game_booster::console_log_path(&game_path);
                if let Ok(Some(freed)) = game_booster::clean_console_log_if_oversized(
                    &clog,
                    game_booster::CONSOLE_LOG_MAX_BYTES,
                    false,
                ) {
                    parts.push(format!("console.log обрезан (−{})", tools::format_size(freed)));
                }
                let stats = tools::scan_cache(&steam_path, &game_path);
                let mut w = cache.write();
                w.cache_stats = Some(stats);
            } else {
                parts.push("игра запущена: кэш и console.log не тронуты".to_owned());
            }

            let console_log_bytes = game_booster::console_log_size(&game_path);
            {
                let mut w = cache.write();
                w.console_log_bytes = console_log_bytes;
            }
            cache.bump_revision();

            emit(
                tx,
                ctx,
                IoEvent::ManualOptimized {
                    before_ram,
                    console_log_bytes,
                    parts,
                },
            );
        }
        HeavyCmd::ImportGiUrl {
            url,
            name,
            steam_path,
            game_path,
        } => match gameinfo_presets::import_from_url(&url, &name) {
            Ok(created) => {
                let text = gameinfo_presets::read_preset(&created).unwrap_or_default();
                cache.reload_gameinfo_domain(&steam_path, &game_path, None);
                {
                    let mut w = cache.write();
                    w.gi_texts.insert(created.clone(), text.clone());
                }
                cache.bump_revision();
                emit(
                    tx,
                    ctx,
                    IoEvent::HeavyFinished {
                        ok: true,
                        msg: format!("Профиль «{created}» скачан по ссылке"),
                        select_gi: Some(created),
                        gi_text: Some(text),
                        clear_url_inputs: true,
                        refresh_valve_editor: false,
                    },
                );
            }
            Err(e) => emit(
                tx,
                ctx,
                IoEvent::HeavyFinished {
                    ok: false,
                    msg: e,
                    select_gi: None,
                    gi_text: None,
                    clear_url_inputs: false,
                    refresh_valve_editor: false,
                },
            ),
        },
        HeavyCmd::RefreshGiUrl {
            name,
            steam_path,
            game_path,
        } => match gameinfo_presets::refresh_from_url(&name) {
            Ok(updated) => {
                let text = gameinfo_presets::read_preset(&updated).unwrap_or_default();
                cache.reload_gameinfo_domain(&steam_path, &game_path, None);
                {
                    let mut w = cache.write();
                    w.gi_texts.insert(updated.clone(), text.clone());
                }
                cache.bump_revision();
                emit(
                    tx,
                    ctx,
                    IoEvent::HeavyFinished {
                        ok: true,
                        msg: format!("Профиль «{updated}» обновлён по ссылке"),
                        select_gi: Some(updated),
                        gi_text: Some(text),
                        clear_url_inputs: false,
                        refresh_valve_editor: false,
                    },
                );
            }
            Err(e) => emit(
                tx,
                ctx,
                IoEvent::HeavyFinished {
                    ok: false,
                    msg: e,
                    select_gi: None,
                    gi_text: None,
                    clear_url_inputs: false,
                    refresh_valve_editor: false,
                },
            ),
        },
        HeavyCmd::RefreshSteamDb {
            game_path,
            steam_path,
        } => match gameinfo_sync::refresh_official_vanilla_from_steamdb(&game_path) {
            Ok(msg) => {
                let valve_text =
                    gameinfo_presets::read_preset(gameinfo_presets::DEFAULT_PRESET_VALVE)
                        .unwrap_or_default();
                cache.reload_gameinfo_domain(&steam_path, &game_path, None);
                {
                    let mut w = cache.write();
                    w.gi_texts.insert(
                        gameinfo_presets::DEFAULT_PRESET_VALVE.to_owned(),
                        valve_text.clone(),
                    );
                }
                cache.bump_revision();
                emit(
                    tx,
                    ctx,
                    IoEvent::HeavyFinished {
                        ok: true,
                        msg,
                        select_gi: None,
                        gi_text: Some(valve_text),
                        clear_url_inputs: false,
                        refresh_valve_editor: true,
                    },
                );
            }
            Err(e) => emit(
                tx,
                ctx,
                IoEvent::HeavyFinished {
                    ok: false,
                    msg: e,
                    select_gi: None,
                    gi_text: None,
                    clear_url_inputs: false,
                    refresh_valve_editor: false,
                },
            ),
        },
        HeavyCmd::HashAddonsParallel { game_path } => {
            let mods = tools::list_addon_mods(&game_path);
            let hashed = tools::hash_addon_mods_parallel(&mods);
            let mut ok_count = 0usize;
            let mut err_count = 0usize;
            {
                let mut w = cache.write();
                w.addon_hashes.clear();
                for (path, res) in hashed {
                    match res {
                        Ok(hex) => {
                            ok_count += 1;
                            w.addon_hashes.insert(path, hex);
                        }
                        Err(_) => {
                            err_count += 1;
                        }
                    }
                }
            }
            cache.bump_revision();
            let (ok, msg) = if err_count == 0 {
                (
                    true,
                    format!("SHA-256 посчитан (64 КБ чанки, параллельно) для {ok_count} модов"),
                )
            } else {
                (
                    false,
                    format!("SHA-256: готово {ok_count}, ошибок {err_count}"),
                )
            };
            emit(tx, ctx, IoEvent::AddonsHashed { ok, msg });
        }
        HeavyCmd::Shutdown => {}
    }
}

fn apply_gi_internal(
    settings: &Settings,
    name: &str,
    cache: &SharedAppCache,
    tx: &Sender<IoEvent>,
    ctx: &UiNotifier,
) {
    let mut s = settings.clone();
    s.gameinfo.active_preset = name.to_owned();
    let result = gameinfo_sync::apply_preset(&s, name).map(|r| r.message);
    cache.reload_gameinfo_domain(Path::new(&s.steam_path), Path::new(&s.game_path), None);
    emit(
        tx,
        ctx,
        IoEvent::GiPresetApplied {
            name: name.to_owned(),
            result,
        },
    );
}

fn apply_ae_internal(
    settings: &Settings,
    name: &str,
    cache: &SharedAppCache,
    tx: &Sender<IoEvent>,
    ctx: &UiNotifier,
) {
    let mut s = settings.clone();
    s.autoexec_preset = name.to_owned();
    let result = launcher::write_autoexec(&s);
    cache.reload_autoexec_domain(Path::new(&s.game_path), None);
    emit(
        tx,
        ctx,
        IoEvent::AePresetApplied {
            name: name.to_owned(),
            result,
        },
    );
}

fn apply_vid_internal(
    settings: &Settings,
    name: &str,
    cache: &SharedAppCache,
    tx: &Sender<IoEvent>,
    ctx: &UiNotifier,
) {
    let result = video_presets::apply_video_preset(settings, name);
    cache.reload_video_domain(
        Path::new(&settings.steam_path),
        Path::new(&settings.game_path),
        settings.video_apply_all_accounts,
        None,
    );
    emit(
        tx,
        ctx,
        IoEvent::VidPresetApplied {
            name: name.to_owned(),
            result,
        },
    );
}

fn refresh_tweaks_cache(cache: &SharedAppCache, game_path: &Path) {
    let tw = game_booster::inspect_windows_tweaks(game_path);
    {
        let mut w = cache.write();
        w.win_tweaks = Some(tw);
    }
    cache.bump_revision();
}

fn emit_res(tx: &Sender<IoEvent>, ctx: &UiNotifier, res: Result<String, String>) {
    match res {
        Ok(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: true, msg }),
        Err(msg) => emit(tx, ctx, IoEvent::StatusOnly { ok: false, msg }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_app_cache_notify_watcher_detects_preset_change() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_notify_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let gi_dir = root.join("gi");
        let bak_dir = root.join("bak");
        let ae_dir = root.join("ae");
        let vid_dir = root.join("vid");
        let prof_dir = root.join("prof");
        for d in [&gi_dir, &bak_dir, &ae_dir, &vid_dir, &prof_dir] {
            fs::create_dir_all(d).unwrap();
        }

        let cache = SharedAppCache::new();
        {
            let mut w = cache.write();
            w.gi_texts.insert("Custom".to_owned(), "old_text".to_owned());
        }
        let rev_before = cache.revision();

        let roots = WatchRoots {
            gi_presets: gi_dir.clone(),
            gi_backups: bak_dir,
            ae_presets: ae_dir,
            vid_presets: vid_dir,
            profiles: prof_dir,
            addons: None,
            steam_path: root.clone(),
            game_path: root.clone(),
            video_apply_all: false,
        };
        let min_flag = Arc::new(AtomicBool::new(false));
        let watcher = PresetWatcher::spawn_with_roots(cache.clone(), roots, None, min_flag);

        // Небольшая пауза на регистрацию ReadDirectoryChangesW
        thread::sleep(Duration::from_millis(80));
        fs::write(gi_dir.join("Custom.gi"), b"\"GameInfo\" { }").unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if cache.revision() > rev_before && !cache.read().gi_texts.contains_key("Custom") {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }

        assert!(
            cache.revision() > rev_before,
            "notify watcher должен инкрементировать revision при записи пресета"
        );
        assert!(
            !cache.read().gi_texts.contains_key("Custom"),
            "кэш текста изменённого пресета должен быть инвалидирован"
        );

        watcher.shutdown();
        let _ = fs::remove_dir_all(&root);
    }
}
