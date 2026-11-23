//! Минималистичный интерфейс лаунчера ShaurMeaw в эстетике Deadlock (Оккультный нуар + Арт-деко 1930-х).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::app_cache::{
    DiskCmd, HeavyCmd, IoEvent, IoWorker, PresetKind, PresetWatcher, SharedAppCache,
};
use crate::autoexec_presets::{self, AutoexecPresetEntry, MODE_BUILDER};
use crate::config_editor::{self, BraceCheck, ConfigKind, ParsedEntry, TextMatch};
use crate::config_gen;
use crate::discord_rpc::{DiscordRpcService, RpcConnectionStatus, RpcSnapshot};
use crate::game_booster;
use crate::gameinfo_presets::{self, BackupEntry, PresetEntry};
use crate::gameinfo_sync::{GameInfoWatcher, SyncStatus};
use crate::map_timers::{self, MatchTimerState};
use crate::settings::{
    Bind, Crosshair, FovPreset, LaunchMode, OptimizationProfile, Preset, RenderApi, Settings,
    Theme, WindowMode,
};
use crate::sfx::{self, UiSound};
use crate::steam;
use crate::theme::{self, HighlightMode, Icon, Palette};
use crate::tools::{self, AddonModEntry, CacheStats, ProcessMonitor};
use crate::tray::TrayController;
use crate::video_presets::{self, VideoPresetEntry};

const LANGUAGES: [(&str, &str); 6] = [
    ("", "По умолчанию"),
    ("russian", "Русский"),
    ("english", "English"),
    ("ukrainian", "Українська"),
    ("german", "Deutsch"),
    ("schinese", "Китайский (упр.)"),
];

const CROSSHAIR_BGS: [(&str, Color32); 4] = [
    ("Нуар", Color32::from_rgb(20, 22, 26)),
    ("Грифель", Color32::from_rgb(70, 82, 96)),
    ("Пергамент", Color32::from_rgb(208, 202, 190)),
    ("Небо", Color32::from_rgb(106, 152, 198)),
];

const COMMUNITY_LINKS: [(&str, &str, &str); 4] = [
    ("Tracklock", "Статистика и билды героев", "https://tracklock.gg"),
    ("Deadlock Tracker", "Профили и текущая мета", "https://deadlocktracker.gg"),
    ("Deadlock Wiki", "Герои, предметы и патчи", "https://deadlocked.wiki"),
    ("SteamDB", "Обновления и онлайн", "https://steamdb.info/app/1422450/"),
];

const OCCULT_EASTER_PHRASES: [&str; 8] = [
    "Шаурма благоволит тебе... с двойным чесночным.",
    "Настоящий воин не сдаётся, пока лаваш хрустит.",
    "Секретный ингредиент победы в Deadlock — горячая шаурма.",
    "Внутри каждого сильного конфига скрыт идеальный соус.",
    "Катка без шаурмы — как ульта без душ.",
    "Мяу! Лаваш скручен идеально, можно заходить в матч.",
    "Две шаурмы в лаваше дают +100% к точности и FPS.",
    "Кто познал вкус ночной шаурмы — тому не страшен никакой патч.",
];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Home,
    Configs,
    Mods,
    Utilities,
    Settings,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Home,
        Tab::Configs,
        Tab::Mods,
        Tab::Utilities,
        Tab::Settings,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Home => "Главная",
            Tab::Configs => "Конфиги",
            Tab::Mods => "Моды",
            Tab::Utilities => "Утилиты",
            Tab::Settings => "Настройки",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Tab::Home => "Запуск Deadlock и выбор профилей gameinfo.gi",
            Tab::Configs => "Профили autoexec.cfg, настройки графики video.txt и конструктор команд",
            Tab::Mods => "Установка и управление .vpk модами Deadlock (citadel/addons)",
            Tab::Utilities => "Очистка шейдеров, ускорение ОЗУ, тайминги карты и полигон",
            Tab::Settings => "Пути игры, трей, статус Discord и звук интерфейса",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Home => Icon::Launch,
            Tab::Configs => Icon::Edit,
            Tab::Mods => Icon::Archive,
            Tab::Utilities => Icon::Tools,
            Tab::Settings => Icon::Settings,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigSubTab {
    Autoexec,
    Video,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum EditorViewMode {
    #[default]
    Code,
    Commands,
}

/// Кэшированное состояние поиска, режима «Команды» и справочника в модальном редакторе конфигов.
struct EditorUiState {
    view_mode: EditorViewMode,
    search_query: String,
    replace_query: String,
    show_replace: bool,
    case_sensitive: bool,
    active_match: usize,
    scroll_to_line: Option<usize>,
    select_char_range: Option<(usize, usize)>,
    focus_search_box: bool,
    catalog_open: bool,
    catalog_category: String,
    catalog_filter: String,
    custom_key: String,
    custom_value: String,
    cmd_only_active: bool,
    text_rev: u64,
    parsed_key: Option<(ConfigKind, u64)>,
    match_key: Option<(ConfigKind, u64, String, bool)>,
    filtered_key: Option<(ConfigKind, u64, String, bool)>,
    cached_line_count: usize,
    cached_matches: Vec<TextMatch>,
    cached_entries: Vec<ParsedEntry>,
    cached_filtered_entries: Vec<usize>,
    cached_present_keys: HashSet<String>,
    cached_brace_check: Option<BraceCheck>,
    galley_cache: theme::ManuscriptGalleyCache,
}

impl Default for EditorUiState {
    fn default() -> Self {
        Self {
            view_mode: EditorViewMode::Code,
            search_query: String::new(),
            replace_query: String::new(),
            show_replace: false,
            case_sensitive: false,
            active_match: 0,
            scroll_to_line: None,
            select_char_range: None,
            focus_search_box: false,
            catalog_open: true,
            catalog_category: String::new(),
            catalog_filter: String::new(),
            custom_key: String::new(),
            custom_value: String::new(),
            cmd_only_active: false,
            text_rev: 1,
            parsed_key: None,
            match_key: None,
            filtered_key: None,
            cached_line_count: 1,
            cached_matches: Vec::new(),
            cached_entries: Vec::new(),
            cached_filtered_entries: Vec::new(),
            cached_present_keys: HashSet::new(),
            cached_brace_check: None,
            galley_cache: theme::ManuscriptGalleyCache::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupStage {
    PendingFirstFrame,
    SpawnDeferred,
    Ready,
}

pub struct LauncherApp {
    s: Settings,
    saved: Settings,
    saving_settings_snapshot: Option<Settings>,
    applied_theme: Theme,
    applied_ui_scale: f32,
    applied_animations: bool,
    tab: Tab,
    config_subtab: ConfigSubTab,

    status: String,
    status_ok: bool,
    toast_until: Option<Instant>,
    avatar: egui::TextureHandle,
    monitor: Option<ProcessMonitor>,
    profiles: Vec<String>,
    new_profile_name: String,
    cache_stats: Option<CacheStats>,
    crosshair_bg: usize,

    tab_switched_at: f64,
    pending_save_since: Option<f64>,
    cached_launch_line: String,
    cached_ae_preview: String,
    previews_synced: bool,

    // --- autoexec.cfg пресеты
    ae_presets: Vec<AutoexecPresetEntry>,
    selected_ae_preset: String,
    ae_editor_text: String,
    ae_editor_dirty: bool,
    new_ae_preset_name: String,

    // --- video.txt пресеты
    vid_presets: Vec<VideoPresetEntry>,
    selected_vid_preset: String,
    vid_editor_text: String,
    vid_editor_dirty: bool,
    new_vid_preset_name: String,

    // --- gameinfo.gi пресеты и автозащита
    gi_status: SyncStatus,
    gi_watcher: Option<GameInfoWatcher>,
    gi_presets: Vec<PresetEntry>,
    gi_backups: Vec<BackupEntry>,
    selected_gi_preset: String,
    gi_editor_text: String,
    gi_editor_dirty: bool,
    new_gi_preset_name: String,
    rename_gi_name: String,
    url_gi_input: String,
    url_gi_name: String,
    patch_banner: Option<String>,
    autostart_sync: bool,
    last_watcher_key: (String, String, crate::settings::GameInfoConfig),
    last_watcher_game_up: bool,
    discord_rpc: Option<DiscordRpcService>,
    last_rpc_sync: (crate::settings::DiscordRpcConfig, String, String, String, bool, Option<u32>),
    cached_rpc_rev: u64,
    cached_rpc_snap: RpcSnapshot,
    last_booster_opts: (bool, bool, bool, bool, bool),
    last_auto_english_layout: bool,

    // --- моды addons/
    addon_mods: Vec<AddonModEntry>,
    addon_hashes: HashMap<PathBuf, String>,
    hashing_addons: bool,
    dmm_conflicts: usize,
    has_shortcut: bool,

    // --- кэшированное состояние путей и бэкапов
    vid_targets: Vec<PathBuf>,
    paths_steam_ok: bool,
    paths_game_ok: bool,
    is_portable: bool,
    cached_app_dir: PathBuf,
    cached_settings_file: PathBuf,
    has_autoexec_bak: bool,
    has_vanilla_bak: bool,

    // --- нативная иконка окна, трей и пасхалки
    main_hwnd: usize,
    tray: Option<TrayController>,
    logo_click_times: Vec<Instant>,
    occult_easter_until: Option<Instant>,
    occult_phrase_idx: usize,
    midnight_easter_until: Option<Instant>,
    easter_prev_status: Option<(bool, String)>,

    // --- модальные окна и шпаргалка таймеров карты
    code_modal: Option<ConfigKind>,
    editor_ui: EditorUiState,
    new_preset_modal_open: bool,
    new_preset_tab: u8,
    new_cfg_modal: Option<ConfigSubTab>,
    rename_modal_target: Option<String>,
    gi_tweaks_modal_open: bool,
    map_timers_modal_open: bool,
    map_timer: MatchTimerState,

    // --- оптимизация ресурсов, телеметрия и состояние окна
    window_minimized: Arc<AtomicBool>,
    shared_game_running: Arc<AtomicBool>,
    was_minimized: bool,
    was_hidden: bool,
    hidden_in_tray: bool,
    restore_repaint_frames: u8,
    was_game_running: bool,
    cached_ram_bytes: u64,
    cached_console_log_bytes: u64,
    last_telemetry_at: Option<Instant>,
    cached_win_tweaks: Option<game_booster::WindowsTweaksStatus>,

    // --- ленивый старт, In-Memory Cache и асинхронный I/O воркер
    startup_stage: StartupStage,
    cache: SharedAppCache,
    applied_cache_rev: u64,
    io_worker: Option<IoWorker>,
    preset_watcher: Option<Arc<PresetWatcher>>,
    heavy_busy: bool,
    heavy_busy_label: Option<String>,
    cache_scanning: bool,

    // --- автообновление из GitHub Releases
    update_available: Option<(String, String)>,
    update_applying: bool,
    update_rx: Option<crossbeam_channel::Receiver<(String, String)>>,
    update_err_rx: Option<crossbeam_channel::Receiver<String>>,
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = Settings::load();
        theme::setup_fonts_and_style(&cc.egui_ctx, settings.theme);

        let clamped_scale = settings.ui_scale.clamp(0.85, 1.25);
        settings.ui_scale = clamped_scale;
        settings.editor_font_size = settings.editor_font_size.clamp(11.0, 17.0);
        settings.ui_sound_volume = settings.ui_sound_volume.clamp(0.0, 1.0);
        cc.egui_ctx.set_zoom_factor(clamped_scale);
        let anims_enabled = settings.animations_enabled;
        cc.egui_ctx.style_mut(|st| {
            st.animation_time = if anims_enabled {
                theme::ANIM_DURATION_SEC
            } else {
                0.0
            };
        });

        if settings.gameinfo.active_preset == gameinfo_presets::LEGACY_PRESET_VANILLA {
            settings.gameinfo.active_preset = gameinfo_presets::DEFAULT_PRESET_VALVE.to_owned();
        }

        let avatar_img =
            egui::ColorImage::from_rgba_unmultiplied([128, 128], crate::embedded::icon_128_rgba());
        let avatar = cc.egui_ctx.load_texture(
            "shaurmeaw_avatar",
            avatar_img,
            egui::TextureOptions::LINEAR,
        );

        let selected_gi_preset = settings.gameinfo.active_preset.clone();
        let selected_ae_preset = settings.autoexec_preset.clone();
        let ae_editor_text = if selected_ae_preset == MODE_BUILDER {
            config_gen::build_autoexec(&settings)
        } else {
            String::new()
        };
        let selected_vid_preset = settings.video_preset.clone();

        let last_rpc_sync = (
            settings.discord_rpc.clone(),
            settings.game_path.clone(),
            settings.steam_path.clone(),
            settings.gameinfo.active_preset.clone(),
            false,
            None,
        );
        let init_booster_opts = (
            settings.auto_high_priority,
            settings.ram_trim_on_minimize,
            settings.boost_timer_1ms,
            settings.boost_power_plan,
            settings.boost_disable_cpu0,
        );
        let init_auto_en = settings.auto_english_layout;
        let saved = settings.clone();
        let mut app = Self {
            applied_theme: settings.theme,
            applied_ui_scale: settings.ui_scale,
            applied_animations: settings.animations_enabled,
            last_watcher_key: (
                settings.steam_path.clone(),
                settings.game_path.clone(),
                settings.gameinfo.clone(),
            ),
            monitor: None,
            s: settings,
            saved,
            saving_settings_snapshot: None,
            tab: Tab::Home,
            config_subtab: ConfigSubTab::Autoexec,
            status: "Загрузка профилей…".to_owned(),
            status_ok: true,
            toast_until: Some(Instant::now() + Duration::from_secs(4)),
            avatar,
            profiles: Vec::new(),
            new_profile_name: String::new(),
            cache_stats: None,
            crosshair_bg: 0,
            tab_switched_at: -10.0,
            pending_save_since: None,
            cached_launch_line: String::new(),
            cached_ae_preview: String::new(),
            previews_synced: true,
            ae_presets: Vec::new(),
            selected_ae_preset,
            ae_editor_text,
            ae_editor_dirty: false,
            new_ae_preset_name: String::new(),
            vid_presets: Vec::new(),
            selected_vid_preset,
            vid_editor_text: String::new(),
            vid_editor_dirty: false,
            new_vid_preset_name: String::new(),
            gi_status: SyncStatus::GameNotFound,
            gi_watcher: None,
            gi_presets: Vec::new(),
            gi_backups: Vec::new(),
            selected_gi_preset,
            gi_editor_text: String::new(),
            gi_editor_dirty: false,
            new_gi_preset_name: String::new(),
            rename_gi_name: String::new(),
            url_gi_input: String::new(),
            url_gi_name: String::new(),
            patch_banner: None,
            autostart_sync: false,
            last_watcher_game_up: false,
            discord_rpc: None,
            last_rpc_sync,
            cached_rpc_rev: 0,
            cached_rpc_snap: RpcSnapshot::default(),
            last_booster_opts: init_booster_opts,
            last_auto_english_layout: init_auto_en,
            addon_mods: Vec::new(),
            addon_hashes: HashMap::new(),
            hashing_addons: false,
            dmm_conflicts: 0,
            has_shortcut: false,
            vid_targets: Vec::new(),
            paths_steam_ok: false,
            paths_game_ok: false,
            is_portable: false,
            cached_app_dir: Settings::app_dir(),
            cached_settings_file: Settings::file_path(),
            has_autoexec_bak: false,
            has_vanilla_bak: false,
            main_hwnd: 0,
            tray: None,
            logo_click_times: Vec::with_capacity(8),
            occult_easter_until: None,
            occult_phrase_idx: 0,
            midnight_easter_until: None,
            easter_prev_status: None,
            code_modal: None,
            editor_ui: EditorUiState::default(),
            new_preset_modal_open: false,
            new_preset_tab: 0,
            new_cfg_modal: None,
            rename_modal_target: None,
            gi_tweaks_modal_open: false,
            map_timers_modal_open: false,
            map_timer: MatchTimerState::default(),
            window_minimized: Arc::new(AtomicBool::new(false)),
            shared_game_running: Arc::new(AtomicBool::new(false)),
            was_minimized: false,
            was_hidden: false,
            hidden_in_tray: false,
            restore_repaint_frames: 0,
            was_game_running: false,
            cached_ram_bytes: 0,
            cached_console_log_bytes: 0,
            last_telemetry_at: None,
            cached_win_tweaks: None,
            startup_stage: StartupStage::PendingFirstFrame,
            cache: SharedAppCache::new(),
            applied_cache_rev: 0,
            io_worker: None,
            preset_watcher: None,
            heavy_busy: false,
            heavy_busy_label: None,
            cache_scanning: false,
            update_available: None,
            update_applying: false,
            update_rx: None,
            update_err_rx: None,
        };
        app.refresh_cached_previews();
        app.previews_synced = true;
        if is_midnight_hunt_now() {
            app.easter_prev_status = Some((app.status_ok, app.status.clone()));
            app.status_ok = true;
            app.status = "Ночной рейд за шаурмой открыт...".to_owned();
            app.midnight_easter_until = Some(Instant::now() + Duration::from_millis(2500));
        }
        app
    }

    fn spawn_background_services(&mut self, ctx: &egui::Context) {
        self.cached_ram_bytes = game_booster::current_working_set_bytes();
        let watcher = GameInfoWatcher::spawn(
            &self.s,
            ctx.clone(),
            Arc::clone(&self.window_minimized),
            Arc::clone(&self.shared_game_running),
        );
        watcher.sync_config(&self.s, false);
        let discord_rpc = DiscordRpcService::spawn(
            &self.s,
            ctx.clone(),
            Arc::clone(&self.window_minimized),
            Arc::clone(&self.shared_game_running),
        );

        let mut extra_wakes = Vec::new();
        if let Some(w) = watcher.wake_handle() {
            extra_wakes.push(w);
        }
        if let Some(w) = discord_rpc.wake_handle() {
            extra_wakes.push(w);
        }

        self.monitor = Some(ProcessMonitor::spawn(
            ctx.clone(),
            &self.s,
            Arc::clone(&self.window_minimized),
            Arc::clone(&self.shared_game_running),
            extra_wakes,
        ));
        self.gi_watcher = Some(watcher);
        self.discord_rpc = Some(discord_rpc);

        let preset_watcher = PresetWatcher::spawn(
            self.cache.clone(),
            &self.s,
            ctx.clone(),
            Arc::clone(&self.window_minimized),
        );
        let (io_worker, shared_watcher) = IoWorker::spawn(
            self.cache.clone(),
            ctx.clone(),
            preset_watcher,
            Arc::clone(&self.window_minimized),
        );
        io_worker.send_disk(DiskCmd::StartupHydrate {
            initial: Box::new(self.s.clone()),
        });
        self.io_worker = Some(io_worker);
        self.preset_watcher = Some(shared_watcher);

        let (update_tx, update_rx) = crossbeam_channel::bounded::<(String, String)>(1);
        let update_ctx = ctx.clone();
        let update_min = Arc::clone(&self.window_minimized);
        if std::thread::Builder::new()
            .name("shaurmeaw-updater".to_owned())
            .spawn(move || {
                if let Some(info) = crate::updater::check_update() {
                    if update_tx.send(info).is_ok() && !update_min.load(Ordering::Relaxed) {
                        update_ctx.request_repaint();
                    }
                }
            })
            .is_ok()
        {
            self.update_rx = Some(update_rx);
        }
    }

    fn refresh_cached_previews(&mut self) -> bool {
        let next_launch = config_gen::build_launch_args(&self.s).join(" ");
        let next_ae = config_gen::build_autoexec(&self.s);
        let changed = next_launch != self.cached_launch_line || next_ae != self.cached_ae_preview;
        self.cached_launch_line = next_launch;
        self.cached_ae_preview = next_ae;
        changed
    }

    fn palette(&self) -> Palette {
        Palette::from_theme(self.s.theme)
    }

    fn sfx(&self, kind: UiSound) {
        sfx::play(kind, self.s.ui_sound_enabled, self.s.ui_sound_volume);
    }

    fn set_status(&mut self, ok: bool, msg: impl Into<String>) {
        self.easter_prev_status = None;
        self.status_ok = ok;
        self.status = msg.into();
        self.toast_until = Some(Instant::now() + Duration::from_secs(if ok { 5 } else { 9 }));
    }

    fn is_steam_running(&self) -> bool {
        self.monitor
            .as_ref()
            .is_some_and(ProcessMonitor::is_steam_running)
    }

    fn is_game_running(&self) -> bool {
        self.monitor
            .as_ref()
            .is_some_and(ProcessMonitor::is_game_running)
    }

    fn is_priority_boosted(&self) -> bool {
        self.monitor
            .as_ref()
            .is_some_and(ProcessMonitor::is_priority_boosted)
    }

    fn is_affinity_optimized(&self) -> bool {
        self.monitor
            .as_ref()
            .is_some_and(ProcessMonitor::is_affinity_optimized)
    }

    fn game_pid(&self) -> Option<u32> {
        self.monitor.as_ref().and_then(ProcessMonitor::game_pid)
    }

    fn poll_updater_events(&mut self) {
        if let Some(ref rx) = self.update_rx {
            match rx.try_recv() {
                Ok(info) => {
                    self.update_available = Some(info);
                    self.update_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.update_rx = None;
                }
            }
        }
        if let Some(ref rx) = self.update_err_rx {
            match rx.try_recv() {
                Ok(err) => {
                    self.update_applying = false;
                    self.update_err_rx = None;
                    self.sfx(UiSound::Error);
                    self.set_status(false, err);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.update_err_rx = None;
                    self.update_applying = false;
                }
            }
        }
    }

    fn start_self_update(&mut self, ctx: &egui::Context, version: String, url: String) {
        if self.update_applying {
            return;
        }
        self.update_applying = true;
        self.set_status(true, format!("Скачивание обновления v{version}…"));
        let (err_tx, err_rx) = crossbeam_channel::bounded::<String>(1);
        let ui_ctx = ctx.clone();
        let min_flag = Arc::clone(&self.window_minimized);
        if std::thread::Builder::new()
            .name("shaurmeaw-apply-update".to_owned())
            .spawn(move || {
                if let Err(err) = crate::updater::apply_update(&url) {
                    if err_tx.send(err).is_ok() && !min_flag.load(Ordering::Relaxed) {
                        ui_ctx.request_repaint();
                    }
                }
            })
            .is_ok()
        {
            self.update_err_rx = Some(err_rx);
        } else {
            self.update_applying = false;
            self.sfx(UiSound::Error);
            self.set_status(false, "Не удалось запустить поток обновления");
        }
    }

    fn send_disk(&self, cmd: DiskCmd) {
        if let Some(ref w) = self.io_worker {
            w.send_disk(cmd);
        }
    }

    fn send_heavy(&mut self, label: &str, cmd: HeavyCmd) -> bool {
        if self.heavy_busy {
            self.set_status(
                false,
                format!(
                    "Дождитесь завершения операции: {}",
                    self.heavy_busy_label.as_deref().unwrap_or("фоновая задача")
                ),
            );
            return false;
        }
        if self.io_worker.is_none() {
            return false;
        }
        self.heavy_busy = true;
        self.heavy_busy_label = Some(label.to_owned());
        self.set_status(true, format!("{label}…"));
        if let Some(ref w) = self.io_worker {
            w.send_heavy(cmd);
        }
        true
    }

    fn is_bg_busy(&self) -> bool {
        self.heavy_busy
    }

    fn switch_tab(&mut self, ctx: &egui::Context, next: Tab) {
        if self.tab != next {
            self.sfx(UiSound::Click);
            self.tab = next;
            self.tab_switched_at = ctx.input(|i| i.time);
            ctx.request_repaint();
        }
        self.sync_from_cache_if_changed();
        if next == Tab::Utilities {
            if self.cache_stats.is_none() {
                self.refresh_cache_stats();
            }
            if self.cached_win_tweaks.is_none() {
                self.refresh_windows_tweaks();
            }
            self.last_telemetry_at = None;
        }
    }

    fn sync_from_cache_if_changed(&mut self) {
        let cur_rev = self.cache.revision();
        if cur_rev == self.applied_cache_rev {
            return;
        }
        let r = self.cache.read();
        self.gi_presets.clone_from(&r.gi_presets);
        self.gi_backups.clone_from(&r.gi_backups);
        self.ae_presets.clone_from(&r.ae_presets);
        self.vid_presets.clone_from(&r.vid_presets);
        self.profiles.clone_from(&r.profiles);
        self.addon_mods.clone_from(&r.addon_mods);
        self.addon_hashes.clone_from(&r.addon_hashes);
        self.dmm_conflicts = r.dmm_conflicts;
        self.vid_targets.clone_from(&r.vid_targets);
        self.gi_status = r.gi_status.clone();
        self.has_vanilla_bak = r.has_vanilla_bak;
        self.has_autoexec_bak = r.has_autoexec_bak;
        self.paths_steam_ok = r.paths_steam_ok;
        self.paths_game_ok = r.paths_game_ok;
        self.is_portable = r.is_portable;
        self.cached_app_dir.clone_from(&r.cached_app_dir);
        self.cached_settings_file.clone_from(&r.cached_settings_file);
        self.autostart_sync = r.autostart_sync;
        self.has_shortcut = r.has_shortcut;
        self.cached_console_log_bytes = r.console_log_bytes;
        self.cached_win_tweaks = r.win_tweaks;
        self.cache_stats = r.cache_stats.clone();

        if r.hydrated {
            if !self.gi_editor_dirty {
                if let Some(txt) = r.gi_texts.get(&self.selected_gi_preset) {
                    if txt != &self.gi_editor_text {
                        self.gi_editor_text.clone_from(txt);
                        self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                    }
                }
            }
            if !self.ae_editor_dirty && self.selected_ae_preset != MODE_BUILDER {
                if let Some(txt) = r.ae_texts.get(&self.selected_ae_preset) {
                    if txt != &self.ae_editor_text {
                        self.ae_editor_text.clone_from(txt);
                        self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                    }
                }
            }
            if !self.vid_editor_dirty {
                if let Some(txt) = r.vid_texts.get(&self.selected_vid_preset) {
                    if txt != &self.vid_editor_text {
                        self.vid_editor_text.clone_from(txt);
                        self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                    }
                }
            }
        }
        drop(r);
        self.applied_cache_rev = cur_rev;
    }

    fn poll_io_events(&mut self) {
        let mut events = Vec::new();
        if let Some(ref w) = self.io_worker {
            while let Some(ev) = w.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            self.handle_io_event(ev);
        }
    }

    fn handle_io_event(&mut self, ev: IoEvent) {
        match ev {
            IoEvent::StartupHydrated {
                settings,
                selected_gi_preset,
                gi_text,
                selected_ae_preset,
                ae_text,
                selected_vid_preset,
                vid_text,
                status_ok,
                status_msg,
                patch_banner,
            } => {
                if self.s == self.saved {
                    self.s = *settings;
                    self.saved = self.s.clone();
                    self.refresh_cached_previews();
                    self.previews_synced = true;
                } else {
                    if !steam::is_valid_steam_dir(Path::new(&self.s.steam_path)) {
                        self.s.steam_path = settings.steam_path;
                    }
                    if !steam::is_valid_game_dir(Path::new(&self.s.game_path)) {
                        self.s.game_path = settings.game_path;
                    }
                }
                self.sync_from_cache_if_changed();
                if !self.gi_editor_dirty {
                    self.selected_gi_preset = selected_gi_preset;
                    self.gi_editor_text = gi_text;
                }
                if !self.ae_editor_dirty {
                    self.selected_ae_preset = selected_ae_preset;
                    self.ae_editor_text = ae_text;
                }
                if !self.vid_editor_dirty {
                    self.selected_vid_preset = selected_vid_preset;
                    self.vid_editor_text = vid_text;
                }
                if let Some(banner) = patch_banner {
                    self.patch_banner = Some(banner);
                }
                if self.midnight_easter_until.is_none() {
                    self.set_status(status_ok, status_msg);
                } else {
                    self.easter_prev_status = Some((status_ok, status_msg));
                }
            }
            IoEvent::SettingsSaved(res) => {
                self.saving_settings_snapshot = None;
                match res {
                    Ok(saved_s) => {
                        self.saved = *saved_s;
                        self.pending_save_since = None;
                    }
                    Err(e) => {
                        self.pending_save_since = None;
                        self.sfx(UiSound::Error);
                        self.set_status(false, format!("Не удалось сохранить настройки: {e}"));
                    }
                }
            }
            IoEvent::PresetTextLoaded { kind, name, result } => match result {
                Ok(text) => {
                    match kind {
                        PresetKind::GameInfo => {
                            if self.selected_gi_preset == name && !self.gi_editor_dirty {
                                self.gi_editor_text = text;
                                self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                            }
                        }
                        PresetKind::Autoexec => {
                            if self.selected_ae_preset == name && !self.ae_editor_dirty {
                                self.ae_editor_text = text;
                                self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                            }
                        }
                        PresetKind::Video => {
                            if self.selected_vid_preset == name && !self.vid_editor_dirty {
                                self.vid_editor_text = text;
                                self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
                            }
                        }
                    }
                }
                Err(e) => {
                    self.sfx(UiSound::Error);
                    self.set_status(false, e);
                }
            },
            IoEvent::PresetSaved {
                kind,
                name,
                and_apply,
                result,
            } => match result {
                Ok(()) => {
                    match kind {
                        PresetKind::GameInfo if self.selected_gi_preset == name => {
                            self.gi_editor_dirty = false;
                        }
                        PresetKind::Autoexec if self.selected_ae_preset == name => {
                            self.ae_editor_dirty = false;
                        }
                        PresetKind::Video if self.selected_vid_preset == name => {
                            self.vid_editor_dirty = false;
                        }
                        _ => {}
                    }
                    self.sync_from_cache_if_changed();
                    if !and_apply {
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("Изменения в «{name}» сохранены"));
                    }
                }
                Err(e) => {
                    self.sfx(UiSound::Error);
                    self.set_status(false, e);
                }
            },
            IoEvent::PresetCreatedOrImported {
                kind,
                select_name,
                status_msg,
            } => {
                self.sync_from_cache_if_changed();
                match select_name {
                    Ok(created) => {
                        match kind {
                            PresetKind::GameInfo => {
                                self.new_gi_preset_name.clear();
                                self.new_preset_modal_open = false;
                                self.select_gi_preset(&created);
                            }
                            PresetKind::Autoexec => {
                                self.new_ae_preset_name.clear();
                                self.new_cfg_modal = None;
                                self.select_ae_preset(&created);
                            }
                            PresetKind::Video => {
                                self.new_vid_preset_name.clear();
                                self.new_cfg_modal = None;
                                self.select_vid_preset(&created);
                            }
                        }
                        self.sfx(UiSound::Success);
                        self.set_status(true, status_msg);
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::GiPresetApplied { name, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(msg) => {
                        self.s.gameinfo.active_preset = name;
                        self.sfx(UiSound::Success);
                        self.set_status(true, msg);
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::GiPresetDeleted { deleted, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(()) => {
                        if self.s.gameinfo.active_preset == deleted {
                            if let Some(first) = self.gi_presets.first() {
                                self.s.gameinfo.active_preset = first.name.clone();
                            }
                        }
                        if let Some(first) = self.gi_presets.first().map(|p| p.name.clone()) {
                            self.select_gi_preset(&first);
                        }
                        self.set_status(true, format!("Пресет «{deleted}» удалён"));
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::GiPresetRenamed { old_name, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(renamed) => {
                        if self.s.gameinfo.active_preset == old_name {
                            self.s.gameinfo.active_preset = renamed.clone();
                        }
                        self.rename_modal_target = None;
                        self.select_gi_preset(&renamed);
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("Пресет переименован в «{renamed}»"));
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::GiPatchChecked { explicit, report } => {
                self.sync_from_cache_if_changed();
                match report {
                    Ok(Some(rep)) => {
                        if rep.is_patch_adaptation {
                            self.patch_banner = Some(rep.message.clone());
                        }
                        self.sfx(UiSound::Success);
                        self.set_status(true, rep.message);
                    }
                    Ok(None) if explicit => {
                        if let SyncStatus::UpToDate { active_preset, .. } = &self.gi_status {
                            self.set_status(
                                true,
                                format!("Пресет «{active_preset}» полностью актуален"),
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::AePresetApplied { name, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(p) => {
                        self.s.autoexec_preset = name;
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("autoexec.cfg записан: {}", p.display()));
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::VidPresetApplied { name, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(msg) => {
                        self.s.video_preset = name;
                        self.sfx(UiSound::Success);
                        self.set_status(true, msg);
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::ProfileLoaded { result, status_msg } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(loaded) => {
                        self.s = *loaded;
                        self.saved = self.s.clone();
                        self.pending_save_since = None;
                        self.refresh_cached_previews();
                        self.previews_synced = true;
                        if !self.gi_editor_dirty {
                            let target = self.s.gameinfo.active_preset.clone();
                            self.select_gi_preset(&target);
                        }
                        if !self.ae_editor_dirty {
                            let target = self.s.autoexec_preset.clone();
                            self.select_ae_preset(&target);
                        }
                        if !self.vid_editor_dirty {
                            let target = self.s.video_preset.clone();
                            self.select_vid_preset(&target);
                        }
                        self.sfx(UiSound::Success);
                        self.set_status(true, status_msg);
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::ResetToValveCompleted {
                gi_res,
                ae_res,
                vid_res,
            } => {
                self.sync_from_cache_if_changed();
                if let Err(e) = gi_res {
                    self.sfx(UiSound::Error);
                    self.set_status(false, e);
                    return;
                }
                self.s.gameinfo.active_preset = gameinfo_presets::DEFAULT_PRESET_VALVE.to_owned();
                if !self.gi_editor_dirty {
                    self.select_gi_preset(gameinfo_presets::DEFAULT_PRESET_VALVE);
                }
                if let Err(e) = ae_res {
                    self.sfx(UiSound::Error);
                    self.set_status(false, e);
                    return;
                }
                self.s.autoexec_preset = autoexec_presets::PRESET_VALVE.to_owned();
                if !self.ae_editor_dirty {
                    self.select_ae_preset(autoexec_presets::PRESET_VALVE);
                }
                match vid_res {
                    Ok(vmsg) => {
                        self.sfx(UiSound::Success);
                        self.set_status(
                            true,
                            format!("Файлы игры возвращены к стандарту Valve • {vmsg}"),
                        );
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::PathsDetected {
                settings,
                status_ok,
                status_msg,
            } => {
                self.s.steam_path = settings.steam_path;
                self.s.game_path = settings.game_path;
                self.sync_from_cache_if_changed();
                self.set_status(status_ok, status_msg);
            }
            IoEvent::PortableToggled { enable, result } => {
                self.sync_from_cache_if_changed();
                match result {
                    Ok(dir) => {
                        if !self.ae_editor_dirty {
                            let name = self.selected_ae_preset.clone();
                            self.select_ae_preset(&name);
                        }
                        if !self.vid_editor_dirty {
                            let name = self.selected_vid_preset.clone();
                            self.select_vid_preset(&name);
                        }
                        if !self.gi_editor_dirty {
                            let name = self.selected_gi_preset.clone();
                            self.select_gi_preset(&name);
                        }
                        self.saved = self.s.clone();
                        self.pending_save_since = None;
                        let msg = if enable {
                            format!("Портативный режим включён: {}", dir.display())
                        } else {
                            format!("Данные перенесены в %APPDATA%: {}", dir.display())
                        };
                        self.sfx(UiSound::Success);
                        self.set_status(true, msg);
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::PlayCompleted {
                result,
                console_log_bytes,
                updated_settings,
            } => {
                self.sync_from_cache_if_changed();
                self.cached_console_log_bytes = console_log_bytes;
                self.saved = *updated_settings;
                self.pending_save_since = None;
                match result {
                    Ok(msg) => {
                        self.set_status(true, msg);
                        if let Some(ref m) = self.monitor {
                            m.wake_now();
                        }
                        if self.s.ram_trim_on_minimize {
                            self.cached_ram_bytes = game_booster::current_working_set_bytes();
                        }
                    }
                    Err(e) => {
                        self.sfx(UiSound::Error);
                        self.set_status(false, e);
                    }
                }
            }
            IoEvent::StatusOnly { ok, msg } => {
                self.sync_from_cache_if_changed();
                if !ok {
                    self.sfx(UiSound::Error);
                }
                self.set_status(ok, msg);
            }
            IoEvent::CacheScanned => {
                self.cache_scanning = false;
                self.sync_from_cache_if_changed();
            }
            IoEvent::HeavyFinished {
                ok,
                msg,
                select_gi,
                gi_text,
                clear_url_inputs,
                refresh_valve_editor,
            } => {
                self.heavy_busy = false;
                self.heavy_busy_label = None;
                self.sync_from_cache_if_changed();
                if clear_url_inputs && ok {
                    self.url_gi_input.clear();
                    self.url_gi_name.clear();
                    self.new_preset_modal_open = false;
                }
                if let Some(name) = select_gi {
                    self.selected_gi_preset = name;
                    if let Some(txt) = gi_text {
                        self.gi_editor_text = txt;
                        self.gi_editor_dirty = false;
                    }
                } else if refresh_valve_editor
                    && self.selected_gi_preset == gameinfo_presets::DEFAULT_PRESET_VALVE
                    && !self.gi_editor_dirty
                {
                    if let Some(txt) = gi_text {
                        self.gi_editor_text = txt;
                    }
                }
                if ok {
                    self.sfx(UiSound::Success);
                } else {
                    self.sfx(UiSound::Error);
                }
                self.set_status(ok, msg);
            }
            IoEvent::ManualOptimized {
                before_ram,
                console_log_bytes,
                parts,
            } => {
                self.heavy_busy = false;
                self.heavy_busy_label = None;
                self.sync_from_cache_if_changed();
                game_booster::trim_memory();
                let after_ram = game_booster::current_working_set_bytes();
                self.cached_ram_bytes = after_ram;
                self.cached_console_log_bytes = console_log_bytes;
                self.last_telemetry_at = Some(Instant::now());

                let ram_note = format!(
                    "Память: {} → {}",
                    game_booster::format_ram_mb(before_ram),
                    game_booster::format_ram_mb(after_ram)
                );
                self.sfx(UiSound::Success);
                if parts.is_empty() {
                    self.set_status(true, format!("Оптимизация завершена ({ram_note})"));
                } else {
                    self.set_status(true, format!("{ram_note} • {}", parts.join(" • ")));
                }
            }
            IoEvent::AddonsHashed { ok, msg } => {
                self.hashing_addons = false;
                self.sync_from_cache_if_changed();
                if ok {
                    self.sfx(UiSound::Success);
                } else {
                    self.sfx(UiSound::Error);
                }
                self.set_status(ok, msg);
            }
        }
    }

    fn refresh_windows_tweaks(&self) {
        self.send_disk(DiskCmd::InspectWindowsTweaks {
            game_path: PathBuf::from(&self.s.game_path),
        });
    }

    fn refresh_cache_stats(&mut self) {
        if self.cache_scanning {
            return;
        }
        if let Some(ref w) = self.io_worker {
            self.cache_scanning = true;
            w.send_heavy(HeavyCmd::ScanCache {
                steam_path: PathBuf::from(&self.s.steam_path),
                game_path: PathBuf::from(&self.s.game_path),
            });
        }
    }

    fn refresh_addons(&self) {
        self.send_disk(DiskCmd::RefreshAddons {
            game_path: PathBuf::from(&self.s.game_path),
        });
    }

    fn refresh_vid_targets(&self) {
        self.send_disk(DiskCmd::RefreshVidTargets {
            steam_path: PathBuf::from(&self.s.steam_path),
            game_path: PathBuf::from(&self.s.game_path),
            apply_all: self.s.video_apply_all_accounts,
        });
    }

    fn refresh_paths_state(&self) {
        self.send_disk(DiskCmd::RefreshPathsState {
            steam_path: PathBuf::from(&self.s.steam_path),
            game_path: PathBuf::from(&self.s.game_path),
            apply_all: self.s.video_apply_all_accounts,
        });
    }

    fn refresh_gi_state(&self) {
        self.send_disk(DiskCmd::RefreshGiState {
            steam_path: PathBuf::from(&self.s.steam_path),
            game_path: PathBuf::from(&self.s.game_path),
        });
    }

    fn select_gi_preset(&mut self, name: &str) {
        self.selected_gi_preset = name.to_owned();
        self.gi_editor_dirty = false;
        let cached = self.cache.read().gi_texts.get(name).cloned();
        if let Some(txt) = cached {
            self.gi_editor_text = txt;
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
        } else {
            self.gi_editor_text.clear();
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
            self.send_disk(DiskCmd::ReadPresetText {
                kind: PresetKind::GameInfo,
                name: name.to_owned(),
            });
        }
    }

    fn select_ae_preset(&mut self, name: &str) {
        self.selected_ae_preset = name.to_owned();
        self.ae_editor_dirty = false;
        if name == MODE_BUILDER {
            self.ae_editor_text = config_gen::build_autoexec(&self.s);
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
            return;
        }
        let cached = self.cache.read().ae_texts.get(name).cloned();
        if let Some(txt) = cached {
            self.ae_editor_text = txt;
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
        } else {
            self.ae_editor_text.clear();
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
            self.send_disk(DiskCmd::ReadPresetText {
                kind: PresetKind::Autoexec,
                name: name.to_owned(),
            });
        }
    }

    fn apply_ae_preset(&mut self, name: &str) {
        self.s.autoexec_preset = name.to_owned();
        let mut s = self.s.clone();
        s.autoexec_preset = name.to_owned();
        self.send_disk(DiskCmd::ApplyAePreset {
            settings: Box::new(s),
            name: name.to_owned(),
        });
    }

    fn select_vid_preset(&mut self, name: &str) {
        self.selected_vid_preset = name.to_owned();
        self.vid_editor_dirty = false;
        let cached = self.cache.read().vid_texts.get(name).cloned();
        if let Some(txt) = cached {
            self.vid_editor_text = txt;
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
        } else {
            self.vid_editor_text.clear();
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
            self.send_disk(DiskCmd::ReadPresetText {
                kind: PresetKind::Video,
                name: name.to_owned(),
            });
        }
    }

    fn apply_vid_preset(&mut self, name: &str) {
        self.s.video_preset = name.to_owned();
        self.send_disk(DiskCmd::ApplyVidPreset {
            settings: Box::new(self.s.clone()),
            name: name.to_owned(),
        });
    }

    fn apply_gi_preset(&mut self, name: &str) {
        self.s.gameinfo.active_preset = name.to_owned();
        let mut s = self.s.clone();
        s.gameinfo.active_preset = name.to_owned();
        self.send_disk(DiskCmd::ApplyGiPreset {
            settings: Box::new(s),
            name: name.to_owned(),
        });
    }

    fn preferred_editor_preset(&self, kind: ConfigKind) -> String {
        match kind {
            ConfigKind::GameInfo => {
                if self.gi_editor_dirty && !self.selected_gi_preset.is_empty() {
                    self.selected_gi_preset.clone()
                } else if !self.s.gameinfo.active_preset.is_empty() {
                    self.s.gameinfo.active_preset.clone()
                } else if !self.selected_gi_preset.is_empty() {
                    self.selected_gi_preset.clone()
                } else {
                    self.gi_presets
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_default()
                }
            }
            ConfigKind::Autoexec => {
                if self.ae_editor_dirty
                    && !self.selected_ae_preset.is_empty()
                    && self.selected_ae_preset != MODE_BUILDER
                {
                    self.selected_ae_preset.clone()
                } else if !self.s.autoexec_preset.is_empty()
                    && self.s.autoexec_preset != MODE_BUILDER
                {
                    self.s.autoexec_preset.clone()
                } else if !self.selected_ae_preset.is_empty()
                    && self.selected_ae_preset != MODE_BUILDER
                {
                    self.selected_ae_preset.clone()
                } else {
                    self.ae_presets
                        .iter()
                        .find(|p| p.name != MODE_BUILDER)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| MODE_BUILDER.to_owned())
                }
            }
            ConfigKind::Video => {
                if self.vid_editor_dirty && !self.selected_vid_preset.is_empty() {
                    self.selected_vid_preset.clone()
                } else if !self.s.video_preset.is_empty() {
                    self.s.video_preset.clone()
                } else if !self.selected_vid_preset.is_empty() {
                    self.selected_vid_preset.clone()
                } else {
                    self.vid_presets
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_default()
                }
            }
        }
    }

    fn open_code_editor(&mut self, kind: ConfigKind, preset_name: &str) {
        let name = if preset_name.trim().is_empty() {
            self.preferred_editor_preset(kind)
        } else {
            preset_name.to_owned()
        };

        if !name.is_empty() {
            match kind {
                ConfigKind::GameInfo => {
                    if self.selected_gi_preset != name
                        || (!self.gi_editor_dirty && self.gi_editor_text.is_empty())
                    {
                        self.select_gi_preset(&name);
                    }
                }
                ConfigKind::Autoexec => {
                    if self.selected_ae_preset != name
                        || (!self.ae_editor_dirty && self.ae_editor_text.is_empty())
                    {
                        self.select_ae_preset(&name);
                    }
                }
                ConfigKind::Video => {
                    if self.selected_vid_preset != name
                        || (!self.vid_editor_dirty && self.vid_editor_text.is_empty())
                    {
                        self.select_vid_preset(&name);
                    }
                }
            }
        }

        self.editor_ui.catalog_category.clear();
        self.editor_ui.scroll_to_line = None;
        self.editor_ui.select_char_range = None;
        self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
        self.code_modal = Some(kind);
    }

    fn check_gi_patch_now(&mut self) {
        self.set_status(true, "Проверка обновлений gameinfo.gi…");
        self.send_disk(DiskCmd::CheckGiPatch {
            settings: Box::new(self.s.clone()),
            explicit: true,
        });
    }

    fn auto_detect(&mut self, force: bool) {
        self.set_status(true, "Поиск Steam и Deadlock…");
        self.send_disk(DiskCmd::AutoDetectPaths {
            settings: Box::new(self.s.clone()),
            force,
        });
    }

    fn write_autoexec(&mut self) {
        self.send_disk(DiskCmd::WriteAutoexec {
            settings: Box::new(self.s.clone()),
        });
    }

    fn launch_game(&mut self, sandbox: bool) {
        self.sfx(UiSound::Launch);
        self.pending_save_since = None;
        self.refresh_cached_previews();
        self.previews_synced = true;
        self.set_status(
            true,
            if sandbox {
                "Запуск Deadlock на полигоне (Sandbox)…"
            } else {
                "Подготовка конфигов и запуск Deadlock…"
            },
        );
        self.send_disk(DiskCmd::PlayLaunch {
            settings: Box::new(self.s.clone()),
            game_running: self.is_game_running(),
            sandbox,
        });
    }

    fn play(&mut self) {
        self.launch_game(false);
    }

    fn play_sandbox(&mut self) {
        self.launch_game(true);
    }

    fn run_manual_optimization(&mut self) {
        let before_ram = game_booster::current_working_set_bytes();
        let game_up = self.is_game_running();
        let steam_path = PathBuf::from(&self.s.steam_path);
        let game_path = PathBuf::from(&self.s.game_path);

        self.send_heavy(
            "Оптимизация памяти и логов",
            HeavyCmd::ManualOptimize {
                steam_path,
                game_path,
                game_running: game_up,
                before_ram,
            },
        );
    }

    fn refresh_live_telemetry(&mut self, ctx: &egui::Context) {
        if self.window_minimized.load(Ordering::Relaxed) {
            return;
        }
        let now_inst = Instant::now();
        let need_refresh = self
            .last_telemetry_at
            .is_none_or(|prev| now_inst.duration_since(prev) >= Duration::from_secs(5));
        if need_refresh {
            self.cached_ram_bytes = game_booster::current_working_set_bytes();
            self.last_telemetry_at = Some(now_inst);
        }

        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.tab == Tab::Utilities && focused {
            ctx.request_repaint_after(Duration::from_secs(3));
        }
    }

    fn open(&self, path: PathBuf) {
        self.send_disk(DiskCmd::OpenDir {
            path,
            ensure_exists: false,
        });
    }

    fn open_dir(&self, path: PathBuf, ensure_exists: bool) {
        self.send_disk(DiskCmd::OpenDir {
            path,
            ensure_exists,
        });
    }

    fn apply_portable_toggle(&self, enable: bool) {
        self.send_disk(DiskCmd::TogglePortableMode {
            enable,
            settings: Box::new(self.s.clone()),
        });
    }

    // ============================================================ Вкладка 1: Главная

    fn ui_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.palette();
        let game = PathBuf::from(&self.s.game_path);
        let game_up = self.is_game_running();
        let active_name = self.s.gameinfo.active_preset.clone();
        let bg_busy = self.is_bg_busy();

        // ---- 1. Верхняя карточка запуска (Hero Card)
        theme::card_frame(ui, &pal, true, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 252.0).max(180.0));
                    ui.label(theme::serif_heading(
                        "АКТИВНЫЙ ПРОФИЛЬ",
                        10.5,
                        theme::TEXT_MUTED,
                    ));
                    ui.add_space(3.0);
                    let hero_name = if active_name.trim().is_empty() {
                        "Не выбран"
                    } else {
                        active_name.as_str()
                    };
                    ui.add(
                        egui::Label::new(theme::serif_heading(hero_name, 19.0, theme::TEXT_CREAM))
                            .truncate(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (dot_col, state_txt) = if !self.paths_game_ok {
                            (theme::CRIMSON, "Укажите папку игры в Настройках")
                        } else if game_up {
                            (theme::SOUL_TEAL, "Deadlock сейчас запущен")
                        } else {
                            (theme::SOUL_TEAL, "Готов к запуску")
                        };
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 3.2, dot_col);
                        ui.label(RichText::new(state_txt).size(12.5).color(dot_col));
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::brass_primary_button(
                        ui,
                        Some(Icon::Play),
                        "ИГРАТЬ В DEADLOCK",
                        egui::vec2(236.0, 48.0),
                        true,
                        &pal,
                    )
                    .on_hover_text("Запустить Deadlock с выбранным профилем конфигурации")
                    .clicked()
                    {
                        self.play();
                    }
                });
            });
        });

        ui.add_space(14.0);

        // ---- 2. Компактный заголовок сетки пресетов
        ui.horizontal(|ui| {
            ui.label(theme::serif_heading("ПРОФИЛИ", 13.5, pal.accent_light));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Settings),
                    "Твики и бэкапы",
                    egui::vec2(0.0, 28.0),
                    true,
                    &pal,
                )
                .on_hover_text("Угол обзора (FOV), визуальные настройки и архив резервных копий")
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    self.gi_tweaks_modal_open = true;
                }
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Edit),
                    "Редактор кода",
                    egui::vec2(0.0, 28.0),
                    true,
                    &pal,
                )
                .on_hover_text("Удобный редактор gameinfo.gi с поиском, командами и справочником")
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    let target = self.preferred_editor_preset(ConfigKind::GameInfo);
                    self.open_code_editor(ConfigKind::GameInfo, &target);
                }
            });
        });
        ui.add_space(10.0);

        // ---- 3. Сетка карточек пресетов (2 колонки, максимум 3 элемента + кнопка «...»)
        let presets = self.gi_presets.clone();
        let presets_count = presets.len();
        let avail_w = ui.available_width();
        let gap = 14.0_f32;
        let cols: usize = if avail_w >= 560.0 { 2 } else { 1 };
        let col_w = ((avail_w - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(220.0);

        enum CardCmd {
            Apply(String),
            EditCode(String),
            Rename(String),
            Share(String),
            RefreshUrl(String),
            ExportFile(String),
            Delete(String),
            OpenNewModal,
        }

        let mut card_cmd: Option<CardCmd> = None;
        let total_items = presets_count + 1;

        for row_start in (0..total_items).step_by(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for offset in 0..cols {
                    let idx = row_start + offset;
                    if idx >= total_items {
                        break;
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, 122.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(col_w);
                            ui.set_max_width(col_w);

                            if let Some(entry) = presets.get(idx) {
                                let is_active = entry.name == active_name;
                                theme::card_frame(ui, &pal, is_active, |ui| {
                                    // Элемент 1: Название пресета + меню «...» в углу
                                    ui.horizontal(|ui| {
                                        let title_w = (ui.available_width() - 34.0).max(100.0);
                                        ui.allocate_ui(egui::vec2(title_w, 24.0), |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&entry.name)
                                                        .size(15.0)
                                                        .strong()
                                                        .color(theme::TEXT_CREAM),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(format!(
                                                "Изменён: {}\nРазмер: {}",
                                                entry.modified_label,
                                                tools::format_size(entry.size_bytes)
                                            ));
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let dots_resp = theme::dots_menu_button(ui, &pal)
                                                    .on_hover_text("Действия с профилем");
                                                let popup_id = ui
                                                    .make_persistent_id(("gi_menu", &entry.name));
                                                if dots_resp.clicked() {
                                                    self.sfx(UiSound::Click);
                                                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                                                }
                                                egui::popup_below_widget(
                                                    ui,
                                                    popup_id,
                                                    &dots_resp,
                                                    egui::PopupCloseBehavior::CloseOnClick,
                                                    |ui| {
                                                        ui.set_min_width(178.0);
                                                        if ui.button("Редактировать код").clicked()
                                                        {
                                                            card_cmd = Some(CardCmd::EditCode(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if ui.button("Переименовать").clicked() {
                                                            card_cmd = Some(CardCmd::Rename(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if ui
                                                            .button("Поделиться ссылкой")
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(CardCmd::Share(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if entry.source_url.is_some()
                                                            && ui
                                                                .add_enabled(
                                                                    !bg_busy,
                                                                    egui::Button::new(
                                                                        "Обновить из сети",
                                                                    ),
                                                                )
                                                                .clicked()
                                                        {
                                                            card_cmd = Some(CardCmd::RefreshUrl(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if ui
                                                            .button("Сохранить копию (.gi)…")
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(CardCmd::ExportFile(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        ui.separator();
                                                        let can_del = presets_count > 1;
                                                        let del_text = RichText::new("Удалить")
                                                            .color(if can_del {
                                                                theme::CRIMSON
                                                            } else {
                                                                theme::TEXT_MUTED
                                                            });
                                                        if ui
                                                            .add_enabled(
                                                                can_del,
                                                                egui::Button::new(del_text),
                                                            )
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(CardCmd::Delete(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                    },
                                                );
                                            },
                                        );
                                    });

                                    ui.add_space(6.0);

                                    // Элемент 2: Короткий тег статуса
                                    if is_active {
                                        theme::badge_pill(ui, "Активен", theme::SOUL_TEAL, true);
                                    } else if entry.source_url.is_some() {
                                        theme::badge_pill(ui, "Обновлён", pal.accent, false);
                                    } else {
                                        theme::badge_pill(ui, "Готов", theme::TEXT_MUTED, false);
                                    }

                                    ui.add_space(10.0);

                                    // Элемент 3: Одна кнопка действия
                                    let btn_lbl = if is_active {
                                        "Применить снова"
                                    } else {
                                        "Выбрать"
                                    };
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        btn_lbl,
                                        egui::vec2(ui.available_width(), 30.0),
                                        true,
                                        &pal,
                                    )
                                    .clicked()
                                    {
                                        card_cmd = Some(CardCmd::Apply(entry.name.clone()));
                                    }
                                });
                            } else {
                                // Карточка «+ Новый пресет» с пунктирной латунной рамкой
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(col_w, 122.0),
                                    egui::Sense::click(),
                                );
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                let hover_t = ui.ctx().animate_bool_with_time_and_easing(
                                    resp.id.with("new_card_hover"),
                                    resp.hovered(),
                                    ui.style().animation_time,
                                    egui::emath::easing::quadratic_out,
                                );
                                if ui.is_rect_visible(rect) {
                                    let fill = theme::BG_MANUSCRIPT
                                        .lerp_to_gamma(theme::SURFACE_CARD, hover_t);
                                    ui.painter().rect_filled(rect, 4.0, fill);
                                    let stroke_col =
                                        pal.accent_border.lerp_to_gamma(pal.accent_light, hover_t);
                                    theme::paint_dashed_rect(
                                        ui.painter(),
                                        rect.shrink(1.0),
                                        egui::Stroke::new(1.3_f32, stroke_col),
                                        6.0,
                                        5.0,
                                    );
                                    let label_col =
                                        theme::TEXT_MUTED.lerp_to_gamma(pal.accent_light, hover_t);
                                    let ic_r = egui::Rect::from_center_size(
                                        egui::pos2(rect.center().x, rect.center().y - 12.0),
                                        egui::vec2(18.0, 18.0),
                                    );
                                    theme::paint_icon(ui.painter(), ic_r, Icon::Plus, label_col, 1.6);
                                    let galley = ui.painter().layout_no_wrap(
                                        "Новый пресет".to_owned(),
                                        egui::FontId::proportional(14.0),
                                        label_col,
                                    );
                                    ui.painter().galley(
                                        egui::pos2(
                                            rect.center().x - galley.size().x * 0.5,
                                            rect.center().y + 6.0,
                                        ),
                                        galley,
                                        label_col,
                                    );
                                }
                                if resp
                                    .on_hover_text("Создать, импортировать или загрузить пресет по ссылке")
                                    .clicked()
                                {
                                    card_cmd = Some(CardCmd::OpenNewModal);
                                }
                            }
                        },
                    );
                }
            });
            ui.add_space(gap);
        }

        match card_cmd {
            Some(CardCmd::Apply(name)) => {
                self.sfx(UiSound::Click);
                self.select_gi_preset(&name);
                self.apply_gi_preset(&name);
            }
            Some(CardCmd::EditCode(name)) => {
                self.sfx(UiSound::Click);
                self.open_code_editor(ConfigKind::GameInfo, &name);
            }
            Some(CardCmd::Rename(name)) => {
                self.sfx(UiSound::Click);
                self.select_gi_preset(&name);
                self.rename_gi_name = name.clone();
                self.rename_modal_target = Some(name);
            }
            Some(CardCmd::Share(name)) => {
                let url_opt = self
                    .gi_presets
                    .iter()
                    .find(|p| p.name == name)
                    .and_then(|p| p.source_url.clone());
                if let Some(url) = url_opt {
                    ctx.copy_text(url);
                    self.sfx(UiSound::Success);
                    self.set_status(true, format!("Ссылка на пресет «{name}» скопирована"));
                } else {
                    let text = self
                        .cache
                        .read()
                        .gi_texts
                        .get(&name)
                        .cloned()
                        .or_else(|| gameinfo_presets::read_preset(&name).ok());
                    if let Some(body) = text {
                        ctx.copy_text(body);
                        self.sfx(UiSound::Success);
                        self.set_status(
                            true,
                            format!("Текст пресета «{name}» скопирован для отправки"),
                        );
                    } else {
                        self.sfx(UiSound::Error);
                        self.set_status(false, "Не удалось прочитать содержимое пресета");
                    }
                }
            }
            Some(CardCmd::RefreshUrl(name)) => {
                self.sfx(UiSound::Click);
                self.send_heavy(
                    &format!("Обновление «{name}»"),
                    HeavyCmd::RefreshGiUrl {
                        name,
                        steam_path: PathBuf::from(&self.s.steam_path),
                        game_path: game.clone(),
                    },
                );
            }
            Some(CardCmd::ExportFile(name)) => {
                self.sfx(UiSound::Click);
                let text = self
                    .cache
                    .read()
                    .gi_texts
                    .get(&name)
                    .cloned()
                    .or_else(|| gameinfo_presets::read_preset(&name).ok())
                    .unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("GameInfo", &["gi", "txt"])
                    .set_file_name(format!("{name}.gi"))
                    .save_file()
                {
                    if let Err(e) =
                        crate::fs_safe::atomic_write_with_retry(&path, text.as_bytes())
                    {
                        self.sfx(UiSound::Error);
                        self.set_status(false, format!("Не удалось сохранить файл: {e}"));
                    } else {
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("Копия сохранена: {}", path.display()));
                    }
                }
            }
            Some(CardCmd::Delete(name)) => {
                self.sfx(UiSound::Click);
                self.send_disk(DiskCmd::DeleteGiPreset {
                    name,
                    steam_path: PathBuf::from(&self.s.steam_path),
                    game_path: game,
                });
            }
            Some(CardCmd::OpenNewModal) => {
                self.sfx(UiSound::Click);
                self.new_preset_modal_open = true;
            }
            None => {}
        }
    }

    // ============================================================ Вкладка 2: Конфиги (autoexec.cfg и video.txt)

    fn read_ae_preset_text(&self, name: &str) -> Option<String> {
        if name == MODE_BUILDER {
            Some(config_gen::build_autoexec(&self.s))
        } else {
            self.cache
                .read()
                .ae_texts
                .get(name)
                .cloned()
                .or_else(|| autoexec_presets::read_preset(name).ok())
        }
    }

    fn read_vid_preset_text(&self, name: &str) -> Option<String> {
        self.cache
            .read()
            .vid_texts
            .get(name)
            .cloned()
            .or_else(|| video_presets::read_preset(name).ok())
    }

    fn ui_new_dashed_card(
        ui: &mut egui::Ui,
        col_w: f32,
        id_salt: &str,
        label: &str,
        tooltip: &str,
        pal: &Palette,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(col_w, 122.0), egui::Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let hover_t = ui.ctx().animate_bool_with_time_and_easing(
            resp.id.with(("new_cfg_hover", id_salt)),
            resp.hovered(),
            ui.style().animation_time,
            egui::emath::easing::quadratic_out,
        );
        if ui.is_rect_visible(rect) {
            let fill = theme::BG_MANUSCRIPT.lerp_to_gamma(theme::SURFACE_CARD, hover_t);
            ui.painter().rect_filled(rect, 4.0, fill);
            let stroke_col = pal.accent_border.lerp_to_gamma(pal.accent_light, hover_t);
            theme::paint_dashed_rect(
                ui.painter(),
                rect.shrink(1.0),
                egui::Stroke::new(1.3_f32, stroke_col),
                6.0,
                5.0,
            );
            let label_col = theme::TEXT_MUTED.lerp_to_gamma(pal.accent_light, hover_t);
            let ic_r = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.center().y - 12.0),
                egui::vec2(18.0, 18.0),
            );
            theme::paint_icon(ui.painter(), ic_r, Icon::Plus, label_col, 1.6);
            let galley = ui.painter().layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(14.0),
                label_col,
            );
            ui.painter().galley(
                egui::pos2(
                    rect.center().x - galley.size().x * 0.5,
                    rect.center().y + 6.0,
                ),
                galley,
                label_col,
            );
        }
        resp.on_hover_text(tooltip).clicked()
    }

    fn ui_configs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.palette();

        ui.horizontal(|ui| {
            ui.label(theme::serif_heading("КОНФИГИ", 14.5, pal.accent_light));
            ui.add_space(12.0);
            if theme::deco_chip(
                ui,
                self.config_subtab == ConfigSubTab::Autoexec,
                "autoexec.cfg",
                &pal,
            )
            .clicked()
            {
                self.sfx(UiSound::Click);
                self.config_subtab = ConfigSubTab::Autoexec;
            }
            if theme::deco_chip(
                ui,
                self.config_subtab == ConfigSubTab::Video,
                "video.txt",
                &pal,
            )
            .clicked()
            {
                self.sfx(UiSound::Click);
                self.config_subtab = ConfigSubTab::Video;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (folder_path, folder_tip) = match self.config_subtab {
                    ConfigSubTab::Autoexec => (
                        Settings::autoexec_presets_dir(),
                        "Открыть папку профилей autoexec.cfg",
                    ),
                    ConfigSubTab::Video => (
                        Settings::video_presets_dir(),
                        "Открыть папку профилей video.txt",
                    ),
                };
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Folder),
                    "Открыть папку",
                    egui::vec2(0.0, 28.0),
                    true,
                    &pal,
                )
                .on_hover_text(folder_tip)
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    self.open_dir(folder_path, true);
                }
            });
        });

        ui.add_space(12.0);

        match self.config_subtab {
            ConfigSubTab::Autoexec => self.ui_configs_autoexec(ui, ctx, &pal),
            ConfigSubTab::Video => self.ui_configs_video(ui, ctx, &pal),
        }
    }

    fn ui_configs_autoexec(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        pal: &Palette,
    ) {
        let game = PathBuf::from(&self.s.game_path);
        let active_name = self.s.autoexec_preset.clone();
        let right_reserved = if self.has_autoexec_bak { 370.0 } else { 236.0 };

        // ---- Hero-карточка active autoexec
        theme::card_frame(ui, pal, true, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - right_reserved).max(160.0));
                    ui.label(theme::serif_heading(
                        "АКТИВНЫЙ AUTOEXEC.CFG",
                        10.5,
                        theme::TEXT_MUTED,
                    ));
                    ui.add_space(3.0);
                    let hero_name = if active_name.trim().is_empty() {
                        "Не выбран"
                    } else {
                        active_name.as_str()
                    };
                    ui.add(
                        egui::Label::new(theme::serif_heading(hero_name, 18.0, theme::TEXT_CREAM))
                            .truncate(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (dot_col, state_txt) = if !self.paths_game_ok {
                            (theme::CRIMSON, "Укажите папку Deadlock в Настройках")
                        } else if active_name == MODE_BUILDER {
                            (theme::SOUL_TEAL, "Соберётся из настроек Конструктора")
                        } else {
                            (
                                theme::SOUL_TEAL,
                                "Готов к записи в game/citadel/cfg/autoexec.cfg",
                            )
                        };
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 3.2, dot_col);
                        ui.label(RichText::new(state_txt).size(12.5).color(dot_col));
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::brass_primary_button(
                        ui,
                        Some(Icon::Save),
                        "ЗАПИСАТЬ AUTOEXEC",
                        egui::vec2(215.0, 44.0),
                        true,
                        pal,
                    )
                    .on_hover_text("Записать выбранный autoexec.cfg в папку игры")
                    .clicked()
                    {
                        self.sfx(UiSound::Click);
                        self.write_autoexec();
                    }
                    if self.has_autoexec_bak
                        && theme::brass_secondary_button(
                            ui,
                            Some(Icon::Restore),
                            "Восстановить .bak",
                            egui::vec2(0.0, 44.0),
                            true,
                            pal,
                        )
                        .on_hover_text("Восстановить прежний autoexec.cfg из резервной копии")
                        .clicked()
                    {
                        self.sfx(UiSound::Click);
                        self.send_disk(DiskCmd::RestoreAutoexecBak {
                            game_path: game.clone(),
                        });
                    }
                });
            });
        });

        ui.add_space(12.0);

        // ---- Переключатели автоматизации
        theme::card_frame(ui, pal, false, |ui| {
            if theme::setting_toggle_row(
                ui,
                "Обновлять autoexec.cfg перед каждым запуском",
                "Перезаписывает конфиг активным профилем перед каждым стартом Deadlock.",
                &mut self.s.write_autoexec_on_launch,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Выполнять при старте игры (+exec autoexec)",
                "Передаёт параметр +exec autoexec, чтобы движок сразу применил команды.",
                &mut self.s.exec_autoexec,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
        });

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(theme::serif_heading(
                "ПРОФИЛИ AUTOEXEC.CFG",
                13.5,
                pal.accent_light,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Edit),
                    "Редактор кода",
                    egui::vec2(0.0, 28.0),
                    true,
                    pal,
                )
                .on_hover_text("Редактор autoexec.cfg с поиском, командами и справочником Deadlock")
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    let target = self.preferred_editor_preset(ConfigKind::Autoexec);
                    self.open_code_editor(ConfigKind::Autoexec, &target);
                }
            });
        });
        ui.add_space(10.0);

        // ---- Сетка профилей autoexec.cfg + Конструктор + кнопка "+ Новый autoexec"
        let presets = self.ae_presets.clone();
        let avail_w = ui.available_width();
        let gap = 14.0_f32;
        let cols: usize = if avail_w >= 560.0 { 2 } else { 1 };
        let col_w = ((avail_w - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(220.0);

        enum AeCardCmd {
            Apply(String),
            EditCode(String),
            CopyText(String),
            ExportFile(String),
            Delete(String),
            OpenNewModal,
        }

        let mut card_cmd: Option<AeCardCmd> = None;
        let presets_len = presets.len();
        let total_items = presets_len + 2;

        for row_start in (0..total_items).step_by(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for offset in 0..cols {
                    let idx = row_start + offset;
                    if idx >= total_items {
                        break;
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, 122.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(col_w);
                            ui.set_max_width(col_w);

                            if idx < presets_len + 1 {
                                let (name, is_builder, hover_tip) = if let Some(entry) =
                                    presets.get(idx)
                                {
                                    (
                                        entry.name.as_str(),
                                        false,
                                        format!(
                                            "Изменён: {}\nРазмер: {}",
                                            entry.modified_label,
                                            tools::format_size(entry.size_bytes)
                                        ),
                                    )
                                } else {
                                    (
                                        MODE_BUILDER,
                                        true,
                                        "Собирает autoexec.cfg из настроек графики, прицела и биндов"
                                            .to_owned(),
                                    )
                                };
                                let is_active = name == active_name;

                                theme::card_frame(ui, pal, is_active, |ui| {
                                    ui.horizontal(|ui| {
                                        let title_w = (ui.available_width() - 34.0).max(100.0);
                                        ui.allocate_ui(egui::vec2(title_w, 24.0), |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(name)
                                                        .size(15.0)
                                                        .strong()
                                                        .color(theme::TEXT_CREAM),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(hover_tip);
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let dots_resp = theme::dots_menu_button(ui, pal)
                                                    .on_hover_text("Действия с профилем");
                                                let popup_id =
                                                    ui.make_persistent_id(("ae_menu", name));
                                                if dots_resp.clicked() {
                                                    self.sfx(UiSound::Click);
                                                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                                                }
                                                egui::popup_below_widget(
                                                    ui,
                                                    popup_id,
                                                    &dots_resp,
                                                    egui::PopupCloseBehavior::CloseOnClick,
                                                    |ui| {
                                                        ui.set_min_width(178.0);
                                                        let code_lbl = if is_builder {
                                                            "Посмотреть код"
                                                        } else {
                                                            "Редактировать код"
                                                        };
                                                        if ui.button(code_lbl).clicked() {
                                                            card_cmd = Some(AeCardCmd::EditCode(
                                                                name.to_owned(),
                                                            ));
                                                        }
                                                        if ui.button("Скопировать текст").clicked() {
                                                            card_cmd = Some(AeCardCmd::CopyText(
                                                                name.to_owned(),
                                                            ));
                                                        }
                                                        if ui
                                                            .button("Сохранить копию (.cfg)…")
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(AeCardCmd::ExportFile(
                                                                name.to_owned(),
                                                            ));
                                                        }
                                                        ui.separator();
                                                        let can_del = !is_builder
                                                            && name
                                                                != autoexec_presets::PRESET_SHAURMEAW
                                                            && name
                                                                != autoexec_presets::PRESET_VALVE;
                                                        let del_text = RichText::new("Удалить")
                                                            .color(if can_del {
                                                                theme::CRIMSON
                                                            } else {
                                                                theme::TEXT_MUTED
                                                            });
                                                        if ui
                                                            .add_enabled(
                                                                can_del,
                                                                egui::Button::new(del_text),
                                                            )
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(AeCardCmd::Delete(
                                                                name.to_owned(),
                                                            ));
                                                        }
                                                    },
                                                );
                                            },
                                        );
                                    });

                                    ui.add_space(6.0);
                                    if is_active {
                                        theme::badge_pill(ui, "Активен", theme::SOUL_TEAL, true);
                                    } else if is_builder {
                                        theme::badge_pill(ui, "Конструктор", pal.accent, false);
                                    } else {
                                        theme::badge_pill(ui, "Готов", theme::TEXT_MUTED, false);
                                    }

                                    ui.add_space(10.0);
                                    let btn_lbl = if is_active {
                                        "Записать снова"
                                    } else {
                                        "Выбрать"
                                    };
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        btn_lbl,
                                        egui::vec2(ui.available_width(), 30.0),
                                        true,
                                        pal,
                                    )
                                    .clicked()
                                    {
                                        card_cmd = Some(AeCardCmd::Apply(name.to_owned()));
                                    }
                                });
                            } else if Self::ui_new_dashed_card(
                                ui,
                                col_w,
                                "ae",
                                "Новый autoexec",
                                "Сохранить текущий autoexec.cfg игры или импортировать .cfg",
                                pal,
                            ) {
                                card_cmd = Some(AeCardCmd::OpenNewModal);
                            }
                        },
                    );
                }
            });
            ui.add_space(gap);
        }

        match card_cmd {
            Some(AeCardCmd::Apply(name)) => {
                self.sfx(UiSound::Click);
                self.select_ae_preset(&name);
                self.apply_ae_preset(&name);
            }
            Some(AeCardCmd::EditCode(name)) => {
                self.sfx(UiSound::Click);
                self.open_code_editor(ConfigKind::Autoexec, &name);
            }
            Some(AeCardCmd::CopyText(name)) => {
                if let Some(body) = self.read_ae_preset_text(&name) {
                    ctx.copy_text(body);
                    self.sfx(UiSound::Success);
                    self.set_status(true, format!("Текст «{name}» скопирован в буфер обмена"));
                } else {
                    self.sfx(UiSound::Error);
                    self.set_status(false, "Не удалось прочитать содержимое профиля");
                }
            }
            Some(AeCardCmd::ExportFile(name)) => {
                self.sfx(UiSound::Click);
                let text = self.read_ae_preset_text(&name).unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Config", &["cfg", "txt"])
                    .set_file_name(format!("{name}.cfg"))
                    .save_file()
                {
                    if let Err(e) =
                        crate::fs_safe::atomic_write_with_retry(&path, text.as_bytes())
                    {
                        self.sfx(UiSound::Error);
                        self.set_status(false, format!("Не удалось сохранить файл: {e}"));
                    } else {
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("Копия сохранена: {}", path.display()));
                    }
                }
            }
            Some(AeCardCmd::Delete(name)) => {
                self.sfx(UiSound::Click);
                if self.s.autoexec_preset == name {
                    self.s.autoexec_preset = autoexec_presets::PRESET_SHAURMEAW.to_owned();
                    self.select_ae_preset(autoexec_presets::PRESET_SHAURMEAW);
                }
                self.send_disk(DiskCmd::DeleteAePreset {
                    name,
                    game_path: game,
                });
            }
            Some(AeCardCmd::OpenNewModal) => {
                self.sfx(UiSound::Click);
                self.new_cfg_modal = Some(ConfigSubTab::Autoexec);
            }
            None => {}
        }

        // ---- Сворачиваемый конструктор графики, лимита FPS, прицела и биндов
        theme::card_frame(ui, pal, false, |ui| {
            egui::CollapsingHeader::new("Конструктор графики, лимита FPS, прицела и биндов")
                .id_salt("cfg_ae_builder")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    self.ui_builder_controls(ui, pal);
                });
        });
    }

    fn ui_configs_video(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let game = PathBuf::from(&self.s.game_path);
        let steam_dir = PathBuf::from(&self.s.steam_path);
        let active_name = self.s.video_preset.clone();
        let targets_count = self.vid_targets.len();
        let targets_tip = if self.vid_targets.is_empty() {
            "Папки Steam userdata или citadel\\cfg пока не найдены".to_owned()
        } else {
            self.vid_targets
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };

        // ---- Hero-карточка active video.txt
        theme::card_frame(ui, pal, true, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 375.0).max(160.0));
                    ui.label(theme::serif_heading(
                        "АКТИВНЫЙ VIDEO.TXT",
                        10.5,
                        theme::TEXT_MUTED,
                    ));
                    ui.add_space(3.0);
                    let hero_name = if active_name.trim().is_empty() {
                        "Не выбран"
                    } else {
                        active_name.as_str()
                    };
                    ui.add(
                        egui::Label::new(theme::serif_heading(hero_name, 18.0, theme::TEXT_CREAM))
                            .truncate(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (dot_col, state_txt) = if targets_count == 0 {
                            (
                                theme::CRIMSON,
                                "Файлы video.txt не найдены — проверьте папку Steam/игры".to_owned(),
                            )
                        } else {
                            (
                                theme::SOUL_TEAL,
                                format!("Целевых файлов video.txt: {targets_count}"),
                            )
                        };
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 3.2, dot_col);
                        ui.label(RichText::new(state_txt).size(12.5).color(dot_col))
                            .on_hover_text(targets_tip);
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::brass_primary_button(
                        ui,
                        Some(Icon::Play),
                        "ПРИМЕНИТЬ VIDEO.TXT",
                        egui::vec2(215.0, 44.0),
                        !active_name.trim().is_empty(),
                        pal,
                    )
                    .on_hover_text("Записать выбранные настройки видео для Deadlock")
                    .clicked()
                    {
                        self.sfx(UiSound::Click);
                        self.select_vid_preset(&active_name);
                        self.apply_vid_preset(&active_name);
                    }
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Restore),
                        "Сбросить к Valve",
                        egui::vec2(0.0, 44.0),
                        true,
                        pal,
                    )
                    .on_hover_text("Удалит video.txt, чтобы Deadlock заново создал настройки экрана")
                    .clicked()
                    {
                        self.sfx(UiSound::Click);
                        self.send_disk(DiskCmd::ResetVidAutogen {
                            settings: Box::new(self.s.clone()),
                        });
                    }
                });
            });
        });

        ui.add_space(12.0);

        // ---- Параметры video.txt
        theme::card_frame(ui, pal, false, |ui| {
            if theme::setting_toggle_row(
                ui,
                "Применять video.txt перед каждым запуском",
                "Обновляет файл video.txt выбранным профилем при старте Deadlock.",
                &mut self.s.write_video_on_launch,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Сохранять ID вашей видеокарты (VendorID / DeviceID)",
                "Не даёт игре сбросить настройки при переносе конфига с чужого ПК.",
                &mut self.s.video_preserve_gpu_ids,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Сохранять текущее разрешение экрана и герцовку",
                "Оставляет ваше родное разрешение и частоту экрана вместо значений из пресета.",
                &mut self.s.video_preserve_resolution,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Применять для всех аккаунтов Steam на ПК",
                "Записывает video.txt сразу во все найденные профили Steam userdata.",
                &mut self.s.video_apply_all_accounts,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
                self.refresh_vid_targets();
            }
            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Защита от перезаписи игрой (Только чтение)",
                "Ставит атрибут «Только для чтения», чтобы Deadlock не перезаписывал настройки видео.",
                &mut self.s.video_set_readonly,
                pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }
        });

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(theme::serif_heading(
                "ПРОФИЛИ VIDEO.TXT",
                13.5,
                pal.accent_light,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Edit),
                    "Редактор кода",
                    egui::vec2(0.0, 28.0),
                    true,
                    pal,
                )
                .on_hover_text("Редактор video.txt с поиском, параметрами графики и справочником")
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    let target = self.preferred_editor_preset(ConfigKind::Video);
                    self.open_code_editor(ConfigKind::Video, &target);
                }
            });
        });
        ui.add_space(10.0);

        // ---- Сетка профилей video.txt + кнопка "+ Новый video.txt"
        let presets = self.vid_presets.clone();
        let avail_w = ui.available_width();
        let gap = 14.0_f32;
        let cols: usize = if avail_w >= 560.0 { 2 } else { 1 };
        let col_w = ((avail_w - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(220.0);

        enum VidCardCmd {
            Apply(String),
            EditCode(String),
            CopyText(String),
            ExportFile(String),
            Delete(String),
            OpenNewModal,
        }

        let mut card_cmd: Option<VidCardCmd> = None;
        let total_items = presets.len() + 1;

        for row_start in (0..total_items).step_by(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for offset in 0..cols {
                    let idx = row_start + offset;
                    if idx >= total_items {
                        break;
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, 122.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(col_w);
                            ui.set_max_width(col_w);

                            if let Some(entry) = presets.get(idx) {
                                let is_active = entry.name == active_name;
                                theme::card_frame(ui, pal, is_active, |ui| {
                                    ui.horizontal(|ui| {
                                        let title_w = (ui.available_width() - 34.0).max(100.0);
                                        ui.allocate_ui(egui::vec2(title_w, 24.0), |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&entry.name)
                                                        .size(15.0)
                                                        .strong()
                                                        .color(theme::TEXT_CREAM),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(format!(
                                                "Изменён: {}\nРазмер: {}",
                                                entry.modified_label,
                                                tools::format_size(entry.size_bytes)
                                            ));
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let dots_resp = theme::dots_menu_button(ui, pal)
                                                    .on_hover_text("Действия с профилем");
                                                let popup_id = ui
                                                    .make_persistent_id(("vid_menu", &entry.name));
                                                if dots_resp.clicked() {
                                                    self.sfx(UiSound::Click);
                                                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                                                }
                                                egui::popup_below_widget(
                                                    ui,
                                                    popup_id,
                                                    &dots_resp,
                                                    egui::PopupCloseBehavior::CloseOnClick,
                                                    |ui| {
                                                        ui.set_min_width(178.0);
                                                        if ui.button("Редактировать код").clicked() {
                                                            card_cmd = Some(VidCardCmd::EditCode(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if ui.button("Скопировать текст").clicked() {
                                                            card_cmd = Some(VidCardCmd::CopyText(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                        if ui
                                                            .button("Сохранить копию (.txt)…")
                                                            .clicked()
                                                        {
                                                            card_cmd =
                                                                Some(VidCardCmd::ExportFile(
                                                                    entry.name.clone(),
                                                                ));
                                                        }
                                                        ui.separator();
                                                        let can_del = entry.name
                                                            != video_presets::PRESET_SHAURMEAW
                                                            && entry.name
                                                                != video_presets::PRESET_VALVE;
                                                        let del_text = RichText::new("Удалить")
                                                            .color(if can_del {
                                                                theme::CRIMSON
                                                            } else {
                                                                theme::TEXT_MUTED
                                                            });
                                                        if ui
                                                            .add_enabled(
                                                                can_del,
                                                                egui::Button::new(del_text),
                                                            )
                                                            .clicked()
                                                        {
                                                            card_cmd = Some(VidCardCmd::Delete(
                                                                entry.name.clone(),
                                                            ));
                                                        }
                                                    },
                                                );
                                            },
                                        );
                                    });

                                    ui.add_space(6.0);
                                    if is_active {
                                        theme::badge_pill(ui, "Активен", theme::SOUL_TEAL, true);
                                    } else {
                                        theme::badge_pill(ui, "Готов", theme::TEXT_MUTED, false);
                                    }

                                    ui.add_space(10.0);
                                    let btn_lbl = if is_active {
                                        "Применить снова"
                                    } else {
                                        "Выбрать"
                                    };
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        btn_lbl,
                                        egui::vec2(ui.available_width(), 30.0),
                                        true,
                                        pal,
                                    )
                                    .clicked()
                                    {
                                        card_cmd = Some(VidCardCmd::Apply(entry.name.clone()));
                                    }
                                });
                            } else if Self::ui_new_dashed_card(
                                ui,
                                col_w,
                                "vid",
                                "Новый video.txt",
                                "Сохранить текущий video.txt игры или импортировать .txt/.cfg",
                                pal,
                            ) {
                                card_cmd = Some(VidCardCmd::OpenNewModal);
                            }
                        },
                    );
                }
            });
            ui.add_space(gap);
        }

        match card_cmd {
            Some(VidCardCmd::Apply(name)) => {
                self.sfx(UiSound::Click);
                self.select_vid_preset(&name);
                self.apply_vid_preset(&name);
            }
            Some(VidCardCmd::EditCode(name)) => {
                self.sfx(UiSound::Click);
                self.open_code_editor(ConfigKind::Video, &name);
            }
            Some(VidCardCmd::CopyText(name)) => {
                if let Some(body) = self.read_vid_preset_text(&name) {
                    ctx.copy_text(body);
                    self.sfx(UiSound::Success);
                    self.set_status(true, format!("Текст «{name}» скопирован в буфер обмена"));
                } else {
                    self.sfx(UiSound::Error);
                    self.set_status(false, "Не удалось прочитать содержимое профиля");
                }
            }
            Some(VidCardCmd::ExportFile(name)) => {
                self.sfx(UiSound::Click);
                let text = self.read_vid_preset_text(&name).unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Video config", &["txt", "cfg"])
                    .set_file_name(format!("{name}.txt"))
                    .save_file()
                {
                    if let Err(e) =
                        crate::fs_safe::atomic_write_with_retry(&path, text.as_bytes())
                    {
                        self.sfx(UiSound::Error);
                        self.set_status(false, format!("Не удалось сохранить файл: {e}"));
                    } else {
                        self.sfx(UiSound::Success);
                        self.set_status(true, format!("Копия сохранена: {}", path.display()));
                    }
                }
            }
            Some(VidCardCmd::Delete(name)) => {
                self.sfx(UiSound::Click);
                if self.s.video_preset == name {
                    self.s.video_preset = video_presets::PRESET_SHAURMEAW.to_owned();
                    self.select_vid_preset(video_presets::PRESET_SHAURMEAW);
                }
                self.send_disk(DiskCmd::DeleteVidPreset {
                    name,
                    steam_path: steam_dir,
                    game_path: game,
                    apply_all: self.s.video_apply_all_accounts,
                });
            }
            Some(VidCardCmd::OpenNewModal) => {
                self.sfx(UiSound::Click);
                self.new_cfg_modal = Some(ConfigSubTab::Video);
            }
            None => {}
        }
    }

    // ============================================================ Вкладка 3: Моды (.vpk)

    fn ui_mods(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.palette();
        let game = PathBuf::from(&self.s.game_path);

        ui.horizontal(|ui| {
            ui.label(theme::serif_heading(
                "МОДЫ (.VPK)",
                14.5,
                pal.accent_light,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dots_resp =
                    theme::dots_menu_button(ui, &pal).on_hover_text("Дополнительные действия");
                let popup_id = ui.make_persistent_id("mods_header_menu");
                if dots_resp.clicked() {
                    self.sfx(UiSound::Click);
                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                }
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    &dots_resp,
                    egui::PopupCloseBehavior::CloseOnClick,
                    |ui| {
                        ui.set_min_width(195.0);
                        if ui.button("Обновить список").clicked() {
                            self.sfx(UiSound::Click);
                            self.refresh_addons();
                        }
                        if ui
                            .add_enabled(
                                !self.addon_mods.is_empty() && !self.hashing_addons,
                                egui::Button::new(if self.hashing_addons {
                                    "Проверка файлов…"
                                } else {
                                    "Проверить целостность (.vpk)"
                                }),
                            )
                            .clicked()
                        {
                            self.hashing_addons = true;
                            self.set_status(true, "Проверка целостности установленных модов…");
                            if let Some(ref w) = self.io_worker {
                                w.send_heavy(HeavyCmd::HashAddonsParallel {
                                    game_path: game.clone(),
                                });
                            }
                        }
                    },
                );

                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Folder),
                    "Открыть папку",
                    egui::vec2(0.0, 28.0),
                    true,
                    &pal,
                )
                .on_hover_text("Открыть папку модов игры (addons)")
                .clicked()
                {
                    self.open_dir(tools::addons_dir(&game), true);
                }
            });
        });

        ui.add_space(12.0);

        // ---- Зона Drag & Drop сверху
        let hovering_file = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let drop_w = ui.available_width();
        let (drop_rect, drop_resp) =
            ui.allocate_exact_size(egui::vec2(drop_w, 92.0), egui::Sense::click());
        if drop_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let drop_hover_t = ui.ctx().animate_bool_with_time_and_easing(
            drop_resp.id.with("drop_zone"),
            drop_resp.hovered() || hovering_file,
            ui.style().animation_time,
            egui::emath::easing::quadratic_out,
        );
        if ui.is_rect_visible(drop_rect) {
            let p = ui.painter();
            let fill = if hovering_file {
                theme::BG_MANUSCRIPT.lerp_to_gamma(Color32::from_rgb(20, 38, 34), 0.85)
            } else {
                theme::BG_MANUSCRIPT.lerp_to_gamma(theme::SURFACE_CARD, drop_hover_t)
            };
            p.rect_filled(drop_rect, 4.0, fill);
            let stroke_col = if hovering_file {
                theme::SOUL_TEAL
            } else {
                pal.accent_border.lerp_to_gamma(pal.accent_light, drop_hover_t)
            };
            theme::paint_dashed_rect(
                p,
                drop_rect.shrink(1.0),
                egui::Stroke::new(if hovering_file { 1.8_f32 } else { 1.3_f32 }, stroke_col),
                7.0,
                5.0,
            );

            let title_col = if hovering_file {
                theme::SOUL_TEAL
            } else {
                theme::TEXT_CREAM
            };
            let ic_rect = egui::Rect::from_center_size(
                egui::pos2(drop_rect.center().x, drop_rect.center().y - 16.0),
                egui::vec2(20.0, 20.0),
            );
            theme::paint_icon(p, ic_rect, Icon::Drop, stroke_col, 1.6);

            let g1 = p.layout_no_wrap(
                "Перетащите файлы .vpk сюда для установки мода".to_owned(),
                egui::FontId::proportional(14.0),
                title_col,
            );
            p.galley(
                egui::pos2(drop_rect.center().x - g1.size().x * 0.5, drop_rect.center().y),
                g1,
                title_col,
            );
            let g2 = p.layout_no_wrap(
                "или нажмите, чтобы выбрать файл с компьютера".to_owned(),
                egui::FontId::proportional(11.5),
                theme::TEXT_MUTED,
            );
            p.galley(
                egui::pos2(drop_rect.center().x - g2.size().x * 0.5, drop_rect.center().y + 18.0),
                g2,
                theme::TEXT_MUTED,
            );
        }
        if drop_resp.clicked() {
            self.sfx(UiSound::Click);
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("Моды Deadlock (.vpk)", &["vpk"])
                .pick_files()
            {
                if !files.is_empty() {
                    self.send_disk(DiskCmd::InstallDroppedFiles {
                        paths: files,
                        game_path: game.clone(),
                    });
                }
            }
        }

        if self.dmm_conflicts > 0 {
            ui.add_space(10.0);
            theme::accent_banner_frame(
                ui,
                Color32::from_rgb(38, 22, 24),
                theme::CRIMSON,
                |ui| {
                    ui.horizontal(|ui| {
                        let (ic_r, _) = ui.allocate_exact_size(
                            egui::vec2(14.0, 14.0),
                            egui::Sense::hover(),
                        );
                        theme::paint_icon(ui.painter(), ic_r, Icon::Alert, theme::CRIMSON, 1.5);
                        theme::badge_pill(ui, "Конфликт", theme::CRIMSON, false);
                        ui.label(
                            RichText::new(format!(
                                "Обнаружено возможных пересечений модов: {}",
                                self.dmm_conflicts
                            ))
                            .color(theme::TEXT_CREAM),
                        );
                    });
                },
            );
        }

        ui.add_space(14.0);
        ui.label(theme::serif_heading(
            "УСТАНОВЛЕННЫЕ МОДЫ (.VPK)",
            12.5,
            pal.accent_light,
        ));
        ui.add_space(8.0);

        if self.addon_mods.is_empty() {
            theme::card_frame(ui, &pal, false, |ui| {
                ui.label(
                    RichText::new("Пока нет ни одного установленного .vpk мода.")
                        .color(theme::TEXT_MUTED),
                );
            });
        } else {
            let mut toggle_mod_path: Option<PathBuf> = None;
            let mods_snapshot = self.addon_mods.clone();
            for entry in &mods_snapshot {
                let mut tip = format!("Размер: {}", tools::format_size(entry.size_bytes));
                if let Some(hex) = self.addon_hashes.get(&entry.path) {
                    let short = hex.get(..12).unwrap_or(hex.as_str());
                    tip.push_str(&format!("\nКонтрольная сумма: {short}…"));
                }
                theme::card_frame(ui, &pal, entry.enabled, |ui| {
                    let mut on = entry.enabled;
                    if theme::setting_toggle_row(ui, &entry.name, &tip, &mut on, &pal).changed() {
                        toggle_mod_path = Some(entry.path.clone());
                    }
                });
                ui.add_space(8.0);
            }
            if let Some(path) = toggle_mod_path {
                self.sfx(UiSound::Toggle);
                self.send_disk(DiskCmd::ToggleAddonMod {
                    path,
                    game_path: game,
                });
            }
        }
    }

    // ============================================================ Вкладка 3: Утилиты

    fn ui_utilities(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette();
        let game = PathBuf::from(&self.s.game_path);
        let steam_path = PathBuf::from(&self.s.steam_path);
        let game_up = self.is_game_running();
        let bg_busy = self.is_bg_busy();

        if self.cache_stats.is_none() {
            self.refresh_cache_stats();
        }
        if self.cached_win_tweaks.is_none() {
            self.refresh_windows_tweaks();
        }

        ui.label(theme::serif_heading("УТИЛИТЫ", 14.5, pal.accent_light));
        ui.add_space(12.0);

        let avail_w = ui.available_width();
        let gap = 14.0_f32;
        let cols: usize = if avail_w >= 580.0 { 2 } else { 1 };
        let col_w = ((avail_w - gap * (cols.saturating_sub(1) as f32)) / cols as f32).max(220.0);

        enum UtilCmd {
            ClearShaders,
            OptimizeNow,
            OpenMapTimers,
            LaunchSandbox,
        }
        let mut util_cmd: Option<UtilCmd> = None;

        // 4 крупные карточки-действия в сетке 2×2
        for row_idx in 0..(4 / cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for col_idx in 0..cols {
                    let card_idx = row_idx * cols + col_idx;
                    ui.allocate_ui_with_layout(
                        egui::vec2(col_w, 132.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(col_w);
                            ui.set_max_width(col_w);

                            match card_idx {
                                0 => theme::card_frame(ui, &pal, false, |ui| {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(
                                            egui::vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        theme::paint_icon(
                                            ui.painter(),
                                            r,
                                            Icon::Trash,
                                            pal.accent_light,
                                            1.5,
                                        );
                                        ui.label(
                                            RichText::new("Очистить кэш шейдеров")
                                                .size(15.0)
                                                .strong()
                                                .color(theme::TEXT_CREAM),
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new("Убирает статтеры и фризы после патчей Valve")
                                            .color(theme::TEXT_MUTED)
                                            .size(12.5),
                                    );
                                    ui.add_space(12.0);
                                    let stats = self.cache_stats.clone().unwrap_or_default();
                                    let shader_tip = if game_up {
                                        "Закройте Deadlock перед очисткой кэша шейдеров".to_owned()
                                    } else {
                                        let mut t = format!(
                                            "Удаляет {} ({} файлов кэша)",
                                            tools::format_size(stats.shader_bytes),
                                            stats.shader_files
                                        );
                                        if let Some(ref vpk_dt) = stats.shader_vpk_updated {
                                            t.push_str(&format!("\nШейдеры игры обновлены: {vpk_dt}"));
                                        }
                                        t
                                    };
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        "Очистить",
                                        egui::vec2(ui.available_width(), 32.0),
                                        !game_up && !bg_busy,
                                        &pal,
                                    )
                                    .on_hover_text(shader_tip)
                                    .clicked()
                                    {
                                        util_cmd = Some(UtilCmd::ClearShaders);
                                    }
                                }),
                                1 => theme::card_frame(ui, &pal, true, |ui| {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(
                                            egui::vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        theme::paint_icon(
                                            ui.painter(),
                                            r,
                                            Icon::Bolt,
                                            pal.accent_light,
                                            1.5,
                                        );
                                        ui.label(
                                            RichText::new("Оптимизация памяти и логов")
                                                .size(15.0)
                                                .strong()
                                                .color(theme::TEXT_CREAM),
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(
                                            "Сжимает ОЗУ лаунчера и чистит старый console.log",
                                        )
                                        .color(theme::TEXT_MUTED)
                                        .size(12.5),
                                    );
                                    ui.add_space(12.0);
                                    if theme::brass_primary_button(
                                        ui,
                                        None,
                                        "Ускорить",
                                        egui::vec2(ui.available_width(), 32.0),
                                        !bg_busy,
                                        &pal,
                                    )
                                    .on_hover_text(
                                        "Освобождает неиспользуемую память и обрезает огромный лог игры",
                                    )
                                    .clicked()
                                    {
                                        util_cmd = Some(UtilCmd::OptimizeNow);
                                    }
                                }),
                                2 => theme::card_frame(ui, &pal, false, |ui| {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(
                                            egui::vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        theme::paint_icon(
                                            ui.painter(),
                                            r,
                                            Icon::Timer,
                                            pal.accent_light,
                                            1.5,
                                        );
                                        ui.label(
                                            RichText::new("Тайминги карты")
                                                .size(15.0)
                                                .strong()
                                                .color(theme::TEXT_CREAM),
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(
                                            "Шпаргалка спавна Урны Душ, Мид-Босса и рун",
                                        )
                                        .color(theme::TEXT_MUTED)
                                        .size(12.5),
                                    );
                                    ui.add_space(12.0);
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        "Открыть трекер",
                                        egui::vec2(ui.available_width(), 32.0),
                                        true,
                                        &pal,
                                    )
                                    .on_hover_text("Открывает шпаргалку и таймер объектов карты")
                                    .clicked()
                                    {
                                        util_cmd = Some(UtilCmd::OpenMapTimers);
                                    }
                                }),
                                _ => theme::card_frame(ui, &pal, false, |ui| {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(
                                            egui::vec2(16.0, 16.0),
                                            egui::Sense::hover(),
                                        );
                                        theme::paint_icon(
                                            ui.painter(),
                                            r,
                                            Icon::Play,
                                            pal.accent_light,
                                            1.5,
                                        );
                                        ui.label(
                                            RichText::new("Запуск на полигоне (Sandbox)")
                                                .size(15.0)
                                                .strong()
                                                .color(theme::TEXT_CREAM),
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(
                                            "Мгновенный тест пресета без поиска матча",
                                        )
                                        .color(theme::TEXT_MUTED)
                                        .size(12.5),
                                    );
                                    ui.add_space(12.0);
                                    if theme::brass_secondary_button(
                                        ui,
                                        None,
                                        "Тест",
                                        egui::vec2(ui.available_width(), 32.0),
                                        true,
                                        &pal,
                                    )
                                    .on_hover_text(
                                        "Сразу запускает тестовую карту для проверки FPS и настроек",
                                    )
                                    .clicked()
                                    {
                                        util_cmd = Some(UtilCmd::LaunchSandbox);
                                    }
                                }),
                            }
                        },
                    );
                }
            });
            ui.add_space(gap);
        }

        match util_cmd {
            Some(UtilCmd::ClearShaders) => {
                self.sfx(UiSound::Click);
                self.send_heavy(
                    "Очистка кэша шейдеров",
                    HeavyCmd::ClearShaderCache {
                        steam_path: steam_path.clone(),
                        game_path: game.clone(),
                    },
                );
            }
            Some(UtilCmd::OptimizeNow) => {
                self.sfx(UiSound::Click);
                self.run_manual_optimization();
            }
            Some(UtilCmd::OpenMapTimers) => {
                self.sfx(UiSound::Click);
                self.map_timers_modal_open = true;
            }
            Some(UtilCmd::LaunchSandbox) => {
                self.play_sandbox();
            }
            None => {}
        }

        // ---- Скрытые по умолчанию дополнительные инструменты (ни одного лишнего элемента на первом экране)
        ui.add_space(4.0);
        egui::CollapsingHeader::new("Дополнительные инструменты и профили")
            .id_salt("util_extra_collapsing")
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(8.0);
                // Быстрые профили оптимизации
                theme::card_frame(ui, &pal, false, |ui| {
                    ui.label(theme::serif_heading(
                        "БЫСТРЫЕ ПРОФИЛИ FPS",
                        12.0,
                        pal.accent_light,
                    ));
                    ui.add_space(8.0);
                    let mut chosen: Option<OptimizationProfile> = None;
                    ui.horizontal_wrapped(|ui| {
                        for p in [
                            OptimizationProfile::EsportsMaxFps,
                            OptimizationProfile::Balanced,
                            OptimizationProfile::StandardReset,
                        ] {
                            if theme::brass_secondary_button(
                                ui,
                                None,
                                p.label(),
                                egui::vec2(0.0, 28.0),
                                true,
                                &pal,
                            )
                            .clicked()
                            {
                                chosen = Some(p);
                            }
                        }
                    });
                    if let Some(p) = chosen {
                        self.sfx(UiSound::Success);
                        self.s.apply_optimization_profile(p);
                        self.refresh_cached_previews();
                        self.set_status(true, format!("Применён профиль: {}", p.label()));
                    }
                });

                ui.add_space(10.0);
                // Настройки Windows (без жаргона)
                let wt = self.cached_win_tweaks.unwrap_or_default();
                let mut tweak_cmd: Option<DiskCmd> = None;
                theme::card_frame(ui, &pal, false, |ui| {
                    ui.label(theme::serif_heading(
                        "ИГРОВЫЕ НАСТРОЙКИ WINDOWS",
                        12.0,
                        pal.accent_light,
                    ));
                    ui.add_space(8.0);
                    let mut game_mode = wt.game_mode_enabled;
                    if theme::setting_toggle_row(
                        ui,
                        "Игровой режим Windows",
                        "Приоритет распределения ресурсов для игр на уровне системы.",
                        &mut game_mode,
                        &pal,
                    )
                    .changed()
                    {
                        tweak_cmd = Some(DiskCmd::SetWindowsGameMode {
                            enable: game_mode,
                            game_path: game.clone(),
                        });
                    }
                    ui.add_space(4.0);
                    let mut no_dvr = wt.game_dvr_disabled;
                    if theme::setting_toggle_row(
                        ui,
                        "Отключить фоновую запись экрана (Game DVR)",
                        "Убирает лишние фоновые просадки FPS от записи Windows.",
                        &mut no_dvr,
                        &pal,
                    )
                    .changed()
                    {
                        tweak_cmd = Some(DiskCmd::SetDisableGameDvr {
                            disable: no_dvr,
                            game_path: game.clone(),
                        });
                    }
                    ui.add_space(4.0);
                    let mut gpu_hi = wt.gpu_high_perf;
                    if theme::setting_toggle_row(
                        ui,
                        "Максимальная производительность видеокарты",
                        "Закрепляет дискретную видеокарту за игрой Deadlock.",
                        &mut gpu_hi,
                        &pal,
                    )
                    .changed()
                    {
                        tweak_cmd = Some(DiskCmd::SetDeadlockHighPerfGpu {
                            enable: gpu_hi,
                            game_path: game.clone(),
                        });
                    }
                    ui.add_space(4.0);
                    let mut no_fso = wt.fso_disabled;
                    if theme::setting_toggle_row(
                        ui,
                        "Отключить экранную прослойку Windows",
                        "Может снизить задержку ввода в полноэкранном режиме.",
                        &mut no_fso,
                        &pal,
                    )
                    .changed()
                    {
                        tweak_cmd = Some(DiskCmd::SetDeadlockDisableFso {
                            disable: no_fso,
                            game_path: game.clone(),
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Check),
                            "Включить все сразу",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            tweak_cmd = Some(DiskCmd::ApplyAllWindowsTweaks {
                                game_path: game.clone(),
                            });
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Restore),
                            "Вернуть по умолчанию",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            tweak_cmd = Some(DiskCmd::RevertAllWindowsTweaks {
                                game_path: game.clone(),
                            });
                        }
                    });
                });
                if let Some(cmd) = tweak_cmd {
                    self.sfx(UiSound::Toggle);
                    self.send_disk(cmd);
                }

                ui.add_space(10.0);
                // Обслуживание и откат отчётов о сбоях
                let stats = self.cache_stats.clone().unwrap_or_default();
                theme::card_frame(ui, &pal, false, |ui| {
                    ui.label(theme::serif_heading(
                        "ОТЧЁТЫ О СБОЯХ И ПРОФИЛИ НАСТРОЕК",
                        12.0,
                        pal.accent_light,
                    ));
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "Кэш шейдеров: {}  •  Файлы отчётов о сбоях: {} ({} шт.)",
                            tools::format_size(stats.shader_bytes),
                            tools::format_size(stats.dump_bytes),
                            stats.dump_files
                        ));
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Trash),
                            "Удалить отчёты о сбоях",
                            egui::vec2(0.0, 26.0),
                            !bg_busy,
                            &pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            self.send_heavy(
                                "Удаление отчётов о сбоях",
                                HeavyCmd::ClearCrashDumps {
                                    steam_path,
                                    game_path: game.clone(),
                                },
                            );
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Refresh),
                            "Обновить размеры",
                            egui::vec2(0.0, 26.0),
                            !self.cache_scanning,
                            &pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            self.refresh_cache_stats();
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Restore),
                            "Вернуть старый autoexec",
                            egui::vec2(0.0, 26.0),
                            self.has_autoexec_bak,
                            &pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            self.send_disk(DiskCmd::RestoreAutoexecBak { game_path: game });
                        }
                    });

                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_profile_name)
                                .hint_text("Название профиля настроек")
                                .desired_width(210.0),
                        );
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Save),
                            "Сохранить профиль",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            let name = self.new_profile_name.clone();
                            self.new_profile_name.clear();
                            self.send_disk(DiskCmd::SaveProfile {
                                name,
                                settings: Box::new(self.s.clone()),
                            });
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Copy),
                            "Экспорт файла",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Настройки", &["json"])
                                .set_file_name("shaurmeaw-profile.json")
                                .save_file()
                            {
                                self.send_disk(DiskCmd::ExportSettings {
                                    path,
                                    settings: Box::new(self.s.clone()),
                                });
                            }
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Import),
                            "Импорт файла",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Настройки", &["json"])
                                .pick_file()
                            {
                                self.send_disk(DiskCmd::ImportSettings {
                                    path,
                                    current: Box::new(self.s.clone()),
                                });
                            }
                        }
                    });
                    self.ui_profile_list(ui);
                });

                ui.add_space(10.0);
                // Полезные ссылки
                theme::card_frame(ui, &pal, false, |ui| {
                    ui.label(theme::serif_heading(
                        "ПОЛЕЗНЫЕ РЕСУРСЫ DEADLOCK",
                        12.0,
                        pal.accent_light,
                    ));
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (title, desc, url) in COMMUNITY_LINKS {
                            if theme::brass_secondary_button(
                                ui,
                                Some(Icon::External),
                                title,
                                egui::vec2(148.0, 28.0),
                                true,
                                &pal,
                            )
                            .on_hover_text(desc)
                            .clicked()
                            {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            }
                        }
                    });
                });
            });
    }

    fn ui_profile_list(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette();
        if self.profiles.is_empty() {
            return;
        }
        ui.add_space(6.0);
        let mut action_load: Option<String> = None;
        let mut action_del: Option<String> = None;
        for name in &self.profiles {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).color(pal.accent_light));
                if theme::brass_secondary_button(
                    ui,
                    Some(Icon::Play),
                    "Загрузить",
                    egui::vec2(0.0, 24.0),
                    true,
                    &pal,
                )
                .clicked()
                {
                    action_load = Some(name.clone());
                }
                if theme::crimson_button(
                    ui,
                    Some(Icon::Trash),
                    "",
                    egui::vec2(26.0, 24.0),
                    true,
                )
                .on_hover_text("Удалить сохранённый профиль")
                .clicked()
                {
                    action_del = Some(name.clone());
                }
            });
        }
        if let Some(name) = action_load {
            self.send_disk(DiskCmd::LoadProfile {
                name,
                current: Box::new(self.s.clone()),
            });
        }
        if let Some(name) = action_del {
            self.send_disk(DiskCmd::DeleteProfile { name });
        }
    }

    // ============================================================ Вкладка 5: Настройки (Progressive Disclosure)

    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.palette();
        let game = PathBuf::from(&self.s.game_path);

        ui.label(theme::serif_heading("НАСТРОЙКИ", 14.5, pal.accent_light));
        ui.add_space(12.0);

        // ---- Карточка 1: Игра и Пути
        theme::card_frame(ui, &pal, false, |ui| {
            ui.label(theme::serif_heading(
                "1. ИГРА И ПУТИ",
                13.0,
                pal.accent_light,
            ));
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Папка Deadlock").color(theme::TEXT_CREAM));
                ui.add_space(6.0);
                if self.paths_game_ok {
                    theme::badge_pill(ui, "Автоопределено", theme::SOUL_TEAL, true);
                } else {
                    theme::badge_pill(ui, "Не найдено", theme::CRIMSON, true);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::brass_secondary_button(
                        ui,
                        None,
                        "Изменить",
                        egui::vec2(0.0, 28.0),
                        true,
                        &pal,
                    )
                    .on_hover_text("Выбрать папку с игрой Deadlock вручную")
                    .clicked()
                    {
                        self.sfx(UiSound::Click);
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.s.game_path = p.display().to_string();
                            self.refresh_paths_state();
                        }
                    }
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Refresh),
                        "Автопоиск",
                        egui::vec2(0.0, 28.0),
                        true,
                        &pal,
                    )
                    .on_hover_text("Найти папки Steam и Deadlock автоматически")
                    .clicked()
                    {
                        self.sfx(UiSound::Click);
                        self.auto_detect(true);
                    }
                });
            });

            let path_display = if self.s.game_path.trim().is_empty() {
                "Папка не выбрана — нажмите «Автопоиск» или «Изменить»".to_owned()
            } else {
                self.s.game_path.clone()
            };
            ui.add(
                egui::Label::new(theme::mono_text(&path_display, 11.5, theme::TEXT_MUTED))
                    .truncate(),
            )
            .on_hover_text(&path_display);

            ui.add_space(10.0);
            if theme::setting_toggle_row(
                ui,
                "Авто-EN раскладка при входе в игру",
                "При запуске Deadlock переключает клавиатуру окна игры на английский, чтобы бинды всегда срабатывали.",
                &mut self.s.auto_english_layout,
                &pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Расширенные настройки запуска, графики и конфигов")
                .id_salt("settings_adv_game")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    // Steam и режим запуска
                    ui.horizontal(|ui| {
                        let steam_badge = if self.is_steam_running() {
                            "Steam запущен"
                        } else if self.paths_steam_ok {
                            "Steam найден"
                        } else {
                            "Steam не найден"
                        };
                        ui.label("Папка Steam:")
                            .on_hover_text(steam_badge);
                        let w = (ui.available_width() - 108.0).clamp(150.0, 420.0);
                        if ui
                            .add(egui::TextEdit::singleline(&mut self.s.steam_path).desired_width(w))
                            .changed()
                        {
                            self.refresh_paths_state();
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Folder),
                            "Обзор…",
                            egui::vec2(0.0, 24.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                self.s.steam_path = p.display().to_string();
                                self.refresh_paths_state();
                            }
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Способ запуска:");
                        ui.radio_value(&mut self.s.launch_mode, LaunchMode::Steam, "Через Steam");
                        ui.radio_value(
                            &mut self.s.launch_mode,
                            LaunchMode::Direct,
                            "Напрямую deadlock.exe",
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.s.novid, "Пропустить интро (-novid)")
                            .on_hover_text("Пропускает вступительный ролик при старте игры.");
                        ui.checkbox(&mut self.s.console, "Консоль (-console)")
                            .on_hover_text("Делает консоль разработчика доступной в игре.");
                        ui.checkbox(&mut self.s.high_priority, "Высокий приоритет (-high)")
                            .on_hover_text("Передаёт флаг приоритета в параметрах запуска.");
                        ui.checkbox(&mut self.s.nojoy, "Без геймпада (-nojoy)")
                            .on_hover_text("Отключает опрос контроллеров для экономии ресурсов.");
                        ui.checkbox(&mut self.s.dev, "Режим отладки (-dev)")
                            .on_hover_text("Включает расширенную отладку движка.");
                    });

                    ui.add_space(6.0);
                    egui::Grid::new("adv_launch_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Графический API:");
                            egui::ComboBox::from_id_salt("adv_render_api")
                                .width(240.0)
                                .selected_text(self.s.render_api.label())
                                .show_ui(ui, |ui| {
                                    for v in RenderApi::ALL {
                                        ui.selectable_value(&mut self.s.render_api, v, v.label());
                                    }
                                });
                            ui.end_row();

                            ui.label("Режим окна:");
                            egui::ComboBox::from_id_salt("adv_window_mode")
                                .width(240.0)
                                .selected_text(self.s.window_mode.label())
                                .show_ui(ui, |ui| {
                                    for v in WindowMode::ALL {
                                        ui.selectable_value(&mut self.s.window_mode, v, v.label());
                                    }
                                });
                            ui.end_row();

                            ui.label("Своё разрешение:");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.s.custom_resolution, "");
                                ui.add_enabled(
                                    self.s.custom_resolution,
                                    egui::DragValue::new(&mut self.s.width).range(640..=7680),
                                );
                                ui.label("×");
                                ui.add_enabled(
                                    self.s.custom_resolution,
                                    egui::DragValue::new(&mut self.s.height).range(480..=4320),
                                );
                            });
                            ui.end_row();

                            ui.label("Язык игры:");
                            let cur_lang = LANGUAGES
                                .iter()
                                .find(|(code, _)| *code == self.s.language)
                                .map(|(_, name)| *name)
                                .unwrap_or(self.s.language.as_str())
                                .to_owned();
                            egui::ComboBox::from_id_salt("adv_language")
                                .width(240.0)
                                .selected_text(cur_lang)
                                .show_ui(ui, |ui| {
                                    for (code, name) in LANGUAGES {
                                        ui.selectable_value(
                                            &mut self.s.language,
                                            code.to_owned(),
                                            name,
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label("Доп. параметры:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.s.extra_launch_args)
                                    .hint_text("например: -fullscreen_refresh 144")
                                    .desired_width(240.0),
                            );
                            ui.end_row();
                        });

                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if !self.has_shortcut {
                            if theme::brass_secondary_button(
                                ui,
                                Some(Icon::Plus),
                                "Ярлык на рабочий стол",
                                egui::vec2(0.0, 26.0),
                                true,
                                &pal,
                            )
                            .on_hover_text("Запускает Deadlock с вашими конфигами в 1 клик")
                            .clicked()
                            {
                                self.send_disk(DiskCmd::ToggleDesktopShortcut { create: true });
                            }
                        } else if theme::crimson_button(
                            ui,
                            Some(Icon::Trash),
                            "Удалить ярлык",
                            egui::vec2(0.0, 26.0),
                            true,
                        )
                        .clicked()
                        {
                            self.send_disk(DiskCmd::ToggleDesktopShortcut { create: false });
                        }

                        if !self.is_portable {
                            if theme::brass_secondary_button(
                                ui,
                                Some(Icon::Archive),
                                "Портативный режим",
                                egui::vec2(0.0, 26.0),
                                true,
                                &pal,
                            )
                            .on_hover_text("Хранить все настройки рядом с ShaurMeaw.exe")
                            .clicked()
                            {
                                self.apply_portable_toggle(true);
                            }
                        } else if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Restore),
                            "Хранить в профиле Windows",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            self.apply_portable_toggle(false);
                        }

                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Folder),
                            "Папка игры",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .clicked()
                        {
                            self.open(game.clone());
                        }
                    });

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Профили autoexec и video:");
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Edit),
                            "Открыть autoexec.cfg",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .on_hover_text("Перейти во вкладку «Конфиги» → autoexec.cfg")
                        .clicked()
                        {
                            self.config_subtab = ConfigSubTab::Autoexec;
                            self.switch_tab(ctx, Tab::Configs);
                        }
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Edit),
                            "Открыть video.txt",
                            egui::vec2(0.0, 26.0),
                            true,
                            &pal,
                        )
                        .on_hover_text("Перейти во вкладку «Конфиги» → video.txt")
                        .clicked()
                        {
                            self.config_subtab = ConfigSubTab::Video;
                            self.switch_tab(ctx, Tab::Configs);
                        }
                    });
                });
        });

        ui.add_space(14.0);

        // ---- Карточка 2: Система и Трей
        theme::card_frame(ui, &pal, false, |ui| {
            ui.label(theme::serif_heading(
                "2. СИСТЕМА И ТРЕЙ",
                13.0,
                pal.accent_light,
            ));
            ui.add_space(10.0);

            let mut autostart = self.autostart_sync;
            if theme::setting_toggle_row(
                ui,
                "Запускать с Windows",
                "Тихо проверяет защиту конфига при старте Windows, не открывая лишних окон.",
                &mut autostart,
                &pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
                self.autostart_sync = autostart;
                self.send_disk(DiskCmd::SetAutostart { enable: autostart });
            }

            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Сворачивать в трей",
                "При сворачивании окно уходит в значок около часов и почти не тратит память.",
                &mut self.s.minimize_to_tray,
                &pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }

            ui.add_space(8.0);
            if theme::setting_toggle_row(
                ui,
                "Повышать приоритет FPS в игре",
                "Выдаёт игре высокий приоритет процессора и экономит ресурсы лаунчера во время матча.",
                &mut self.s.auto_high_priority,
                &pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Расширенные настройки защиты, памяти и движка")
                .id_salt("settings_adv_sys")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.s.gameinfo.enabled,
                        "Автозащита конфига от патчей Valve",
                    )
                    .on_hover_text("Аккуратно переносит ваши настройки, если обновление изменит файл игры.");
                    ui.add_enabled_ui(self.s.gameinfo.enabled, |ui| {
                        ui.checkbox(
                            &mut self.s.gameinfo.auto_watch_updates,
                            "Автоматически отслеживать обновления файла игры в фоне",
                        )
                        .on_hover_text("Мгновенно замечает перезапись после патчей Steam.");
                        ui.checkbox(
                            &mut self.s.gameinfo.rebuild_before_launch,
                            "Проверять целостность конфига перед каждым запуском игры",
                        )
                        .on_hover_text("Гарантирует актуальный профиль прямо перед стартом Deadlock.");
                    });

                    ui.add_space(4.0);
                    ui.checkbox(
                        &mut self.s.ram_trim_on_minimize,
                        "Экономия оперативной памяти при сворачивании и запуске игры",
                    )
                    .on_hover_text("Освобождает неиспользуемую память лаунчера до 3–8 МБ.");
                    ui.checkbox(
                        &mut self.s.boost_timer_1ms,
                        "Стабильная плавность кадров (точный системный таймер на время матча)",
                    )
                    .on_hover_text("Уменьшает разброс времени кадра Windows только пока открыт Deadlock.");
                    ui.checkbox(
                        &mut self.s.boost_power_plan,
                        "Питание «Высокая производительность» во время матча",
                    )
                    .on_hover_text("Автоматически возвращает прежний режим питания после выхода из игры.");
                    ui.checkbox(
                        &mut self.s.auto_clean_console_log,
                        "Автоочистка разросшегося файла console.log (> 50 МБ)",
                    )
                    .on_hover_text("Не даёт журналу игры тормозить запуск и забивать диск.");

                    let cpu_count = game_booster::logical_cpu_count();
                    ui.add_enabled(
                        cpu_count >= 6,
                        egui::Checkbox::new(
                            &mut self.s.boost_disable_cpu0,
                            "Снимать нагрузку системного 0-го ядра с игры (для CPU от 6 потоков)",
                        ),
                    )
                    .on_hover_text("Может убрать редкие микрофризы от фоновых прерываний Windows.");

                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Снижение задержки ввода:");
                        for (val, lbl) in [(0u8, "Выкл"), (1, "Вкл"), (2, "Усиленный")] {
                            if theme::deco_chip(ui, self.s.low_latency_mode == val, lbl, &pal)
                                .clicked()
                            {
                                self.s.low_latency_mode = val;
                            }
                        }
                    });
                    ui.checkbox(
                        &mut self.s.optimize_network,
                        "Оптимизация сетевого отклика (минимальная задержка пакетов)",
                    )
                    .on_hover_text("Настраивает оптимальный размер сетевого буфера Deadlock.");
                    ui.checkbox(
                        &mut self.s.optimize_audio_latency,
                        "Минимальная задержка звука в бою",
                    )
                    .on_hover_text("Ускоряет отклик аудиомикшера игры.");
                    ui.checkbox(
                        &mut self.s.disable_ragdolls_decals,
                        "Убрать физику тел и следы пуль (для слабого ПК)",
                    );
                    ui.checkbox(
                        &mut self.s.disable_shadows_autoexec,
                        "Отключить динамические тени",
                    );
                });
        });

        ui.add_space(14.0);

        // ---- Карточка 3: Discord Статус
        if let Some(ref rpc) = self.discord_rpc {
            rpc.snapshot_if_changed(&mut self.cached_rpc_rev, &mut self.cached_rpc_snap);
        }
        let snap_status = self.cached_rpc_snap.status;
        let mut want_reconnect = false;
        theme::card_frame(ui, &pal, false, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::serif_heading(
                    "3. DISCORD СТАТУС",
                    13.0,
                    pal.accent_light,
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (badge_col, dot) = match snap_status {
                        RpcConnectionStatus::Connected => (theme::SOUL_TEAL, true),
                        RpcConnectionStatus::WaitingForGame
                        | RpcConnectionStatus::WaitingForDiscord
                        | RpcConnectionStatus::NeedsAppId => (pal.accent_light, true),
                        RpcConnectionStatus::Disabled => (theme::TEXT_MUTED, false),
                    };
                    theme::badge_pill(ui, snap_status.label(), badge_col, dot);
                });
            });
            ui.add_space(10.0);

            let rpc = &mut self.s.discord_rpc;
            if theme::setting_toggle_row(
                ui,
                "Включить статус в Discord",
                "Показывает в вашем профиле Discord текущую активность в Deadlock.",
                &mut rpc.enabled,
                &pal,
            )
            .changed()
            {
                sfx::play(
                    UiSound::Toggle,
                    self.s.ui_sound_enabled,
                    self.s.ui_sound_volume,
                );
            }

            ui.add_enabled_ui(rpc.enabled, |ui| {
                ui.add_space(8.0);
                let hero_tip = format!(
                    "Автоматически определяет выбранного героя ({} персонажей Deadlock) и режим матча.",
                    crate::discord_rpc::HERO_COUNT
                );
                if theme::setting_toggle_row(
                    ui,
                    "Показывать героя и матч",
                    &hero_tip,
                    &mut rpc.show_hero,
                    &pal,
                )
                .changed()
                {
                    sfx::play(
                        UiSound::Toggle,
                        self.s.ui_sound_enabled,
                        self.s.ui_sound_volume,
                    );
                }

                ui.add_space(8.0);
                if theme::setting_toggle_row(
                    ui,
                    "Показывать название пресета",
                    "Отображает название вашего активного профиля настроек рядом со статусом.",
                    &mut rpc.show_preset,
                    &pal,
                )
                .changed()
                {
                    sfx::play(
                        UiSound::Toggle,
                        self.s.ui_sound_enabled,
                        self.s.ui_sound_volume,
                    );
                }

                ui.add_space(8.0);
                egui::CollapsingHeader::new("Расширенные настройки")
                    .id_salt("settings_adv_discord")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        ui.checkbox(
                            &mut rpc.show_steam_name,
                            "Показывать никнейм из Steam",
                        )
                        .on_hover_text("Добавляет имя вашего аккаунта Steam в строку статуса.");
                        ui.checkbox(
                            &mut rpc.show_hero_icon,
                            "Показывать портрет выбранного героя",
                        )
                        .on_hover_text("Отображает официальную иконку персонажа Deadlock.");
                        ui.add_enabled(
                            !rpc.app_icon_url.trim().is_empty()
                                && rpc.show_hero
                                && rpc.show_hero_icon,
                            egui::Checkbox::new(
                                &mut rpc.hero_as_large_icon,
                                "Сделать портрет героя крупной картинкой",
                            ),
                        );
                        ui.checkbox(&mut rpc.show_timer, "Показывать длительность матча")
                            .on_hover_text("Запускает счётчик времени с начала карты.");
                        ui.checkbox(
                            &mut rpc.auto_condebug,
                            "Разрешать игре вести журнал событий для статуса",
                        )
                        .on_hover_text("Необходимо для точного определения героя и режима игры.");

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Application ID:");
                            ui.add(
                                egui::TextEdit::singleline(&mut rpc.client_id)
                                    .hint_text(format!(
                                        "По умолчанию: {}",
                                        crate::discord_rpc::DEFAULT_DISCORD_APP_ID
                                    ))
                                    .desired_width(210.0),
                            )
                            .on_hover_text("Оставьте пустым, чтобы использовать стандартный ID приложения Deadlock.");
                            if theme::brass_secondary_button(
                                ui,
                                None,
                                "Переподключить",
                                egui::vec2(0.0, 24.0),
                                true,
                                &pal,
                            )
                            .clicked()
                            {
                                want_reconnect = true;
                            }
                            if theme::brass_secondary_button(
                                ui,
                                None,
                                "Портал Discord",
                                egui::vec2(0.0, 24.0),
                                true,
                                &pal,
                            )
                            .clicked()
                            {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(
                                    "https://discord.com/developers/applications",
                                ));
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Своя иконка (URL):");
                            ui.add(
                                egui::TextEdit::singleline(&mut rpc.app_icon_url)
                                    .hint_text("https://.../icon.png")
                                    .desired_width(240.0),
                            );
                        });
                        if crate::discord_rpc::is_local_filesystem_path(&rpc.app_icon_url) {
                            ui.label(
                                RichText::new("Укажите прямую https://-ссылку — локальные файлы C:\\ Discord не примет.")
                                    .color(theme::CRIMSON)
                                    .small(),
                            );
                        }
                        let snap = &self.cached_rpc_snap;
                        if !snap.details.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("Сейчас: {}  {}", snap.details, snap.state))
                                    .color(theme::TEXT_MUTED)
                                    .small(),
                            );
                        }
                    });
            });
        });
        if want_reconnect {
            self.sfx(UiSound::Click);
            if let Some(ref rpc) = self.discord_rpc {
                rpc.reconnect();
            }
        }

        ui.add_space(14.0);

        // ---- Карточка 4: Звук и Внешний вид
        theme::card_frame(ui, &pal, false, |ui| {
            ui.label(theme::serif_heading(
                "4. ЗВУК И ВНЕШНИЙ ВИД",
                13.0,
                pal.accent_light,
            ));
            ui.add_space(10.0);

            if theme::setting_toggle_row(
                ui,
                "Звуки интерфейса",
                "Короткие тактильные отклики при нажатии кнопок и запуске Deadlock.",
                &mut self.s.ui_sound_enabled,
                &pal,
            )
            .changed()
            {
                self.sfx(UiSound::Toggle);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Громкость звуков:").color(theme::TEXT_MUTED));
                let slider_resp = ui.add_enabled(
                    self.s.ui_sound_enabled,
                    egui::Slider::new(&mut self.s.ui_sound_volume, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{}%", (v * 100.0).round() as u32)),
                );
                if slider_resp.drag_stopped() {
                    self.sfx(UiSound::Click);
                }
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Палитра акцентов:").color(theme::TEXT_CREAM));
                for th in Theme::ALL {
                    if theme::deco_chip(ui, self.s.theme == th, th.label(), &pal).clicked() {
                        self.s.theme = th;
                        self.sfx(UiSound::Click);
                    }
                }
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Масштаб окна и анимации")
                .id_salt("settings_adv_appearance")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Масштаб интерфейса:");
                        ui.add(
                            egui::Slider::new(&mut self.s.ui_scale, 0.85..=1.25)
                                .step_by(0.05)
                                .fixed_decimals(2),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Шрифт редактора кода:");
                        ui.add(
                            egui::Slider::new(&mut self.s.editor_font_size, 11.0..=17.0)
                                .step_by(1.0)
                                .fixed_decimals(0),
                        );
                    });
                    ui.checkbox(
                        &mut self.s.animations_enabled,
                        "Плавные анимации переходов",
                    );
                });

            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if theme::crimson_button(
                    ui,
                    Some(Icon::Restore),
                    "Восстановить оригинальные файлы Valve",
                    egui::vec2(0.0, 30.0),
                    true,
                )
                .on_hover_text(
                    "Вернёт чистые gameinfo.gi, autoexec.cfg и сбросит настройки видео (с сохранением бэкапов)",
                )
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    self.send_disk(DiskCmd::ResetAllToValve {
                        settings: Box::new(self.s.clone()),
                    });
                }

                if theme::brass_secondary_button(
                    ui,
                    None,
                    "Сбросить настройки лаунчера",
                    egui::vec2(0.0, 30.0),
                    true,
                    &pal,
                )
                .on_hover_text("Вернёт настройки лаунчера по умолчанию, сохранив пути к игре")
                .clicked()
                {
                    self.sfx(UiSound::Click);
                    let (steam_path, game_path, th) = (
                        self.s.steam_path.clone(),
                        self.s.game_path.clone(),
                        self.s.theme,
                    );
                    self.s = Settings {
                        steam_path,
                        game_path,
                        theme: th,
                        ..Settings::default()
                    };
                    self.refresh_vid_targets();
                    self.refresh_gi_state();
                    self.set_status(true, "Настройки сброшены (пути и тема сохранены)");
                }
            });
        });
    }

    fn ui_builder_controls(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Пресет конструктора:");
            for p in Preset::ALL {
                if theme::brass_secondary_button(
                    ui,
                    None,
                    p.label(),
                    egui::vec2(0.0, 24.0),
                    true,
                    pal,
                )
                .clicked()
                {
                    self.s.apply_preset(p);
                    self.set_status(true, format!("Применён пресет «{}»", p.label()));
                }
            }
        });

        ui.add_space(4.0);
        let s = &mut self.s;
        ui.horizontal_wrapped(|ui| {
            ui.label("Лимит FPS:");
            ui.add(egui::Slider::new(&mut s.fps_max, 0..=1000).step_by(1.0));
            for v in [60, 144, 240, 360, 0] {
                let text = if v == 0 { "∞".to_owned() } else { v.to_string() };
                if theme::deco_chip(ui, s.fps_max == v, &text, pal).clicked() {
                    s.fps_max = v;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut s.show_fps, "Показывать FPS");
            ui.checkbox(&mut s.no_focus_sleep, "Не снижать FPS в фоне");
            ui.checkbox(&mut s.sound_on_lost_focus, "Звук при свёрнутой игре");
        });

        ui.checkbox(
            &mut s.graphics.enabled,
            "Управлять качеством графики через autoexec",
        );
        ui.add_enabled_ui(s.graphics.enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                let g = &mut s.graphics;
                ui.checkbox(&mut g.ssao, "Затенение (SSAO)");
                ui.checkbox(&mut g.fog, "Туман");
                ui.checkbox(&mut g.outlines, "Обводка героев");
                ui.checkbox(&mut g.depth_of_field, "Размытие фона");
                ui.checkbox(&mut g.farfield, "Дальность объектов");
                ui.checkbox(&mut g.dashboard_hq, "Чёткое меню");
            });
            ui.horizontal(|ui| {
                ui.label("Детализация частиц:");
                ui.add(egui::Slider::new(&mut s.graphics.particle_detail, 0..=3));
            });
        });

        ui.add_space(4.0);
        let gp = &mut s.gameplay;
        ui.horizontal(|ui| {
            ui.checkbox(&mut gp.manage_volume, "Громкость в игре:");
            ui.add_enabled(
                gp.manage_volume,
                egui::Slider::new(&mut gp.volume, 0.0..=1.0).fixed_decimals(2),
            );
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut gp.manage_sensitivity, "Чувствительность мыши:");
            ui.add_enabled(
                gp.manage_sensitivity,
                egui::Slider::new(&mut gp.sensitivity, 0.1..=10.0).fixed_decimals(2),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut gp.telemetry_fps, "HUD: FPS");
            ui.checkbox(&mut gp.telemetry_ping, "HUD: Пинг");
            ui.checkbox(&mut gp.telemetry_loss, "HUD: Потеря пакетов");
        });

        ui.add_space(6.0);
        let c = &mut s.crosshair;
        ui.checkbox(&mut c.enabled, "Свой прицел в autoexec");
        if c.enabled {
            let bg_idx = &mut self.crosshair_bg;
            ui.horizontal_top(|ui| {
                egui::Grid::new("adv_crosshair_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Цвет:");
                        ui.color_edit_button_srgb(&mut c.color);
                        ui.end_row();
                        ui.label("Зазор:");
                        ui.add(egui::Slider::new(&mut c.gap, 0.0..=30.0).step_by(1.0));
                        ui.end_row();
                        ui.label("Толщина:");
                        ui.add(egui::Slider::new(&mut c.width, 1.0..=10.0).step_by(1.0));
                        ui.end_row();
                        ui.label("Длина:");
                        ui.add(egui::Slider::new(&mut c.height, 0.0..=30.0).step_by(1.0));
                        ui.end_row();
                        ui.label("Точка:");
                        ui.add(egui::Slider::new(&mut c.dot_opacity, 0.0..=1.0));
                        ui.end_row();
                        ui.label("Обводка:");
                        ui.checkbox(&mut c.border, "");
                        ui.end_row();
                    });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let bg = CROSSHAIR_BGS[*bg_idx % CROSSHAIR_BGS.len()].1;
                    draw_crosshair_preview(ui, c, bg, pal);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for (i, (label, _)) in CROSSHAIR_BGS.iter().enumerate() {
                            if theme::deco_chip(ui, *bg_idx == i, label, pal).clicked() {
                                *bg_idx = i;
                            }
                        }
                    });
                });
            });
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Бинды клавиш:").strong());
        let mut remove_bind: Option<usize> = None;
        for (i, bind) in self.s.binds.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut bind.key).desired_width(80.0));
                ui.add(egui::TextEdit::singleline(&mut bind.command).desired_width(240.0));
                if theme::crimson_button(ui, Some(Icon::Trash), "", egui::vec2(26.0, 24.0), true)
                    .clicked()
                {
                    remove_bind = Some(i);
                }
            });
        }
        if let Some(i) = remove_bind {
            self.s.binds.remove(i);
        }
        if theme::brass_secondary_button(
            ui,
            Some(Icon::Plus),
            "Добавить бинд",
            egui::vec2(0.0, 24.0),
            true,
            pal,
        )
        .clicked()
        {
            self.s.binds.push(Bind::new("", ""));
        }
    }

    // ============================================================ Модальные окна

    fn any_modal_open(&self) -> bool {
        self.code_modal.is_some()
            || self.new_preset_modal_open
            || self.new_cfg_modal.is_some()
            || self.rename_modal_target.is_some()
            || self.gi_tweaks_modal_open
            || self.map_timers_modal_open
    }

    fn close_top_modal(&mut self) {
        if self.code_modal.is_some() {
            self.code_modal = None;
        } else if self.rename_modal_target.is_some() {
            self.rename_modal_target = None;
        } else if self.new_preset_modal_open {
            self.new_preset_modal_open = false;
        } else if self.new_cfg_modal.is_some() {
            self.new_cfg_modal = None;
        } else if self.gi_tweaks_modal_open {
            self.gi_tweaks_modal_open = false;
        } else if self.map_timers_modal_open {
            self.map_timers_modal_open = false;
        }
    }

    fn ui_modals(&mut self, ctx: &egui::Context) {
        if !self.any_modal_open() {
            return;
        }
        let pal = self.palette();

        // Затемняющая подложка под модальными окнами
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("modal_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    r,
                    0.0,
                    Color32::from_rgba_unmultiplied(0x0B, 0x0C, 0x0E, 175),
                );
            });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.code_modal.is_some() && !self.editor_ui.search_query.is_empty() {
                self.editor_ui.search_query.clear();
                self.editor_ui.cached_matches.clear();
                self.editor_ui.match_key = None;
                self.editor_ui.active_match = 0;
                return;
            }
            self.close_top_modal();
            return;
        }

        if let Some(kind) = self.code_modal {
            self.ui_modal_code_editor(ctx, kind, &pal);
        }
        if self.new_preset_modal_open {
            self.ui_modal_new_preset(ctx, &pal);
        }
        if let Some(sub) = self.new_cfg_modal {
            self.ui_modal_new_config(ctx, sub, &pal);
        }
        if let Some(target) = self.rename_modal_target.clone() {
            self.ui_modal_rename_preset(ctx, target, &pal);
        }
        if self.gi_tweaks_modal_open {
            self.ui_modal_gi_tweaks(ctx, &pal);
        }
        if self.map_timers_modal_open {
            self.ui_modal_map_timers(ctx, &pal);
        }
    }

    fn sync_editor_cache(ui_state: &mut EditorUiState, kind: ConfigKind, text: &str) {
        let rev = ui_state.text_rev;
        if ui_state.parsed_key != Some((kind, rev)) {
            ui_state.cached_line_count = text.bytes().filter(|&b| b == b'\n').count() + 1;
            ui_state.cached_entries = config_editor::parse_config_entries(text, kind);
            ui_state.cached_present_keys = ui_state
                .cached_entries
                .iter()
                .map(|e| e.key.to_ascii_lowercase())
                .collect();
            ui_state.cached_brace_check = if kind.is_vdf() {
                Some(config_editor::validate_vdf_braces(text))
            } else {
                None
            };
            ui_state.parsed_key = Some((kind, rev));
        }

        let need_matches = ui_state
            .match_key
            .as_ref()
            .map(|(k, r, q, cs)| {
                *k != kind || *r != rev || q != &ui_state.search_query || *cs != ui_state.case_sensitive
            })
            .unwrap_or(true);
        if need_matches {
            ui_state.cached_matches = config_editor::find_all_matches(
                text,
                &ui_state.search_query,
                ui_state.case_sensitive,
            );
            if ui_state.cached_matches.is_empty() {
                ui_state.active_match = 0;
            } else if ui_state.active_match >= ui_state.cached_matches.len() {
                ui_state.active_match = 0;
            }
            ui_state.match_key = Some((
                kind,
                rev,
                ui_state.search_query.clone(),
                ui_state.case_sensitive,
            ));
        }
    }

    fn sync_filtered_entries(ui_state: &mut EditorUiState, kind: ConfigKind) {
        let rev = ui_state.text_rev;
        let need_filter = ui_state
            .filtered_key
            .as_ref()
            .map(|(k, r, q, oa)| {
                *k != kind
                    || *r != rev
                    || q != &ui_state.search_query
                    || *oa != ui_state.cmd_only_active
            })
            .unwrap_or(true);
        if !need_filter {
            return;
        }

        let q_lower = ui_state.search_query.trim().to_lowercase();
        let only_active = ui_state.cmd_only_active;
        ui_state.cached_filtered_entries.clear();

        for (idx, entry) in ui_state.cached_entries.iter().enumerate() {
            if only_active && entry.commented {
                continue;
            }
            if !q_lower.is_empty() {
                let catalog_info = config_editor::describe_command(kind, &entry.key);
                let matches_entry = entry.key.to_lowercase().contains(&q_lower)
                    || entry.value.to_lowercase().contains(&q_lower)
                    || entry.section.to_lowercase().contains(&q_lower)
                    || entry
                        .inline_comment
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q_lower)
                    || catalog_info
                        .map(|c| c.summary_ru.to_lowercase().contains(&q_lower))
                        .unwrap_or(false);
                if !matches_entry {
                    continue;
                }
            }
            ui_state.cached_filtered_entries.push(idx);
        }

        ui_state.filtered_key = Some((
            kind,
            rev,
            ui_state.search_query.clone(),
            only_active,
        ));
    }

    fn jump_editor_to_match(ui_state: &mut EditorUiState, text: &str, idx: usize) {
        let Some(m) = ui_state.cached_matches.get(idx).copied() else {
            return;
        };
        ui_state.active_match = idx;
        ui_state.scroll_to_line = Some(m.line_idx);
        let start_c = text.get(..m.start_byte).map(|s| s.chars().count()).unwrap_or(0);
        let match_c = text
            .get(m.start_byte..m.end_byte)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        ui_state.select_char_range = Some((start_c, start_c + match_c));
    }

    fn save_editor_preset(&mut self, kind: ConfigKind, sel_name: String, and_apply: bool) {
        if sel_name.is_empty() || (kind == ConfigKind::Autoexec && sel_name == MODE_BUILDER) {
            return;
        }
        let (preset_kind, content, source_url, settings_box) = match kind {
            ConfigKind::GameInfo => {
                let source_url = self
                    .gi_presets
                    .iter()
                    .find(|p| p.name == sel_name)
                    .and_then(|p| p.source_url.clone());
                let mut s_copy = self.s.clone();
                if and_apply {
                    s_copy.gameinfo.active_preset = sel_name.clone();
                    self.s.gameinfo.active_preset = sel_name.clone();
                }
                (
                    PresetKind::GameInfo,
                    self.gi_editor_text.clone(),
                    source_url,
                    Box::new(s_copy),
                )
            }
            ConfigKind::Autoexec => {
                if and_apply {
                    self.s.autoexec_preset = sel_name.clone();
                }
                (
                    PresetKind::Autoexec,
                    self.ae_editor_text.clone(),
                    None,
                    Box::new(self.s.clone()),
                )
            }
            ConfigKind::Video => {
                if and_apply {
                    self.s.video_preset = sel_name.clone();
                }
                (
                    PresetKind::Video,
                    self.vid_editor_text.clone(),
                    None,
                    Box::new(self.s.clone()),
                )
            }
        };

        self.send_disk(DiskCmd::SavePresetText {
            kind: preset_kind,
            name: sel_name,
            content,
            source_url,
            and_apply,
            settings: Some(settings_box),
        });
    }

    fn revert_editor_preset(&mut self, kind: ConfigKind, sel_name: &str) {
        let orig = {
            let r = self.cache.read();
            match kind {
                ConfigKind::GameInfo => r.gi_texts.get(sel_name).cloned(),
                ConfigKind::Autoexec => r.ae_texts.get(sel_name).cloned(),
                ConfigKind::Video => r.vid_texts.get(sel_name).cloned(),
            }
        };
        if let Some(txt) = orig {
            match kind {
                ConfigKind::GameInfo => {
                    self.gi_editor_text = txt;
                    self.gi_editor_dirty = false;
                }
                ConfigKind::Autoexec => {
                    self.ae_editor_text = txt;
                    self.ae_editor_dirty = false;
                }
                ConfigKind::Video => {
                    self.vid_editor_text = txt;
                    self.vid_editor_dirty = false;
                }
            }
            self.editor_ui.text_rev = self.editor_ui.text_rev.wrapping_add(1);
            self.sfx(UiSound::Click);
            self.set_status(true, format!("Правки в «{sel_name}» отменены"));
        }
    }

    fn ui_modal_code_editor(&mut self, ctx: &egui::Context, kind: ConfigKind, pal: &Palette) {
        let mut open = true;
        let sel_name = match kind {
            ConfigKind::GameInfo => self.selected_gi_preset.clone(),
            ConfigKind::Autoexec => self.selected_ae_preset.clone(),
            ConfigKind::Video => self.selected_vid_preset.clone(),
        };
        let is_builder_preview = kind == ConfigKind::Autoexec && sel_name == MODE_BUILDER;
        let interactive = !is_builder_preview && !sel_name.is_empty();
        let can_revert = interactive
            && match kind {
                ConfigKind::GameInfo => {
                    self.gi_editor_dirty && self.cache.read().gi_texts.contains_key(&sel_name)
                }
                ConfigKind::Autoexec => {
                    self.ae_editor_dirty && self.cache.read().ae_texts.contains_key(&sel_name)
                }
                ConfigKind::Video => {
                    self.vid_editor_dirty && self.cache.read().vid_texts.contains_key(&sel_name)
                }
            };

        let preset_names: Vec<String> = match kind {
            ConfigKind::GameInfo => self.gi_presets.iter().map(|p| p.name.clone()).collect(),
            ConfigKind::Autoexec => self.ae_presets.iter().map(|p| p.name.clone()).collect(),
            ConfigKind::Video => self.vid_presets.iter().map(|p| p.name.clone()).collect(),
        };

        let max_win_size = (ctx.screen_rect().size() - egui::vec2(28.0, 28.0))
            .max(egui::vec2(640.0, 440.0));
        let title = format!(
            "Редактор конфигов • {} — {}",
            kind.display_name(),
            if sel_name.is_empty() {
                "нет пресета"
            } else {
                sel_name.as_str()
            }
        );

        let mut switch_kind: Option<ConfigKind> = None;
        let mut switch_preset: Option<String> = None;
        let mut save_action: Option<bool> = None; // Some(false) = сохранить, Some(true) = сохранить и применить
        let mut want_revert = false;
        let mut pending_status: Option<(bool, String)> = None;
        let mut pending_sfx: Option<UiSound> = None;

        // Глобальные горячие клавиши внутри модального редактора
        let (ctrl_f, ctrl_s, ctrl_shift_s, f3_next, f3_prev) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::F),
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::S),
                !i.modifiers.shift && i.key_pressed(egui::Key::F3),
                i.modifiers.shift && i.key_pressed(egui::Key::F3),
            )
        });
        if ctrl_f {
            self.editor_ui.focus_search_box = true;
        }
        if ctrl_shift_s && interactive {
            save_action = Some(true);
        } else if ctrl_s && interactive {
            save_action = Some(false);
        }

        // В режиме Конструктора переиспользуем кэшированный предпросмотр без сборки строки на каждом кадре
        let mut builder_text_buf = if is_builder_preview {
            self.cached_ae_preview.clone()
        } else {
            String::new()
        };

        egui::Window::new(title)
            .id(egui::Id::new("unified_config_editor_modal"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([980.0, 640.0])
            .min_size([620.0, 440.0])
            .max_size(max_win_size)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                // Синхронизируем кэш разбора и совпадений с актуальным буфером
                let (buf_ref, dirty_flag): (&mut String, &mut bool) = if is_builder_preview {
                    // В режиме Конструктора используется временный read-only буфер
                    (
                        &mut builder_text_buf,
                        &mut self.ae_editor_dirty,
                    )
                } else {
                    match kind {
                        ConfigKind::GameInfo => (&mut self.gi_editor_text, &mut self.gi_editor_dirty),
                        ConfigKind::Autoexec => (&mut self.ae_editor_text, &mut self.ae_editor_dirty),
                        ConfigKind::Video => (&mut self.vid_editor_text, &mut self.vid_editor_dirty),
                    }
                };

                Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);

                if !self.editor_ui.cached_matches.is_empty() {
                    if f3_next {
                        let next = (self.editor_ui.active_match + 1)
                            % self.editor_ui.cached_matches.len();
                        Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, next);
                    } else if f3_prev {
                        let len = self.editor_ui.cached_matches.len();
                        let prev = (self.editor_ui.active_match + len - 1) % len;
                        Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, prev);
                    }
                }

                // ---- Верхняя панель 1: выбор конфига, пресета, режима (Код / Команды) и Справочника
                ui.horizontal_wrapped(|ui| {
                    for k in [ConfigKind::GameInfo, ConfigKind::Autoexec, ConfigKind::Video] {
                        let r = theme::deco_chip(ui, k == kind, k.display_name(), pal);
                        if r.clicked() && k != kind {
                            switch_kind = Some(k);
                        }
                    }

                    ui.add_space(6.0);
                    ui.label(theme::mono_text("Пресет:", 11.5, theme::TEXT_MUTED));
                    egui::ComboBox::from_id_salt(("cfg_ed_preset_combo", kind))
                        .width(175.0)
                        .selected_text(if sel_name.is_empty() {
                            "—"
                        } else {
                            sel_name.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for p in &preset_names {
                                let label = if p == MODE_BUILDER {
                                    "Конструктор (только чтение)".to_owned()
                                } else {
                                    p.clone()
                                };
                                if ui.selectable_label(*p == sel_name, label).clicked() && *p != sel_name {
                                    switch_preset = Some(p.clone());
                                }
                            }
                        });

                    ui.add_space(6.0);
                    if theme::deco_chip(
                        ui,
                        self.editor_ui.view_mode == EditorViewMode::Code,
                        "Код",
                        pal,
                    )
                    .clicked()
                    {
                        self.editor_ui.view_mode = EditorViewMode::Code;
                    }
                    let cmd_label = format!("Команды ({})", self.editor_ui.cached_entries.len());
                    if theme::deco_chip(
                        ui,
                        self.editor_ui.view_mode == EditorViewMode::Commands,
                        &cmd_label,
                        pal,
                    )
                    .on_hover_text("Удобное визуальное редактирование параметров, значений и комментариев")
                    .clicked()
                    {
                        self.editor_ui.view_mode = EditorViewMode::Commands;
                    }

                    ui.add_space(6.0);
                    let cat_btn = theme::deco_chip(
                        ui,
                        self.editor_ui.catalog_open,
                        "Справочник команд",
                        pal,
                    )
                    .on_hover_text("Встроенная база популярных команд Deadlock с описанием на русском");
                    if cat_btn.clicked() {
                        self.editor_ui.catalog_open = !self.editor_ui.catalog_open;
                    }
                });

                ui.add_space(6.0);

                // ---- Панель 2: Поиск и Замена (Ctrl+F)
                theme::card_frame(ui, pal, false, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let search_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.editor_ui.search_query)
                                .id_salt("cfg_ed_search_input")
                                .desired_width(230.0)
                                .hint_text("Поиск команды, значения или описания (Ctrl+F)…"),
                        );
                        if self.editor_ui.focus_search_box {
                            search_resp.request_focus();
                            self.editor_ui.focus_search_box = false;
                        }

                        if search_resp.changed() {
                            Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                            self.editor_ui.active_match = 0;
                            if self.editor_ui.view_mode == EditorViewMode::Code
                                && !self.editor_ui.cached_matches.is_empty()
                            {
                                Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, 0);
                            }
                        }

                        if !self.editor_ui.search_query.is_empty()
                            && search_resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            search_resp.request_focus();
                            if !self.editor_ui.cached_matches.is_empty() {
                                let len = self.editor_ui.cached_matches.len();
                                let back = ui.input(|i| i.modifiers.shift);
                                let next = if back {
                                    (self.editor_ui.active_match + len - 1) % len
                                } else {
                                    (self.editor_ui.active_match + 1) % len
                                };
                                Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, next);
                            }
                        }

                        if !self.editor_ui.search_query.is_empty()
                            && ui
                                .small_button("×")
                                .on_hover_text("Очистить поиск (Esc)")
                                .clicked()
                        {
                            self.editor_ui.search_query.clear();
                            Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                        }

                        let match_total = self.editor_ui.cached_matches.len();
                        let counter_txt = if self.editor_ui.search_query.is_empty() {
                            "0 / 0".to_owned()
                        } else if match_total == 0 {
                            "Не найдено".to_owned()
                        } else {
                            format!("{} / {}", self.editor_ui.active_match + 1, match_total)
                        };
                        let counter_color = if !self.editor_ui.search_query.is_empty() && match_total == 0 {
                            theme::CRIMSON
                        } else if match_total > 0 {
                            pal.accent_light
                        } else {
                            theme::TEXT_MUTED
                        };
                        ui.label(theme::mono_text(counter_txt, 11.5, counter_color));

                        if ui
                            .add_enabled(match_total > 0, egui::Button::new("▲"))
                            .on_hover_text("Предыдущее совпадение (Shift+F3)")
                            .clicked()
                        {
                            self.editor_ui.view_mode = EditorViewMode::Code;
                            let prev = (self.editor_ui.active_match + match_total - 1) % match_total;
                            Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, prev);
                        }
                        if ui
                            .add_enabled(match_total > 0, egui::Button::new("▼"))
                            .on_hover_text("Следующее совпадение (F3 / Enter)")
                            .clicked()
                        {
                            self.editor_ui.view_mode = EditorViewMode::Code;
                            let next = (self.editor_ui.active_match + 1) % match_total;
                            Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, next);
                        }

                        if theme::deco_chip(ui, self.editor_ui.case_sensitive, "Аа", pal)
                            .on_hover_text("Учитывать регистр букв при поиске")
                            .clicked()
                        {
                            self.editor_ui.case_sensitive = !self.editor_ui.case_sensitive;
                            Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                            if self.editor_ui.view_mode == EditorViewMode::Code
                                && !self.editor_ui.cached_matches.is_empty()
                            {
                                let active_idx = self.editor_ui.active_match;
                                Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, active_idx);
                            }
                        }

                        if interactive
                            && theme::deco_chip(ui, self.editor_ui.show_replace, "Замена", pal)
                                .on_hover_text("Показать панель замены текста")
                                .clicked()
                        {
                            self.editor_ui.show_replace = !self.editor_ui.show_replace;
                        }
                    });

                    if interactive && self.editor_ui.show_replace {
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::mono_text("Заменить на:", 11.5, theme::TEXT_MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.editor_ui.replace_query)
                                    .id_salt("cfg_ed_replace_input")
                                    .desired_width(210.0)
                                    .hint_text("Новое значение или текст…"),
                            );
                            let can_rep = !self.editor_ui.search_query.is_empty()
                                && !self.editor_ui.cached_matches.is_empty();
                            if ui
                                .add_enabled(can_rep, egui::Button::new("Заменить"))
                                .on_hover_text("Заменить текущее выделенное совпадение")
                                .clicked()
                            {
                                if let Some(m) = self
                                    .editor_ui
                                    .cached_matches
                                    .get(self.editor_ui.active_match)
                                    .copied()
                                {
                                    *buf_ref = config_editor::replace_match(
                                        buf_ref,
                                        m,
                                        &self.editor_ui.replace_query,
                                    );
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                    Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                                    if !self.editor_ui.cached_matches.is_empty() {
                                        let next_i = self
                                            .editor_ui
                                            .active_match
                                            .min(self.editor_ui.cached_matches.len() - 1);
                                        Self::jump_editor_to_match(
                                            &mut self.editor_ui,
                                            buf_ref,
                                            next_i,
                                        );
                                    }
                                }
                            }
                            if ui
                                .add_enabled(can_rep, egui::Button::new("Заменить всё"))
                                .on_hover_text("Заменить все найденные совпадения в файле")
                                .clicked()
                            {
                                let (updated, cnt) = config_editor::replace_all_matches(
                                    buf_ref,
                                    &self.editor_ui.search_query,
                                    &self.editor_ui.replace_query,
                                    self.editor_ui.case_sensitive,
                                );
                                if cnt > 0 {
                                    *buf_ref = updated;
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                    Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                                    pending_status = Some((true, format!("Заменено вхождений: {cnt}")));
                                }
                            }
                        });
                    }
                });

                if is_builder_preview {
                    ui.add_space(6.0);
                    theme::card_frame(ui, pal, true, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(
                                    "Профиль «Конструктор» собирается автоматически и открыт только для чтения.",
                                )
                                .size(12.0)
                                .color(pal.accent_light),
                            );
                            if let Some(first_ae) = self
                                .ae_presets
                                .iter()
                                .find(|p| p.name != MODE_BUILDER)
                                .map(|p| p.name.clone())
                            {
                                let label = format!("Редактировать «{first_ae}»");
                                if theme::brass_secondary_button(
                                    ui,
                                    Some(Icon::Edit),
                                    &label,
                                    egui::vec2(0.0, 24.0),
                                    true,
                                    pal,
                                )
                                .clicked()
                                {
                                    switch_preset = Some(first_ae);
                                }
                            }
                        });
                    });
                }

                ui.add_space(8.0);

                // ---- Основная двухпанельная область: (Код / Команды) + (Справочник команд)
                let total_w = ui.available_width();
                let show_cat = self.editor_ui.catalog_open && total_w >= 700.0;
                let cat_w = if show_cat { 320.0_f32 } else { 0.0_f32 };
                let left_w = if show_cat {
                    (total_w - cat_w - 14.0).max(310.0)
                } else {
                    total_w
                };
                let body_max_h = (ui.available_height() - 8.0).max(260.0);
                let body_top_y = ui.cursor().min.y;

                ui.horizontal_top(|ui| {
                    // ЛЕВАЯ КОЛОНКА: Редактор кода или Режим команд
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        ui.set_max_width(left_w);

                        match self.editor_ui.view_mode {
                            EditorViewMode::Code => {
                                let hl_mode = match kind {
                                    ConfigKind::GameInfo | ConfigKind::Video => HighlightMode::Vdf,
                                    ConfigKind::Autoexec => HighlightMode::Cfg,
                                };
                                let id_salt = match kind {
                                    ConfigKind::GameInfo => "modal_gi_editor",
                                    ConfigKind::Autoexec => {
                                        if is_builder_preview {
                                            "modal_ae_builder"
                                        } else {
                                            "modal_ae_editor"
                                        }
                                    }
                                    ConfigKind::Video => "modal_vid_editor",
                                };
                                let pending_scroll = self.editor_ui.scroll_to_line.take();
                                let pending_select = self.editor_ui.select_char_range.take();
                                let active_m = if self.editor_ui.cached_matches.is_empty() {
                                    None
                                } else {
                                    Some(self.editor_ui.active_match)
                                };
                                let search_sig = {
                                    use std::hash::{Hash, Hasher};
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    self.editor_ui.search_query.hash(&mut h);
                                    self.editor_ui.case_sensitive.hash(&mut h);
                                    active_m.hash(&mut h);
                                    self.editor_ui.cached_matches.len().hash(&mut h);
                                    h.finish()
                                };
                                let opts = theme::CodeEditorOptions {
                                    search: theme::CodeEditorSearch {
                                        matches: &self.editor_ui.cached_matches,
                                        active_match: active_m,
                                        scroll_to_line: pending_scroll,
                                        select_char_range: pending_select,
                                    },
                                    brace_check: self.editor_ui.cached_brace_check,
                                    can_revert,
                                    line_count: Some(self.editor_ui.cached_line_count),
                                    galley_cache: Some(&mut self.editor_ui.galley_cache),
                                    galley_search_sig: search_sig,
                                };
                                let display_title = if sel_name.is_empty() {
                                    kind.display_name()
                                } else {
                                    sel_name.as_str()
                                };
                                let is_dirty_display = !is_builder_preview && *dirty_flag;
                                let code_scroll_h = (body_max_h
                                    - if is_builder_preview { 54.0 } else { 90.0 })
                                .max(180.0);
                                let (hdr, changed) = theme::code_editor_manuscript(
                                    ui,
                                    id_salt,
                                    display_title,
                                    buf_ref,
                                    interactive,
                                    20,
                                    code_scroll_h,
                                    self.s.editor_font_size,
                                    hl_mode,
                                    is_dirty_display,
                                    !is_builder_preview,
                                    interactive,
                                    None,
                                    opts,
                                    pal,
                                );
                                if changed && interactive {
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                }
                                if hdr.font_smaller {
                                    self.s.editor_font_size =
                                        (self.s.editor_font_size - 1.0).clamp(11.0, 18.0);
                                }
                                if hdr.font_larger {
                                    self.s.editor_font_size =
                                        (self.s.editor_font_size + 1.0).clamp(11.0, 18.0);
                                }
                                if let Some(err_line) = hdr.jump_to_brace_error {
                                    self.editor_ui.scroll_to_line = Some(err_line);
                                }
                                if hdr.revert {
                                    want_revert = true;
                                }
                                if hdr.save {
                                    save_action = Some(false);
                                } else if hdr.save_and_apply {
                                    save_action = Some(true);
                                }
                            }
                            EditorViewMode::Commands => {
                                // Панель действий для режима «Команды»: 2 чистые строки без наложений
                                theme::card_frame(ui, pal, false, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if theme::deco_chip(
                                            ui,
                                            self.editor_ui.cmd_only_active,
                                            "Только активные",
                                            pal,
                                        )
                                        .on_hover_text("Скрыть закомментированные (//) строки")
                                        .clicked()
                                        {
                                            self.editor_ui.cmd_only_active =
                                                !self.editor_ui.cmd_only_active;
                                        }

                                        if let Some(bc) = self.editor_ui.cached_brace_check {
                                            if bc.balanced {
                                                theme::badge_pill(
                                                    ui,
                                                    "{} OK",
                                                    theme::SOUL_TEAL,
                                                    false,
                                                );
                                            } else {
                                                theme::badge_pill(
                                                    ui,
                                                    "⚠ ОШИБКА СКОБОК",
                                                    theme::CRIMSON,
                                                    false,
                                                );
                                            }
                                        }
                                    });

                                    if !is_builder_preview {
                                        ui.add_space(6.0);
                                        ui.horizontal_wrapped(|ui| {
                                            if theme::brass_primary_button(
                                                ui,
                                                Some(Icon::Save),
                                                "СОХРАНИТЬ И ПРИМЕНИТЬ",
                                                egui::vec2(0.0, 26.0),
                                                interactive,
                                                pal,
                                            )
                                            .clicked()
                                            {
                                                save_action = Some(true);
                                            }
                                            if theme::brass_secondary_button(
                                                ui,
                                                Some(Icon::Save),
                                                "СОХРАНИТЬ",
                                                egui::vec2(0.0, 26.0),
                                                interactive,
                                                pal,
                                            )
                                            .clicked()
                                            {
                                                save_action = Some(false);
                                            }
                                            if can_revert
                                                && theme::brass_secondary_button(
                                                    ui,
                                                    Some(Icon::Restore),
                                                    "ОТКАТИТЬ",
                                                    egui::vec2(0.0, 26.0),
                                                    true,
                                                    pal,
                                                )
                                                .clicked()
                                            {
                                                want_revert = true;
                                            }
                                        });
                                    }
                                });

                                ui.add_space(6.0);
                                Self::sync_filtered_entries(&mut self.editor_ui, kind);
                                let mut pending_val_edit: Option<(usize, String)> = None;
                                let mut pending_toggle_comment: Option<usize> = None;
                                let mut pending_jump_line: Option<usize> = None;
                                let cmd_used = (ui.cursor().min.y - body_top_y).max(0.0);
                                let cmd_scroll_h = (body_max_h - cmd_used - 6.0).max(150.0);
                                let total_filtered = self.editor_ui.cached_filtered_entries.len();

                                if total_filtered == 0 {
                                    ui.add_space(18.0);
                                    ui.label(
                                        RichText::new(
                                            "Ни одной команды не найдено. Очистите поиск или добавьте команду из Справочника справа.",
                                        )
                                        .size(12.5)
                                        .color(theme::TEXT_MUTED),
                                    );
                                } else {
                                    // Виртуализированный список: рисуем только видимые строки из row_range
                                    egui::ScrollArea::vertical()
                                        .id_salt(("cfg_ed_commands_scroll", kind))
                                        .max_height(cmd_scroll_h)
                                        .show_rows(ui, 84.0, total_filtered, |ui, row_range| {
                                            if let Some(visible_indices) =
                                                self.editor_ui.cached_filtered_entries.get(row_range)
                                            {
                                                for &entry_idx in visible_indices {
                                                    let Some(entry) =
                                                        self.editor_ui.cached_entries.get(entry_idx)
                                                    else {
                                                        continue;
                                                    };
                                                    let catalog_info = config_editor::describe_command(
                                                        kind,
                                                        &entry.key,
                                                    );

                                                    egui::Frame::new()
                                                        .fill(if entry.commented {
                                                            theme::BG_MANUSCRIPT
                                                        } else {
                                                            theme::SURFACE_CARD
                                                        })
                                                        .stroke(egui::Stroke::new(
                                                            1.0_f32,
                                                            theme::BORDER_SUBTLE,
                                                        ))
                                                        .corner_radius(2)
                                                        .inner_margin(egui::Margin::symmetric(10, 6))
                                                        .show(ui, |ui| {
                                                            ui.set_min_width(ui.available_width());
                                                            ui.set_min_height(72.0);
                                                            ui.set_max_height(72.0);

                                                            // Строка 1: переключатель ВКЛ/ВЫКЛ, имя команды и раздел
                                                            ui.horizontal(|ui| {
                                                                let (cmt_txt, cmt_col) =
                                                                    if entry.commented {
                                                                        ("ВЫКЛ //", theme::TEXT_MUTED)
                                                                    } else {
                                                                        ("ВКЛ", theme::SOUL_TEAL)
                                                                    };
                                                                if ui
                                                                    .add_enabled(
                                                                        interactive,
                                                                        egui::Button::new(
                                                                            theme::mono_text(
                                                                                cmt_txt,
                                                                                10.5,
                                                                                cmt_col,
                                                                            ),
                                                                        ),
                                                                    )
                                                                    .on_hover_text(if entry.commented {
                                                                        "Раскомментировать строку"
                                                                    } else {
                                                                        "Закомментировать строку (//)"
                                                                    })
                                                                    .clicked()
                                                                {
                                                                    pending_toggle_comment =
                                                                        Some(entry.line_idx);
                                                                }

                                                                let mut key_rt = theme::mono_text(
                                                                    &entry.key,
                                                                    12.5,
                                                                    if entry.commented {
                                                                        theme::TEXT_MUTED
                                                                    } else {
                                                                        pal.accent_light
                                                                    },
                                                                );
                                                                if entry.commented {
                                                                    key_rt = key_rt.strikethrough();
                                                                }
                                                                ui.add(
                                                                    egui::Label::new(key_rt)
                                                                        .truncate(),
                                                                );
                                                                ui.add(
                                                                    egui::Label::new(
                                                                        theme::mono_text(
                                                                            format!(
                                                                                "• {}",
                                                                                entry.section
                                                                            ),
                                                                            10.0,
                                                                            theme::TEXT_MUTED,
                                                                        ),
                                                                    )
                                                                    .truncate(),
                                                                );
                                                            });

                                                            // Строка 2: комментарий или описание из справочника
                                                            let hint_line = entry
                                                                .inline_comment
                                                                .as_deref()
                                                                .or_else(|| {
                                                                    catalog_info
                                                                        .map(|c| c.summary_ru)
                                                                });
                                                            if let Some(desc) = hint_line {
                                                                ui.add(
                                                                    egui::Label::new(
                                                                        RichText::new(desc)
                                                                            .size(11.0)
                                                                            .color(theme::TEXT_MUTED),
                                                                    )
                                                                    .truncate(),
                                                                )
                                                                .on_hover_text(desc);
                                                            } else {
                                                                ui.add_space(15.0);
                                                            }

                                                            // Строка 3: поле значения, быстрое переключение и переход к строке
                                                            ui.add_space(3.0);
                                                            ui.horizontal(|ui| {
                                                                ui.label(theme::mono_text(
                                                                    "Значение:",
                                                                    11.0,
                                                                    theme::TEXT_MUTED,
                                                                ));

                                                                if interactive {
                                                                    let mut val_buf =
                                                                        entry.value.clone();
                                                                    let ed = egui::TextEdit::singleline(
                                                                        &mut val_buf,
                                                                    )
                                                                    .id_salt((
                                                                        "cmd_val",
                                                                        kind,
                                                                        entry.line_idx,
                                                                    ))
                                                                    .font(egui::FontId::monospace(
                                                                        12.0,
                                                                    ))
                                                                    .desired_width(150.0);
                                                                    if ui.add(ed).changed() {
                                                                        pending_val_edit = Some((
                                                                            entry.line_idx,
                                                                            val_buf,
                                                                        ));
                                                                    }

                                                                    let trimmed_v =
                                                                        entry.value.trim();
                                                                    let toggled = if trimmed_v
                                                                        == "0"
                                                                    {
                                                                        Some("1")
                                                                    } else if trimmed_v == "1" {
                                                                        Some("0")
                                                                    } else if trimmed_v
                                                                        .eq_ignore_ascii_case(
                                                                            "true",
                                                                        )
                                                                    {
                                                                        Some("false")
                                                                    } else if trimmed_v
                                                                        .eq_ignore_ascii_case(
                                                                            "false",
                                                                        )
                                                                    {
                                                                        Some("true")
                                                                    } else {
                                                                        None
                                                                    };
                                                                    if let Some(next_v) = toggled {
                                                                        if ui
                                                                            .small_button("⇄")
                                                                            .on_hover_text(format!(
                                                                                "Переключить на «{next_v}»"
                                                                            ))
                                                                            .clicked()
                                                                        {
                                                                            pending_val_edit =
                                                                                Some((
                                                                                    entry.line_idx,
                                                                                    next_v
                                                                                        .to_owned(),
                                                                                ));
                                                                        }
                                                                    }
                                                                } else {
                                                                    ui.label(theme::mono_text(
                                                                        &entry.value,
                                                                        12.0,
                                                                        theme::SOUL_TEAL,
                                                                    ));
                                                                }

                                                                let line_lbl = format!(
                                                                    "стр. {}",
                                                                    entry.line_idx + 1
                                                                );
                                                                if ui
                                                                    .small_button(line_lbl)
                                                                    .on_hover_text(
                                                                        "Перейти к этой строке в коде",
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    pending_jump_line =
                                                                        Some(entry.line_idx);
                                                                }
                                                            });
                                                        });
                                                }
                                            }
                                        });
                                }

                                if let Some((line_idx, new_val)) = pending_val_edit {
                                    *buf_ref = config_editor::set_line_value(
                                        buf_ref,
                                        kind,
                                        line_idx,
                                        &new_val,
                                    );
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                }
                                if let Some(line_idx) = pending_toggle_comment {
                                    *buf_ref =
                                        config_editor::toggle_line_comment(buf_ref, line_idx);
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                }
                                if let Some(line_idx) = pending_jump_line {
                                    self.editor_ui.view_mode = EditorViewMode::Code;
                                    self.editor_ui.scroll_to_line = Some(line_idx);
                                }
                            }
                        }
                    });

                    // ПРАВАЯ КОЛОНКА: Встроенный справочник команд Deadlock
                    if show_cat {
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_width(cat_w);
                            ui.set_max_width(cat_w);

                            theme::card_frame(ui, pal, false, |ui| {
                                let cat_col_top = ui.cursor().min.y;
                                ui.label(theme::serif_heading(
                                    "СПРАВОЧНИК КОМАНД",
                                    12.5,
                                    pal.accent_light,
                                ));
                                ui.add_space(4.0);
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.editor_ui.catalog_filter)
                                        .id_salt("cfg_ed_cat_filter")
                                        .desired_width(ui.available_width())
                                        .hint_text("Фильтр по справочнику…"),
                                );
                                ui.add_space(5.0);

                                ui.horizontal_wrapped(|ui| {
                                    if theme::deco_chip(
                                        ui,
                                        self.editor_ui.catalog_category.is_empty(),
                                        "Все",
                                        pal,
                                    )
                                    .clicked()
                                    {
                                        self.editor_ui.catalog_category.clear();
                                    }
                                    for cat in config_editor::categories_for(kind) {
                                        if theme::deco_chip(
                                            ui,
                                            self.editor_ui.catalog_category == cat,
                                            cat,
                                            pal,
                                        )
                                        .clicked()
                                        {
                                            self.editor_ui.catalog_category = cat.to_owned();
                                        }
                                    }
                                });

                                ui.add_space(6.0);
                                let cat_q = self.editor_ui.catalog_filter.trim().to_lowercase();
                                let sel_cat = self.editor_ui.catalog_category.clone();
                                let mut insert_req: Option<(&'static str, &'static str, &'static str)> =
                                    None;
                                let mut find_req: Option<&'static str> = None;
                                let used_top = (ui.cursor().min.y - cat_col_top).max(0.0);
                                let footer_reserve = if interactive { 78.0 } else { 10.0 };
                                let cat_scroll_h =
                                    (body_max_h - used_top - footer_reserve - 16.0).max(120.0);

                                egui::ScrollArea::vertical()
                                    .id_salt(("cfg_ed_catalog_scroll", kind))
                                    .max_height(cat_scroll_h)
                                    .show(ui, |ui| {
                                        for item in config_editor::catalog_for(kind) {
                                            if !sel_cat.is_empty() && item.category != sel_cat {
                                                continue;
                                            }
                                            if !cat_q.is_empty()
                                                && !item.key.to_lowercase().contains(&cat_q)
                                                && !item.summary_ru.to_lowercase().contains(&cat_q)
                                                && !item.category.to_lowercase().contains(&cat_q)
                                            {
                                                continue;
                                            }

                                            let already_present = self
                                                .editor_ui
                                                .cached_present_keys
                                                .contains(&item.key.to_ascii_lowercase());

                                            egui::Frame::new()
                                                .fill(theme::SURFACE_ELEVATED)
                                                .stroke(egui::Stroke::new(
                                                    1.0_f32,
                                                    theme::BORDER_SUBTLE,
                                                ))
                                                .corner_radius(2)
                                                .inner_margin(egui::Margin::same(8))
                                                .show(ui, |ui| {
                                                    ui.set_min_width(ui.available_width());
                                                    ui.add(
                                                        egui::Label::new(theme::mono_text(
                                                            item.key,
                                                            12.0,
                                                            pal.accent_light,
                                                        ))
                                                        .wrap(),
                                                    );
                                                    let rec_line = if item.recommended_value.is_empty()
                                                    {
                                                        format!("Раздел: {}", item.target_section)
                                                    } else {
                                                        format!(
                                                            "Рек.: {} • Умолч.: {}",
                                                            item.recommended_value,
                                                            item.default_value
                                                        )
                                                    };
                                                    ui.add(
                                                        egui::Label::new(theme::mono_text(
                                                            rec_line,
                                                            10.5,
                                                            theme::SOUL_TEAL,
                                                        ))
                                                        .wrap(),
                                                    );
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(item.summary_ru)
                                                                .size(11.0)
                                                                .color(theme::TEXT_MUTED),
                                                        )
                                                        .wrap(),
                                                    );
                                                    ui.add_space(4.0);
                                                    ui.horizontal_wrapped(|ui| {
                                                        if already_present {
                                                            theme::badge_pill(
                                                                ui,
                                                                "В файле",
                                                                theme::SOUL_TEAL,
                                                                false,
                                                            );
                                                            if ui
                                                                .small_button("Найти")
                                                                .on_hover_text(
                                                                    "Перейти к этой команде в файле",
                                                                )
                                                                .clicked()
                                                            {
                                                                find_req = Some(item.key);
                                                            }
                                                        } else if interactive
                                                            && ui
                                                                .small_button("+ Добавить")
                                                                .on_hover_text(format!(
                                                                    "Вставить в раздел {} со значением «{}»",
                                                                    item.target_section,
                                                                    item.recommended_value
                                                                ))
                                                                .clicked()
                                                        {
                                                            insert_req = Some((
                                                                item.target_section,
                                                                item.key,
                                                                item.recommended_value,
                                                            ));
                                                        }
                                                    });
                                                });
                                            ui.add_space(5.0);
                                        }
                                    });

                                if let Some(key) = find_req {
                                    self.editor_ui.search_query = key.to_owned();
                                    Self::sync_editor_cache(&mut self.editor_ui, kind, buf_ref);
                                    if !self.editor_ui.cached_matches.is_empty() {
                                        Self::jump_editor_to_match(&mut self.editor_ui, buf_ref, 0);
                                    }
                                    pending_sfx = Some(UiSound::Click);
                                }
                                if let Some((sec, key, val)) = insert_req {
                                    let (new_text, line_idx) = config_editor::insert_command_smart(
                                        buf_ref,
                                        kind,
                                        sec,
                                        key,
                                        val,
                                        None,
                                    );
                                    *buf_ref = new_text;
                                    *dirty_flag = true;
                                    self.editor_ui.text_rev =
                                        self.editor_ui.text_rev.wrapping_add(1);
                                    self.editor_ui.scroll_to_line = Some(line_idx);
                                    pending_sfx = Some(UiSound::Success);
                                    pending_status =
                                        Some((true, format!("Команда «{key}» добавлена")));
                                }

                                // Быстрое добавление произвольной команды
                                if interactive {
                                    ui.add_space(6.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                    ui.label(theme::mono_text(
                                        "Своя команда:",
                                        11.0,
                                        theme::TEXT_MUTED,
                                    ));
                                    ui.horizontal(|ui| {
                                        let row_w = ui.available_width();
                                        let key_w = (row_w * 0.50).max(80.0);
                                        let val_w = (row_w - key_w - 42.0).max(52.0);
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.editor_ui.custom_key,
                                            )
                                            .id_salt("cfg_ed_custom_key")
                                            .desired_width(key_w)
                                            .hint_text("команда"),
                                        );
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.editor_ui.custom_value,
                                            )
                                            .id_salt("cfg_ed_custom_val")
                                            .desired_width(val_w)
                                            .hint_text("знач."),
                                        );
                                        let can_add =
                                            !self.editor_ui.custom_key.trim().is_empty();
                                        if ui
                                            .add_enabled(can_add, egui::Button::new("+"))
                                            .on_hover_text("Добавить свою команду в конфиг")
                                            .clicked()
                                        {
                                            let k = self.editor_ui.custom_key.trim().to_owned();
                                            let v = self.editor_ui.custom_value.trim().to_owned();
                                            let def_sec = match kind {
                                                ConfigKind::GameInfo => "ConVars",
                                                ConfigKind::Video => "video.cfg",
                                                ConfigKind::Autoexec => "Autoexec",
                                            };
                                            let (new_text, line_idx) =
                                                config_editor::insert_command_smart(
                                                    buf_ref,
                                                    kind,
                                                    def_sec,
                                                    &k,
                                                    &v,
                                                    None,
                                                );
                                            *buf_ref = new_text;
                                            *dirty_flag = true;
                                            self.editor_ui.text_rev =
                                                self.editor_ui.text_rev.wrapping_add(1);
                                            self.editor_ui.scroll_to_line = Some(line_idx);
                                            self.editor_ui.custom_key.clear();
                                            self.editor_ui.custom_value.clear();
                                            pending_sfx = Some(UiSound::Success);
                                            pending_status =
                                                Some((true, format!("Команда «{k}» добавлена")));
                                        }
                                    });
                                }
                            });
                        });
                    }
                });
            });

        if let Some(snd) = pending_sfx {
            self.sfx(snd);
        }
        if let Some((ok, msg)) = pending_status {
            self.set_status(ok, msg);
        }
        if want_revert {
            self.revert_editor_preset(kind, &sel_name);
        }
        if let Some(and_apply) = save_action {
            self.save_editor_preset(kind, sel_name, and_apply);
        }
        if let Some(next_preset) = switch_preset {
            self.sfx(UiSound::Click);
            self.open_code_editor(kind, &next_preset);
        } else if let Some(next_kind) = switch_kind {
            self.sfx(UiSound::Click);
            let target = self.preferred_editor_preset(next_kind);
            self.open_code_editor(next_kind, &target);
        }

        if !open {
            self.code_modal = None;
        }
    }

    fn ui_modal_new_preset(&mut self, ctx: &egui::Context, pal: &Palette) {
        let mut open = true;
        let game = PathBuf::from(&self.s.game_path);
        let steam_path = PathBuf::from(&self.s.steam_path);
        let bg_busy = self.is_bg_busy();

        egui::Window::new("Новый пресет")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(470.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (idx, lbl) in [
                        (0u8, "Из текущей игры"),
                        (1, "Файл с ПК"),
                        (2, "По ссылке"),
                    ] {
                        if theme::deco_chip(ui, self.new_preset_tab == idx, lbl, pal).clicked() {
                            self.new_preset_tab = idx;
                        }
                    }
                });
                ui.add_space(12.0);

                match self.new_preset_tab {
                    0 => {
                        ui.label("Сохранить текущий файл gameinfo.gi из папки Deadlock как новый пресет:");
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_gi_preset_name)
                                .hint_text("Например: Мой конфиг")
                                .desired_width(ui.available_width()),
                        );
                        ui.add_space(12.0);
                        if theme::brass_primary_button(
                            ui,
                            Some(Icon::Plus),
                            "Создать",
                            egui::vec2(140.0, 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            self.send_disk(DiskCmd::CreateGiFromCurrent {
                                game_path: game.clone(),
                                steam_path: steam_path.clone(),
                                name: self.new_gi_preset_name.clone(),
                            });
                        }
                    }
                    1 => {
                        ui.label("Выберите готовый файл .gi или .txt на компьютере:");
                        ui.add_space(12.0);
                        if theme::brass_primary_button(
                            ui,
                            Some(Icon::Import),
                            "Выбрать файл…",
                            egui::vec2(170.0, 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("GameInfo", &["gi", "txt"])
                                .pick_file()
                            {
                                self.send_disk(DiskCmd::ImportGiFile {
                                    src: path,
                                    steam_path: steam_path.clone(),
                                    game_path: game.clone(),
                                });
                            }
                        }
                    }
                    _ => {
                        ui.label("Прямая ссылка на конфиг (например, GitHub Gist Raw):");
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.url_gi_input)
                                .hint_text("https://gist.githubusercontent.com/.../raw/gameinfo.gi")
                                .desired_width(ui.available_width()),
                        );
                        ui.add_space(8.0);
                        ui.label("Название (необязательно):");
                        ui.add_space(4.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.url_gi_name)
                                .hint_text("Конфиг друга")
                                .desired_width(ui.available_width()),
                        );
                        ui.add_space(12.0);
                        if theme::brass_primary_button(
                            ui,
                            Some(Icon::Globe),
                            "Скачать и добавить",
                            egui::vec2(190.0, 32.0),
                            !bg_busy,
                            pal,
                        )
                        .clicked()
                        {
                            let url = self.url_gi_input.clone();
                            let name = self.url_gi_name.clone();
                            self.send_heavy(
                                "Загрузка пресета по ссылке",
                                HeavyCmd::ImportGiUrl {
                                    url,
                                    name,
                                    steam_path,
                                    game_path: game,
                                },
                            );
                        }
                    }
                }
            });

        if !open {
            self.new_preset_modal_open = false;
        }
    }

    fn ui_modal_new_config(&mut self, ctx: &egui::Context, sub: ConfigSubTab, pal: &Palette) {
        let mut open = true;
        let game = PathBuf::from(&self.s.game_path);
        let steam_path = PathBuf::from(&self.s.steam_path);
        let title = match sub {
            ConfigSubTab::Autoexec => "Новый профиль autoexec.cfg",
            ConfigSubTab::Video => "Новый профиль video.txt",
        };

        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(450.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                match sub {
                    ConfigSubTab::Autoexec => {
                        ui.label("Сохранить текущий autoexec.cfg из папки игры как новый профиль:");
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_ae_preset_name)
                                .hint_text("Например: Мой autoexec")
                                .desired_width(ui.available_width()),
                        );
                        ui.add_space(10.0);
                        if theme::brass_primary_button(
                            ui,
                            Some(Icon::Plus),
                            "Сохранить из текущей игры",
                            egui::vec2(230.0, 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            self.send_disk(DiskCmd::CreateAeFromCurrent {
                                game_path: game.clone(),
                                name: self.new_ae_preset_name.clone(),
                            });
                        }

                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label("Или загрузите готовый файл .cfg / .txt с компьютера:");
                        ui.add_space(8.0);
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Import),
                            "Импортировать файл с ПК (.cfg / .txt)…",
                            egui::vec2(ui.available_width(), 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Config", &["cfg", "txt"])
                                .pick_file()
                            {
                                self.send_disk(DiskCmd::ImportAeFile {
                                    src: path,
                                    game_path: game,
                                });
                            }
                        }
                    }
                    ConfigSubTab::Video => {
                        ui.label("Сохранить текущий video.txt игры как новый профиль графики:");
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_vid_preset_name)
                                .hint_text("Например: Мой video.txt")
                                .desired_width(ui.available_width()),
                        );
                        ui.add_space(10.0);
                        if theme::brass_primary_button(
                            ui,
                            Some(Icon::Plus),
                            "Сохранить из текущей игры",
                            egui::vec2(230.0, 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            self.send_disk(DiskCmd::CreateVidFromCurrent {
                                steam_path: steam_path.clone(),
                                game_path: game.clone(),
                                name: self.new_vid_preset_name.clone(),
                                apply_all: self.s.video_apply_all_accounts,
                            });
                        }

                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label("Или загрузите готовый файл video.txt с компьютера:");
                        ui.add_space(8.0);
                        if theme::brass_secondary_button(
                            ui,
                            Some(Icon::Import),
                            "Импортировать файл с ПК (.cfg / .txt)…",
                            egui::vec2(ui.available_width(), 32.0),
                            true,
                            pal,
                        )
                        .clicked()
                        {
                            self.sfx(UiSound::Click);
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Video config", &["txt", "cfg"])
                                .pick_file()
                            {
                                self.send_disk(DiskCmd::ImportVidFile {
                                    src: path,
                                    steam_path,
                                    game_path: game,
                                    apply_all: self.s.video_apply_all_accounts,
                                });
                            }
                        }
                    }
                }
            });

        if !open {
            self.new_cfg_modal = None;
        }
    }

    fn ui_modal_rename_preset(&mut self, ctx: &egui::Context, old_name: String, pal: &Palette) {
        let mut open = true;
        let game = PathBuf::from(&self.s.game_path);
        let steam_path = PathBuf::from(&self.s.steam_path);

        egui::Window::new("Переименовать пресет")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(380.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("Новое название:").color(theme::TEXT_CREAM));
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.rename_gi_name)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if theme::brass_primary_button(
                        ui,
                        Some(Icon::Check),
                        "Сохранить",
                        egui::vec2(130.0, 30.0),
                        true,
                        pal,
                    )
                    .clicked()
                    {
                        self.send_disk(DiskCmd::RenameGiPreset {
                            old_name: old_name.clone(),
                            new_name: self.rename_gi_name.clone(),
                            steam_path,
                            game_path: game,
                        });
                    }
                    if theme::brass_secondary_button(
                        ui,
                        None,
                        "Отмена",
                        egui::vec2(100.0, 30.0),
                        true,
                        pal,
                    )
                    .clicked()
                    {
                        self.rename_modal_target = None;
                    }
                });
            });

        if !open {
            self.rename_modal_target = None;
        }
    }

    fn ui_modal_gi_tweaks(&mut self, ctx: &egui::Context, pal: &Palette) {
        let mut open = true;
        let game = PathBuf::from(&self.s.game_path);
        let steam_path = PathBuf::from(&self.s.steam_path);
        let bg_busy = self.is_bg_busy();

        egui::Window::new("Твики профиля и резервные копии")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(540.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Угол обзора (FOV):");
                    for fov in FovPreset::ALL {
                        if theme::deco_chip(ui, self.s.gameinfo.fov_preset == fov, fov.label(), pal)
                            .clicked()
                        {
                            self.s.gameinfo.fov_preset = fov;
                        }
                    }
                });

                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.s.gameinfo.hide_hp_names,
                    "Скрыть имена над полосками здоровья",
                );
                ui.checkbox(
                    &mut self.s.gameinfo.bright_outlines,
                    "Яркая обводка героев",
                );
                ui.checkbox(
                    &mut self.s.gameinfo.no_flash_lights,
                    "Убрать вспышки выстрелов и свечение эффектов",
                );
                ui.checkbox(
                    &mut self.s.gameinfo.no_camera_shake,
                    "Убрать покачивание камеры и тряску в бою",
                );
                ui.checkbox(
                    &mut self.s.gameinfo.cpu_threads_boost,
                    "Ускорение обработки анимаций и шейдеров на CPU",
                );

                ui.add_space(8.0);
                egui::CollapsingHeader::new("Тонкие параметры изображения")
                    .id_salt("modal_gi_sliders")
                    .default_open(false)
                    .show(ui, |ui| {
                        let gi = &mut self.s.gameinfo;
                        ui.checkbox(&mut gi.use_quick_tweaks, "Использовать значения ниже поверх пресета");
                        ui.add_enabled_ui(gi.use_quick_tweaks, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Соотношение FOV:");
                                ui.add(
                                    egui::Slider::new(&mut gi.fov_aspect_ratio, 1.20..=3.50)
                                        .fixed_decimals(2),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Яркость (гамма):");
                                ui.add(
                                    egui::Slider::new(&mut gi.gamma, 0.80..=2.60)
                                        .fixed_decimals(2),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Память текстур (МБ):");
                                ui.add(egui::Slider::new(&mut gi.texture_pool_mb, 128..=2048));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Упрощение текстур:");
                                ui.add(egui::Slider::new(&mut gi.texture_mip_bias, 0..=16));
                            });
                            ui.checkbox(&mut gi.shadows_enabled, "Динамические тени");
                        });
                    });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if theme::brass_primary_button(
                        ui,
                        Some(Icon::Check),
                        "Применить к профилю",
                        egui::vec2(190.0, 32.0),
                        !self.s.gameinfo.active_preset.trim().is_empty(),
                        pal,
                    )
                    .clicked()
                    {
                        let target = self.s.gameinfo.active_preset.clone();
                        self.apply_gi_preset(&target);
                    }
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Refresh),
                        "Проверить патч",
                        egui::vec2(0.0, 32.0),
                        true,
                        pal,
                    )
                    .clicked()
                    {
                        self.check_gi_patch_now();
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Restore),
                        "Вернуть оригинал Valve",
                        egui::vec2(0.0, 26.0),
                        self.has_vanilla_bak,
                        pal,
                    )
                    .clicked()
                    {
                        self.send_disk(DiskCmd::RestoreGiVanilla {
                            game_path: game.clone(),
                            steam_path: steam_path.clone(),
                        });
                    }
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Globe),
                        "Скачать оригинал (SteamDB)",
                        egui::vec2(0.0, 26.0),
                        !bg_busy,
                        pal,
                    )
                    .clicked()
                    {
                        self.send_heavy(
                            "Загрузка оригинального файла Valve",
                            HeavyCmd::RefreshSteamDb {
                                game_path: game.clone(),
                                steam_path,
                            },
                        );
                    }
                    if theme::brass_secondary_button(
                        ui,
                        Some(Icon::Folder),
                        "Папка бэкапов",
                        egui::vec2(0.0, 26.0),
                        true,
                        pal,
                    )
                    .clicked()
                    {
                        self.open_dir(Settings::gameinfo_backups_dir(), true);
                    }
                });

                if !self.gi_backups.is_empty() {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(format!(
                        "История резервных копий ({})",
                        self.gi_backups.len()
                    ))
                    .id_salt("modal_gi_backups")
                    .default_open(false)
                    .show(ui, |ui| {
                        let mut restore_target: Option<PathBuf> = None;
                        egui::ScrollArea::vertical()
                            .max_height(130.0)
                            .show(ui, |ui| {
                                for b in &self.gi_backups {
                                    ui.horizontal(|ui| {
                                        ui.label(theme::mono_text(
                                            format!(
                                                "{} ({})",
                                                b.name,
                                                tools::format_size(b.size_bytes)
                                            ),
                                            11.5,
                                            theme::TEXT_CREAM,
                                        ));
                                        if theme::brass_secondary_button(
                                            ui,
                                            Some(Icon::Restore),
                                            "Восстановить",
                                            egui::vec2(0.0, 22.0),
                                            true,
                                            pal,
                                        )
                                        .clicked()
                                        {
                                            restore_target = Some(b.path.clone());
                                        }
                                    });
                                }
                            });
                        if let Some(bak_path) = restore_target {
                            self.send_disk(DiskCmd::RestoreGiBackup {
                                backup_path: bak_path,
                                game_path: game,
                                steam_path: PathBuf::from(&self.s.steam_path),
                            });
                        }
                    });
                }
            });

        if !open {
            self.gi_tweaks_modal_open = false;
        }
    }

    fn ui_modal_map_timers(&mut self, ctx: &egui::Context, pal: &Palette) {
        if self.map_timer.is_running() && !self.window_minimized.load(Ordering::Relaxed) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        let mut open = true;
        let elapsed = self.map_timer.elapsed_secs();

        egui::Window::new("Тайминги карты Deadlock")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(600.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(theme::BG_SLATE)
                    .stroke(egui::Stroke::new(1.2_f32, pal.accent))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                // Интерактивный таймер матча
                theme::card_frame(ui, pal, self.map_timer.is_running(), |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(theme::serif_heading(
                                "ВРЕМЯ МАТЧА",
                                10.5,
                                theme::TEXT_MUTED,
                            ));
                            ui.label(theme::mono_text(
                                map_timers::format_mm_ss(elapsed),
                                24.0,
                                if self.map_timer.is_running() {
                                    theme::SOUL_TEAL
                                } else {
                                    theme::TEXT_CREAM
                                },
                            ));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::brass_secondary_button(
                                ui,
                                Some(Icon::Restore),
                                "Сброс",
                                egui::vec2(0.0, 30.0),
                                true,
                                pal,
                            )
                            .clicked()
                            {
                                self.map_timer.reset();
                            }
                            if theme::brass_primary_button(
                                ui,
                                Some(Icon::Timer),
                                if self.map_timer.is_running() {
                                    "Пауза"
                                } else {
                                    "Старт"
                                },
                                egui::vec2(105.0, 30.0),
                                true,
                                pal,
                            )
                            .clicked()
                            {
                                self.map_timer.toggle();
                            }
                            for (lbl, delta) in
                                [("+1 мин", 60i64), ("+10с", 10), ("−10с", -10), ("−1 мин", -60)]
                            {
                                if theme::deco_chip(ui, false, lbl, pal).clicked() {
                                    self.map_timer.shift_secs(delta);
                                }
                            }
                        });
                    });
                });

                ui.add_space(12.0);
                egui::Grid::new("map_timers_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([14.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(theme::serif_heading("ОБЪЕКТ", 11.0, pal.accent_light));
                        ui.label(theme::serif_heading("РАСПИСАНИЕ", 11.0, pal.accent_light));
                        ui.label(theme::serif_heading("ДО СПАВНА", 11.0, pal.accent_light));
                        ui.end_row();

                        for obj in map_timers::OBJECTIVES {
                            let rem = map_timers::next_spawn_in_secs(
                                elapsed,
                                obj.first_spawn_sec,
                                obj.respawn_sec,
                            );
                            let soon = rem <= 30;
                            ui.label(
                                RichText::new(obj.name)
                                    .color(theme::TEXT_CREAM)
                                    .strong(),
                            )
                            .on_hover_text(format!("{} — {}", obj.category, obj.note));
                            ui.label(RichText::new(obj.schedule_hint).color(theme::TEXT_MUTED));
                            let rem_str = if rem == 0 {
                                "Сейчас!".to_owned()
                            } else {
                                map_timers::format_mm_ss(rem)
                            };
                            let rem_col = if soon {
                                theme::SOUL_TEAL
                            } else {
                                theme::TEXT_CREAM
                            };
                            ui.label(theme::mono_text(rem_str, 13.0, rem_col));
                            ui.end_row();
                        }
                    });
            });

        if !open {
            self.map_timers_modal_open = false;
        }
    }
}

impl eframe::App for LauncherApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Фирменный глубокий сланец (#111315) исключает чёрную вспышку при ресайзе и восстановлении окна.
        theme::BG_NOIR.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.startup_stage == StartupStage::SpawnDeferred {
            self.spawn_background_services(ctx);
            self.startup_stage = StartupStage::Ready;
        }
        self.poll_io_events();
        self.poll_updater_events();
        self.sync_from_cache_if_changed();

        if self.main_hwnd == 0 {
            if let Some(hwnd) = apply_native_window_icons_from_resource() {
                self.main_hwnd = hwnd;
                if self.tray.is_none() {
                    self.tray = TrayController::spawn(ctx.clone(), hwnd);
                }
            }
        }

        if self
            .tray
            .as_ref()
            .is_some_and(TrayController::take_pending_play)
        {
            self.play();
        }

        let restored_from_tray = self
            .tray
            .as_ref()
            .is_some_and(TrayController::take_pending_restore);
        if restored_from_tray {
            crate::tray::bring_window_to_front_ui_thread(self.main_hwnd);
            self.hidden_in_tray = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.restore_repaint_frames = 4;
            ctx.request_repaint();
            ctx.request_repaint_after(Duration::from_millis(80));
        }

        // Применяем изменения темы, масштаба (после отпускания мыши) и режима анимаций
        if self.s.theme != self.applied_theme {
            theme::apply_theme(ctx, self.s.theme);
            self.applied_theme = self.s.theme;
        }
        let clamped_scale = self.s.ui_scale.clamp(0.85, 1.25);
        if (clamped_scale - self.applied_ui_scale).abs() > 0.005
            && !ctx.input(|i| i.pointer.any_down())
        {
            ctx.set_zoom_factor(clamped_scale);
            self.applied_ui_scale = clamped_scale;
        }
        if self.s.animations_enabled != self.applied_animations {
            let anims = self.s.animations_enabled;
            ctx.style_mut(|st| {
                st.animation_time = if anims { theme::ANIM_DURATION_SEC } else { 0.0 };
            });
            self.applied_animations = anims;
        }

        let pal = self.palette();
        let game_up = self.is_game_running();
        let game_pid = self.game_pid();
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let effectively_hidden = if self.main_hwnd != 0 {
            crate::tray::is_window_really_hidden(self.main_hwnd)
        } else {
            minimized
        };
        let is_hidden_now = effectively_hidden && !restored_from_tray;
        self.window_minimized.store(is_hidden_now, Ordering::Relaxed);
        let allow_timers = !is_hidden_now && !minimized;

        // Если окно потеряло фокус ОС, снимаем фокус с полей ввода, чтобы мигающий курсор не пробуждал рендер
        if (!focused || !allow_timers) && !restored_from_tray {
            if let Some(focused_id) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(focused_id));
            }
        }

        let cur_booster_opts = (
            self.s.auto_high_priority,
            self.s.ram_trim_on_minimize,
            self.s.boost_timer_1ms,
            self.s.boost_power_plan,
            self.s.boost_disable_cpu0,
        );
        if self.last_booster_opts != cur_booster_opts {
            if let Some(ref m) = self.monitor {
                m.set_booster_options(
                    cur_booster_opts.0,
                    cur_booster_opts.1,
                    cur_booster_opts.2,
                    cur_booster_opts.3,
                    cur_booster_opts.4,
                );
                self.last_booster_opts = cur_booster_opts;
            }
        }
        if self.last_auto_english_layout != self.s.auto_english_layout {
            if let Some(ref m) = self.monitor {
                m.set_auto_english_layout(self.s.auto_english_layout);
            }
            self.last_auto_english_layout = self.s.auto_english_layout;
        }

        // При сворачивании окна прячем в трей без SW_HIDE (исключаем холостой спин eframe) и сбрасываем лишнюю память
        if !restored_from_tray
            && !self.was_minimized
            && minimized
            && self.restore_repaint_frames == 0
        {
            if self.s.minimize_to_tray && self.main_hwnd != 0 {
                crate::tray::hide_window_to_tray_ui_thread(self.main_hwnd);
                self.hidden_in_tray = true;
            }
            if self.s.ram_trim_on_minimize {
                game_booster::trim_memory();
                self.cached_ram_bytes = game_booster::current_working_set_bytes();
            }
            if let Some(ref m) = self.monitor {
                m.wake_now();
            }
        } else if !self.was_game_running && game_up {
            if self.s.ram_trim_on_minimize {
                game_booster::trim_memory();
                self.cached_ram_bytes = game_booster::current_working_set_bytes();
            }
            if let Some(ref m) = self.monitor {
                m.wake_now();
            }
        } else if (self.was_hidden && !effectively_hidden)
            || (self.was_minimized && !minimized)
            || (self.hidden_in_tray && !minimized)
        {
            if self.hidden_in_tray && self.main_hwnd != 0 {
                crate::tray::bring_window_to_front_ui_thread(self.main_hwnd);
                self.hidden_in_tray = false;
            }
            self.restore_repaint_frames = self.restore_repaint_frames.max(4);
            ctx.request_repaint();
            ctx.request_repaint_after(Duration::from_millis(80));
            if let Some(ref m) = self.monitor {
                m.wake_now();
            }
        }
        self.was_minimized = minimized;
        self.was_hidden = effectively_hidden;
        self.was_game_running = game_up;

        if self.last_rpc_sync.0 != self.s.discord_rpc
            || self.last_rpc_sync.1 != self.s.game_path
            || self.last_rpc_sync.2 != self.s.steam_path
            || self.last_rpc_sync.3 != self.s.gameinfo.active_preset
            || self.last_rpc_sync.4 != game_up
            || self.last_rpc_sync.5 != game_pid
        {
            if let Some(ref rpc) = self.discord_rpc {
                rpc.sync(&self.s, game_up, game_pid);
                self.last_rpc_sync = (
                    self.s.discord_rpc.clone(),
                    self.s.game_path.clone(),
                    self.s.steam_path.clone(),
                    self.s.gameinfo.active_preset.clone(),
                    game_up,
                    game_pid,
                );
            }
        }

        let watcher_changed = self.last_watcher_game_up != game_up
            || self.last_watcher_key.0 != self.s.steam_path
            || self.last_watcher_key.1 != self.s.game_path
            || self.last_watcher_key.2 != self.s.gameinfo;
        if watcher_changed {
            if let Some(ref mut w) = self.gi_watcher {
                w.sync_config(&self.s, game_up);
                self.last_watcher_key = (
                    self.s.steam_path.clone(),
                    self.s.game_path.clone(),
                    self.s.gameinfo.clone(),
                );
                self.last_watcher_game_up = game_up;
            }
        }

        if let Some(res) = self
            .gi_watcher
            .as_ref()
            .and_then(GameInfoWatcher::take_notification)
        {
            match res {
                Ok(report) => {
                    if report.is_patch_adaptation {
                        self.patch_banner = Some(report.message.clone());
                    }
                    self.set_status(true, report.message);
                    self.refresh_gi_state();
                }
                Err(e) => {
                    self.sfx(UiSound::Error);
                    self.set_status(false, e);
                    self.refresh_gi_state();
                }
            }
        }

        // При сворачивании окна сохраняем несохранённые настройки, но НИКОГДА не делаем ранний return;
        // (иначе eframe очистит буфер OpenGL и вызовет SwapBuffers с пустым чёрным кадром).
        if effectively_hidden && !restored_from_tray && self.s != self.saved {
            let already_saving = self
                .saving_settings_snapshot
                .as_ref()
                .is_some_and(|snap| snap == &self.s);
            if !already_saving {
                self.saving_settings_snapshot = Some(self.s.clone());
                self.send_disk(DiskCmd::SaveSettings {
                    settings: Box::new(self.s.clone()),
                });
            }
        }

        if self.restore_repaint_frames > 0 {
            self.restore_repaint_frames -= 1;
            ctx.request_repaint();
            if self.restore_repaint_frames == 0 {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        }

        // Обработка перетащенных модов (.vpk)
        let dropped_paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped_paths.is_empty() {
            let has_supported = dropped_paths.iter().any(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("vpk"))
            });
            if has_supported || self.tab == Tab::Mods {
                if self.tab != Tab::Mods {
                    self.switch_tab(ctx, Tab::Mods);
                }
                self.sfx(UiSound::Success);
                self.send_disk(DiskCmd::InstallDroppedFiles {
                    paths: dropped_paths,
                    game_path: PathBuf::from(&self.s.game_path),
                });
            }
        }

        self.refresh_live_telemetry(ctx);

        // Таймеры пасхалок и уведомлений
        let now_inst = Instant::now();
        let mut easter_active = false;
        let mut occult_glow_t = 0.0_f32;
        if let Some(until) = self.occult_easter_until {
            if now_inst >= until {
                self.occult_easter_until = None;
                if let Some((prev_ok, prev_msg)) = self.easter_prev_status.take() {
                    self.status_ok = prev_ok;
                    self.status = prev_msg;
                }
            } else {
                let rem = until.saturating_duration_since(now_inst).as_secs_f32();
                occult_glow_t = (rem / 0.65).clamp(0.0, 1.0);
                easter_active = true;
            }
        }
        let mut midnight_glow_t = 0.0_f32;
        if let Some(until) = self.midnight_easter_until {
            if now_inst >= until {
                self.midnight_easter_until = None;
                if self.occult_easter_until.is_none() {
                    if let Some((prev_ok, prev_msg)) = self.easter_prev_status.take() {
                        self.status_ok = prev_ok;
                        self.status = prev_msg;
                    }
                }
            } else {
                let rem = until.saturating_duration_since(now_inst).as_secs_f32();
                midnight_glow_t = (rem / 0.55).clamp(0.0, 1.0);
                easter_active = true;
            }
        }
        if easter_active && allow_timers {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(until) = self.toast_until {
            if now_inst >= until {
                self.toast_until = None;
            } else if allow_timers && focused {
                let rem = until.saturating_duration_since(now_inst);
                ctx.request_repaint_after(rem + Duration::from_millis(50));
            }
        }

        // ---- 1. Компактная левая боковая панель (180px)
        egui::SidePanel::left("tabs")
            .resizable(false)
            .exact_width(180.0)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let panel_rect = ui.max_rect();
                ui.painter().rect_filled(panel_rect, 0.0, theme::BG_SLATE);
                ui.painter().line_segment(
                    [
                        egui::pos2(panel_rect.right() - 1.0, panel_rect.top()),
                        egui::pos2(panel_rect.right() - 1.0, panel_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0_f32, theme::BORDER_SUBTLE),
                );

                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(12, 14))
                    .show(ui, |ui| {
                        if theme::paint_cursed_apple_seal_compact(
                            ui,
                            self.avatar.id(),
                            &pal,
                            occult_glow_t,
                        )
                        .clicked()
                        {
                            let click_now = Instant::now();
                            self.logo_click_times.push(click_now);
                            self.logo_click_times.retain(|&t| {
                                click_now.duration_since(t) <= Duration::from_secs(3)
                            });
                            if self.logo_click_times.len() >= 7 {
                                self.logo_click_times.clear();
                                if self.easter_prev_status.is_none() {
                                    self.easter_prev_status =
                                        Some((self.status_ok, self.status.clone()));
                                }
                                let phrase = OCCULT_EASTER_PHRASES
                                    [self.occult_phrase_idx % OCCULT_EASTER_PHRASES.len()];
                                self.occult_phrase_idx = self.occult_phrase_idx.wrapping_add(1);
                                self.status_ok = true;
                                self.status = phrase.to_owned();
                                self.toast_until = Some(click_now + Duration::from_secs(3));
                                self.occult_easter_until =
                                    Some(click_now + Duration::from_secs(3));
                                self.sfx(UiSound::Success);
                                ctx.request_repaint();
                            }
                        }

                        ui.add_space(10.0);
                        let mut next_tab: Option<Tab> = None;
                        for t in Tab::ALL {
                            if theme::nav_tab_button(
                                ui,
                                self.tab == t,
                                t.icon(),
                                t.title(),
                                &pal,
                            )
                            .on_hover_text(t.tooltip())
                            .clicked()
                            {
                                next_tab = Some(t);
                            }
                            ui.add_space(6.0);
                        }
                        if let Some(t) = next_tab {
                            self.switch_tab(ctx, t);
                        }

                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                            ui.label(theme::mono_text(
                                concat!("v", env!("CARGO_PKG_VERSION")),
                                10.5,
                                theme::TEXT_MUTED,
                            ));
                            if game_up {
                                ui.add_space(8.0);
                                if theme::crimson_button(
                                    ui,
                                    Some(Icon::Power),
                                    "Закрыть игру",
                                    egui::vec2(ui.available_width(), 28.0),
                                    true,
                                )
                                .on_hover_text("Завершить процесс Deadlock")
                                .clicked()
                                {
                                    self.sfx(UiSound::Click);
                                    match tools::kill_deadlock() {
                                        Ok(msg) => self.set_status(true, msg),
                                        Err(e) => {
                                            self.sfx(UiSound::Error);
                                            self.set_status(false, e);
                                        }
                                    }
                                }
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::vec2(8.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(r.center(), 3.0, theme::SOUL_TEAL);
                                    let boost_tip = match (
                                        self.is_priority_boosted(),
                                        self.is_affinity_optimized(),
                                    ) {
                                        (true, true) => "Высокий приоритет ЦП и свободное 0-е ядро активны",
                                        (true, false) => "Высокий приоритет ЦП активен для Deadlock",
                                        (false, true) => "0-е ядро освобождено от нагрузки игры",
                                        (false, false) => "Deadlock сейчас запущен",
                                    };
                                    ui.label(theme::mono_text(
                                        "В ИГРЕ",
                                        10.5,
                                        theme::SOUL_TEAL,
                                    ))
                                    .on_hover_text(boost_tip);
                                });
                            }
                        });
                    });
            });

        // ---- 2. Нижний статус-бар (ровно 1 компактная строка высотой 28px)
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(28.0)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let status_glow = occult_glow_t.max(midnight_glow_t * 0.85);
                ui.painter().rect_filled(rect, 0.0, theme::BG_SLATE);
                ui.painter().line_segment(
                    [egui::pos2(rect.left(), rect.top()), egui::pos2(rect.right(), rect.top())],
                    egui::Stroke::new(
                        1.0_f32 + 0.6 * status_glow,
                        theme::BORDER_SUBTLE.lerp_to_gamma(theme::SOUL_TEAL, status_glow),
                    ),
                );

                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(14, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Слева: живой индикатор защиты gameinfo.gi
                            let (dot_col, ind_text, ind_tip, can_click_ind) =
                                if !self.s.gameinfo.enabled {
                                    (
                                        theme::TEXT_MUTED,
                                        "Защита gameinfo.gi: Выкл",
                                        "Автозащита отключена (включается в Настройках)",
                                        false,
                                    )
                                } else {
                                    match &self.gi_status {
                                        SyncStatus::UpToDate { active_preset, .. } => (
                                            theme::SOUL_TEAL,
                                            "Защита gameinfo.gi: Активна",
                                            active_preset.as_str(),
                                            false,
                                        ),
                                        SyncStatus::ExternalChangeDetected { .. } => (
                                            pal.accent_light,
                                            "Защита gameinfo.gi: Нужна адаптация",
                                            "Нажмите, чтобы адаптировать профиль после патча Valve",
                                            true,
                                        ),
                                        SyncStatus::GameNotFound => (
                                            theme::CRIMSON,
                                            "Защита gameinfo.gi: Папка не найдена",
                                            "Нажмите, чтобы указать папку Deadlock в Настройках",
                                            true,
                                        ),
                                    }
                                };

                            let (dot_r, _) = ui.allocate_exact_size(
                                egui::vec2(10.0, 14.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(dot_r.center(), 3.4, dot_col);

                            let lbl_resp = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(ind_text).size(12.0).color(dot_col),
                                    )
                                    .sense(if can_click_ind {
                                        egui::Sense::click()
                                    } else {
                                        egui::Sense::hover()
                                    }),
                                )
                                .on_hover_text(ind_tip);
                            if can_click_ind && lbl_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if can_click_ind && lbl_resp.clicked() {
                                if matches!(self.gi_status, SyncStatus::GameNotFound) {
                                    self.switch_tab(ctx, Tab::Settings);
                                } else {
                                    self.check_gi_patch_now();
                                }
                            }

                            // Справа: кнопка звука + память лаунчера
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let sound_active =
                                    self.s.ui_sound_enabled && self.s.ui_sound_volume > 0.01;
                                let (snd_rect, snd_resp) = ui.allocate_exact_size(
                                    egui::vec2(24.0, 18.0),
                                    egui::Sense::click(),
                                );
                                if snd_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                let snd_col = if sound_active {
                                    theme::TEXT_CREAM
                                } else {
                                    theme::TEXT_MUTED
                                };
                                theme::paint_icon(
                                    ui.painter(),
                                    egui::Rect::from_center_size(
                                        snd_rect.center(),
                                        egui::vec2(13.0, 13.0),
                                    ),
                                    if sound_active {
                                        Icon::VolumeOn
                                    } else {
                                        Icon::VolumeOff
                                    },
                                    snd_col,
                                    1.4,
                                );
                                if snd_resp
                                    .on_hover_text(if sound_active {
                                        "Звук интерфейса: Вкл (нажмите, чтобы выключить)"
                                    } else {
                                        "Звук интерфейса: Выкл (нажмите, чтобы включить)"
                                    })
                                    .clicked()
                                {
                                    self.s.ui_sound_enabled = !self.s.ui_sound_enabled;
                                    if self.s.ui_sound_enabled && self.s.ui_sound_volume <= 0.01 {
                                        self.s.ui_sound_volume = 0.45;
                                    }
                                    self.sfx(UiSound::Toggle);
                                }

                                ui.add_space(6.0);
                                let ram_text = game_booster::format_ram_mb(self.cached_ram_bytes);
                                let ram_resp = ui
                                    .add(
                                        egui::Label::new(theme::mono_text(
                                            &ram_text,
                                            11.5,
                                            theme::TEXT_MUTED,
                                        ))
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Память лаунчера — нажмите, чтобы сжать");
                                if ram_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if ram_resp.clicked() {
                                    self.sfx(UiSound::Click);
                                    game_booster::trim_memory();
                                    self.cached_ram_bytes =
                                        game_booster::current_working_set_bytes();
                                    self.set_status(true, "Память лаунчера сжата");
                                }

                                // Центр: краткое уведомление о действии или ошибке
                                let show_toast = !self.status_ok
                                    || self.heavy_busy
                                    || easter_active
                                    || self.toast_until.is_some();
                                if show_toast && !self.status.is_empty() {
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add_space(8.0);
                                            let msg_col = if !self.status_ok {
                                                theme::CRIMSON
                                            } else if easter_active {
                                                theme::SOUL_TEAL
                                            } else {
                                                theme::TEXT_MUTED
                                            };
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&self.status)
                                                        .size(11.5)
                                                        .color(msg_col),
                                                )
                                                .truncate(),
                                            );
                                        },
                                    );
                                }
                            });
                        });
                    });
            });

        // ---- 3. Центральная рабочая область с воздухом (24×20px)
        let now = ctx.input(|i| i.time);
        let ease_t = if self.s.animations_enabled {
            let raw_t =
                ((now - self.tab_switched_at) as f32 / theme::ANIM_DURATION_SEC).clamp(0.0, 1.0);
            if raw_t < 1.0 && allow_timers {
                ctx.request_repaint();
            }
            egui::emath::easing::quadratic_out(raw_t)
        } else {
            1.0
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let crect = ui.max_rect();
                ui.painter().rect_filled(crect, 0.0, theme::BG_NOIR);

                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_opacity(0.28 + 0.72 * ease_t);
                        if ease_t < 0.999 {
                            ui.add_space((1.0 - ease_t) * 5.0);
                        }

                        let mut trigger_update: Option<(String, String)> = None;
                        if let Some((ref new_ver, ref download_url)) = self.update_available {
                            let label = if self.update_applying {
                                format!("Обновление до v{new_ver}…")
                            } else {
                                format!("Обновить до v{new_ver}")
                            };
                            ui.horizontal(|ui| {
                                if theme::brass_primary_button(
                                    ui,
                                    Some(Icon::Refresh),
                                    &label,
                                    egui::vec2(0.0, 28.0),
                                    !self.update_applying,
                                    &pal,
                                )
                                .clicked()
                                {
                                    trigger_update = Some((new_ver.clone(), download_url.clone()));
                                }
                            });
                            ui.add_space(10.0);
                        }
                        if let Some((ver, url)) = trigger_update {
                            self.sfx(UiSound::Click);
                            self.start_self_update(ctx, ver, url);
                        }

                        let mut dismiss_banner = false;
                        if let Some(ref banner_msg) = self.patch_banner {
                            theme::accent_banner_frame(
                                ui,
                                Color32::from_rgb(20, 34, 31),
                                theme::SOUL_TEAL,
                                |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(banner_msg)
                                                .color(theme::SOUL_TEAL)
                                                .strong(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if theme::brass_secondary_button(
                                                    ui,
                                                    None,
                                                    "Скрыть",
                                                    egui::vec2(0.0, 24.0),
                                                    true,
                                                    &pal,
                                                )
                                                .clicked()
                                                {
                                                    dismiss_banner = true;
                                                }
                                            },
                                        );
                                    });
                                },
                            );
                            ui.add_space(12.0);
                        }
                        if dismiss_banner {
                            self.patch_banner = None;
                        }

                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.tab {
                                Tab::Home => self.ui_home(ui, ctx),
                                Tab::Configs => self.ui_configs(ui, ctx),
                                Tab::Mods => self.ui_mods(ui, ctx),
                                Tab::Utilities => self.ui_utilities(ui),
                                Tab::Settings => self.ui_settings(ui, ctx),
                            });
                    });
            });

        // Модальные окна поверх интерфейса
        self.ui_modals(ctx);

        // ---- Автосохранение настроек с дебаунсом 400 мс (асинхронно)
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if !pointer_down {
            if self.s != self.saved {
                self.refresh_cached_previews();
                self.previews_synced = true;

                match self.pending_save_since {
                    None => {
                        self.pending_save_since = Some(now);
                        ctx.request_repaint_after(Duration::from_millis(420));
                    }
                    Some(since) if now - since >= 0.4 => {
                        let already_saving = self
                            .saving_settings_snapshot
                            .as_ref()
                            .is_some_and(|snap| snap == &self.s);
                        if !already_saving {
                            self.saving_settings_snapshot = Some(self.s.clone());
                            self.send_disk(DiskCmd::SaveSettings {
                                settings: Box::new(self.s.clone()),
                            });
                        }
                    }
                    Some(_) => {}
                }
            } else {
                self.pending_save_since = None;
                if !self.previews_synced {
                    self.refresh_cached_previews();
                    self.previews_synced = true;
                }
            }
        } else {
            self.previews_synced = false;
        }

        if self.startup_stage == StartupStage::PendingFirstFrame {
            self.startup_stage = StartupStage::SpawnDeferred;
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(ref mut tray) = self.tray {
            tray.shutdown();
        }
        if let Some(ref mut w) = self.io_worker {
            if self.s != self.saved {
                w.send_disk(DiskCmd::SaveSettings {
                    settings: Box::new(self.s.clone()),
                });
            }
            w.shutdown();
        } else if self.s != self.saved {
            let _ = self.s.save();
        }
        if let Some(ref w) = self.preset_watcher {
            w.shutdown();
        }
        if let Some(ref mut w) = self.gi_watcher {
            w.shutdown();
        }
        if let Some(ref mut rpc) = self.discord_rpc {
            rpc.shutdown();
        }
        if let Some(ref mut m) = self.monitor {
            m.shutdown();
        }
        game_booster::restore_all_runtime_boosts();
    }
}

// ============================================================ Вспомогательные функции

/// Рисует предпросмотр прицела в оптическом визире Art Deco.
fn draw_crosshair_preview(ui: &mut egui::Ui, c: &Crosshair, bg: Color32, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(180.0, 180.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, bg);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0_f32, theme::BORDER_SUBTLE),
        egui::epaint::StrokeKind::Inside,
    );
    theme::paint_corner_brackets(
        &painter,
        rect.shrink(4.0),
        10.0,
        egui::Stroke::new(1.2_f32, pal.accent_border),
    );
    painter.circle_stroke(
        rect.center(),
        52.0,
        egui::Stroke::new(1.0_f32, pal.accent_border.gamma_multiply(0.4)),
    );

    let center = rect.center();
    let scale = 1.8;
    let w = c.width * scale;
    let h = c.height * scale;
    let gap = c.gap * scale;

    let alpha = (c.opacity.clamp(0.0, 1.0) * 255.0) as u8;
    let color = Color32::from_rgba_unmultiplied(c.color[0], c.color[1], c.color[2], alpha);
    let border = Color32::from_rgba_unmultiplied(0, 0, 0, alpha);

    if h > 0.0 && alpha > 0 {
        let pips = [
            egui::Rect::from_center_size(
                egui::pos2(center.x, center.y - gap - h / 2.0),
                egui::vec2(w, h),
            ),
            egui::Rect::from_center_size(
                egui::pos2(center.x, center.y + gap + h / 2.0),
                egui::vec2(w, h),
            ),
            egui::Rect::from_center_size(
                egui::pos2(center.x - gap - h / 2.0, center.y),
                egui::vec2(h, w),
            ),
            egui::Rect::from_center_size(
                egui::pos2(center.x + gap + h / 2.0, center.y),
                egui::vec2(h, w),
            ),
        ];
        for pip in pips {
            if c.border {
                painter.rect_filled(pip.expand(1.5), 0.0, border);
            }
            painter.rect_filled(pip, 0.0, color);
        }
    }

    let dot_r = (w / 2.0).max(1.5);
    if c.dot_outline_opacity > 0.0 {
        let a = (c.dot_outline_opacity.clamp(0.0, 1.0) * 255.0) as u8;
        painter.circle_filled(
            center,
            dot_r + 1.5,
            Color32::from_rgba_unmultiplied(0, 0, 0, a),
        );
    }
    if c.dot_opacity > 0.0 {
        let a = (c.dot_opacity.clamp(0.0, 1.0) * 255.0) as u8;
        painter.circle_filled(
            center,
            dot_r,
            Color32::from_rgba_unmultiplied(c.color[0], c.color[1], c.color[2], a),
        );
    }
}

/// Назначает окну нативные иконки `ICON_BIG` и `ICON_SMALL` из `.rsrc` и возвращает `HWND` окна.
#[cfg(windows)]
fn apply_native_window_icons_from_resource() -> Option<usize> {
    use std::ffi::c_void;

    const GW_OWNER: u32 = 4;
    const IMAGE_ICON: u32 = 1;
    const LR_SHARED: u32 = 0x8000;
    const SM_CXICON: i32 = 11;
    const SM_CYICON: i32 = 12;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const GCLP_HICON: i32 = -14;
    const GCLP_HICONSM: i32 = -34;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
        fn GetModuleHandleW(lp_module_name: *const u16) -> *mut c_void;
    }
    #[link(name = "user32")]
    extern "system" {
        fn EnumThreadWindows(
            dw_thread_id: u32,
            lpfn: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            l_param: isize,
        ) -> i32;
        fn IsWindowVisible(h_wnd: *mut c_void) -> i32;
        fn GetWindow(h_wnd: *mut c_void, u_cmd: u32) -> *mut c_void;
        fn GetWindowTextW(h_wnd: *mut c_void, lp_string: *mut u16, n_max_count: i32) -> i32;
        fn GetSystemMetrics(n_index: i32) -> i32;
        fn LoadImageW(
            h_inst: *mut c_void,
            name: *const u16,
            type_: u32,
            cx: i32,
            cy: i32,
            fu_load: u32,
        ) -> *mut c_void;
        fn SendMessageW(h_wnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> isize;
        fn SetClassLongPtrW(h_wnd: *mut c_void, n_index: i32, dw_new_long: isize) -> usize;
    }

    unsafe extern "system" fn enum_cb(hwnd: *mut c_void, l_param: isize) -> i32 {
        let out = &mut *(l_param as *mut *mut c_void);
        if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            let mut buf = [0u16; 256];
            let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
            if copied > 0 {
                if let Some(slice) = buf.get(..copied as usize) {
                    let title = String::from_utf16_lossy(slice);
                    if title.contains("ShaurMeaw") {
                        *out = hwnd;
                        return 0;
                    }
                }
            }
        }
        1
    }

    unsafe {
        let mut target_hwnd: *mut c_void = std::ptr::null_mut();
        EnumThreadWindows(
            GetCurrentThreadId(),
            enum_cb,
            (&raw mut target_hwnd) as isize,
        );
        if target_hwnd.is_null() {
            return None;
        }

        let h_inst = GetModuleHandleW(std::ptr::null());
        if h_inst.is_null() {
            return Some(target_hwnd as usize);
        }

        let res_id_1 = 1usize as *const u16;
        let big_w = GetSystemMetrics(SM_CXICON).max(32);
        let big_h = GetSystemMetrics(SM_CYICON).max(32);
        let sm_w = GetSystemMetrics(SM_CXSMICON).max(16);
        let sm_h = GetSystemMetrics(SM_CYSMICON).max(16);

        let hicon_big = LoadImageW(h_inst, res_id_1, IMAGE_ICON, big_w, big_h, LR_SHARED);
        let hicon_small = LoadImageW(h_inst, res_id_1, IMAGE_ICON, sm_w, sm_h, LR_SHARED);

        if !hicon_big.is_null() {
            SendMessageW(target_hwnd, WM_SETICON, ICON_BIG, hicon_big as isize);
            SetClassLongPtrW(target_hwnd, GCLP_HICON, hicon_big as isize);
        }
        if !hicon_small.is_null() {
            SendMessageW(target_hwnd, WM_SETICON, ICON_SMALL, hicon_small as isize);
            SetClassLongPtrW(target_hwnd, GCLP_HICONSM, hicon_small as isize);
        }

        Some(target_hwnd as usize)
    }
}

#[cfg(not(windows))]
fn apply_native_window_icons_from_resource() -> Option<usize> {
    None
}

fn is_midnight_hunt_time(hour: u16, minute: u16) -> bool {
    hour < 4 || (hour == 4 && minute == 0)
}

fn is_midnight_hunt_now() -> bool {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            milliseconds: u16,
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(lp_system_time: *mut SystemTime);
        }

        let mut st = SystemTime::default();
        unsafe { GetLocalTime(&mut st) };
        is_midnight_hunt_time(st.hour, st.minute)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn test_setup_style_loads_windows_fonts_and_emojis() {
        let ctx = egui::Context::default();
        theme::setup_fonts_and_style(&ctx, Theme::ShaurMeaw);

        let mut prop_width = 0.0f32;
        let mut mono_width = 0.0f32;
        let mut serif_width = 0.0f32;
        let mut symbols_width = 0.0f32;
        let _ = ctx.run(Default::default(), |ctx| {
            ctx.fonts(|f| {
                prop_width = f
                    .layout_no_wrap(
                        "Оккультный нуар ShaurMeaw".to_owned(),
                        egui::FontId::proportional(14.5),
                        theme::TEXT_CREAM,
                    )
                    .size()
                    .x;
                symbols_width = f
                    .layout_no_wrap(
                        "«»×—•…→∞─◆".to_owned(),
                        egui::FontId::proportional(14.5),
                        theme::TEXT_CREAM,
                    )
                    .size()
                    .x;
                mono_width = f
                    .layout_no_wrap(
                        "// Комментарий gameinfo.gi".to_owned(),
                        egui::FontId::monospace(13.0),
                        theme::TEXT_CREAM,
                    )
                    .size()
                    .x;
                serif_width = f
                    .layout_no_wrap(
                        "SHAURMEAW • ПРОФИЛИ".to_owned(),
                        egui::FontId::new(15.0, theme::serif_family()),
                        theme::BRASS_LIGHT,
                    )
                    .size()
                    .x;
            });
        });

        assert!(prop_width > 0.0, "Пропорциональный шрифт должен рисовать текст");
        assert!(symbols_width > 0.0, "Пропорциональный шрифт должен рисовать спецсимволы интерфейса");
        assert!(mono_width > 0.0, "Моноширинный шрифт должен рисовать кириллицу");
        assert!(serif_width > 0.0, "Шрифт с засечками Art Deco должен рисовать заголовки");
    }

    #[test]
    fn test_midnight_hunt_boundaries() {
        assert!(is_midnight_hunt_time(0, 0));
        assert!(is_midnight_hunt_time(2, 36));
        assert!(is_midnight_hunt_time(3, 59));
        assert!(is_midnight_hunt_time(4, 0));
        assert!(!is_midnight_hunt_time(4, 1));
        assert!(!is_midnight_hunt_time(12, 0));
        assert!(!is_midnight_hunt_time(23, 59));
    }
}
