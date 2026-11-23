//! Настройки лаунчера ShaurMeaw: структура, темы, пресеты, сохранение в JSON.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Папка рядом с ShaurMeaw.exe для портативного режима.
pub const PORTABLE_DIR_NAME: &str = "ShaurMeaw_data";
/// Файл-маркер рядом с ShaurMeaw.exe, включающий портативный режим.
pub const PORTABLE_MARKER_FILE: &str = "portable.txt";

// ---------------------------------------------------------------- перечисления

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    ShaurMeaw,
    Deadlock,
    Amber,
    Neon,
    Amethyst,
}

fn deserialize_theme<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Theme, D::Error> {
    let s = String::deserialize(d)?;
    Ok(match s.as_str() {
        "ShaurMeaw" => Theme::ShaurMeaw,
        "Deadlock" => Theme::Deadlock,
        "Amber" => Theme::Amber,
        "Neon" => Theme::Neon,
        "Amethyst" => Theme::Amethyst,
        _ => Theme::Amber,
    })
}

impl Theme {
    pub const ALL: [Theme; 5] = [
        Theme::ShaurMeaw,
        Theme::Deadlock,
        Theme::Amber,
        Theme::Neon,
        Theme::Amethyst,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Theme::ShaurMeaw => "Оккультная латунь",
            Theme::Deadlock => "Старинная бронза",
            Theme::Amber => "Янтарное золото",
            Theme::Neon => "Патина душ",
            Theme::Amethyst => "Оккультный бордо",
        }
    }

    pub fn accent_rgb(self) -> [u8; 3] {
        match self {
            Theme::ShaurMeaw => [212, 163, 89],
            Theme::Deadlock => [200, 146, 78],
            Theme::Amber => [229, 184, 105],
            Theme::Neon => [62, 208, 165],
            Theme::Amethyst => [184, 107, 119],
        }
    }

    pub fn selection_rgb(self) -> [u8; 3] {
        match self {
            Theme::ShaurMeaw => [76, 59, 34],
            Theme::Deadlock => [72, 54, 30],
            Theme::Amber => [82, 64, 34],
            Theme::Neon => [28, 76, 66],
            Theme::Amethyst => [74, 42, 49],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FovPreset {
    #[default]
    FromConfig,
    Fov80,
    Fov85,
    Fov90,
    Fov100,
    Fov110,
}

impl FovPreset {
    pub const ALL: [FovPreset; 6] = [
        FovPreset::FromConfig,
        FovPreset::Fov80,
        FovPreset::Fov85,
        FovPreset::Fov90,
        FovPreset::Fov100,
        FovPreset::Fov110,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FovPreset::FromConfig => "Как в конфиге",
            FovPreset::Fov80 => "80",
            FovPreset::Fov85 => "85",
            FovPreset::Fov90 => "90",
            FovPreset::Fov100 => "100",
            FovPreset::Fov110 => "110",
        }
    }

    pub fn aspect_ratio(self) -> Option<&'static str> {
        match self {
            FovPreset::FromConfig => None,
            FovPreset::Fov80 => Some("1.75"),
            FovPreset::Fov85 => Some("2.00"),
            FovPreset::Fov90 => Some("2.15"),
            FovPreset::Fov100 => Some("2.49"),
            FovPreset::Fov110 => Some("3.00"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchMode {
    /// steam.exe -applaunch 1422450 ...
    Steam,
    /// напрямую deadlock.exe (Steam должен быть запущен)
    Direct,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderApi {
    Default,
    Dx11,
    Vulkan,
}

impl RenderApi {
    pub const ALL: [RenderApi; 3] = [RenderApi::Default, RenderApi::Dx11, RenderApi::Vulkan];
    pub fn label(self) -> &'static str {
        match self {
            RenderApi::Default => "По умолчанию",
            RenderApi::Dx11 => "DirectX 11  (-dx11)",
            RenderApi::Vulkan => "Vulkan  (-vulkan)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowMode {
    Default,
    Fullscreen,
    Windowed,
    Borderless,
}

impl WindowMode {
    pub const ALL: [WindowMode; 4] = [
        WindowMode::Default,
        WindowMode::Fullscreen,
        WindowMode::Windowed,
        WindowMode::Borderless,
    ];
    pub fn label(self) -> &'static str {
        match self {
            WindowMode::Default => "Как в настройках игры",
            WindowMode::Fullscreen => "Полный экран  (-fullscreen)",
            WindowMode::Windowed => "В окне  (-windowed)",
            WindowMode::Borderless => "Окно без рамки  (-windowed -noborder)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    MaxFps,
    Balanced,
    Quality,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::MaxFps, Preset::Balanced, Preset::Quality];
    pub fn label(self) -> &'static str {
        match self {
            Preset::MaxFps => "Максимум FPS",
            Preset::Balanced => "Баланс",
            Preset::Quality => "Качество",
        }
    }
}

/// Профиль глобальной оптимизации (Game Booster + движок Source 2 + параметры запуска).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OptimizationProfile {
    EsportsMaxFps,
    Balanced,
    StandardReset,
}

impl OptimizationProfile {
    pub fn label(self) -> &'static str {
        match self {
            OptimizationProfile::EsportsMaxFps => "Киберспорт (Макс. FPS)",
            OptimizationProfile::Balanced => "Сбалансированный",
            OptimizationProfile::StandardReset => "Стандарт (Сброс)",
        }
    }
}

// ---------------------------------------------------------------- структуры

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Bind {
    pub key: String,
    pub command: String,
}

impl Bind {
    pub fn new(key: &str, command: &str) -> Self {
        Self {
            key: key.to_owned(),
            command: command.to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Crosshair {
    /// Писать ли настройки прицела в autoexec
    pub enabled: bool,
    pub color: [u8; 3],
    pub gap: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
    pub border: bool,
    pub dot_opacity: f32,
    pub dot_outline_opacity: f32,
}

impl Default for Crosshair {
    fn default() -> Self {
        Self {
            enabled: false,
            color: [255, 255, 255],
            gap: 6.0,
            width: 2.0,
            height: 8.0,
            opacity: 1.0,
            border: true,
            dot_opacity: 0.0,
            dot_outline_opacity: 0.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Graphics {
    /// Писать ли графические команды в autoexec (иначе берутся настройки из меню игры)
    pub enabled: bool,
    pub ssao: bool,
    pub fog: bool,
    pub outlines: bool,
    pub depth_of_field: bool,
    pub farfield: bool,
    pub dashboard_hq: bool,
    /// 0..=3
    pub particle_detail: u8,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            enabled: false,
            ssao: true,
            fog: true,
            outlines: true,
            depth_of_field: true,
            farfield: true,
            dashboard_hq: true,
            particle_detail: 3,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Gameplay {
    pub manage_volume: bool,
    pub volume: f32,
    pub manage_sensitivity: bool,
    pub sensitivity: f32,
    /// Показывать FPS / время кадра в HUD телеметрии Source 2
    pub telemetry_fps: bool,
    /// Показывать пинг в HUD телеметрии Source 2
    pub telemetry_ping: bool,
    /// Показывать потерю пакетов в HUD телеметрии Source 2
    pub telemetry_loss: bool,
}

impl Default for Gameplay {
    fn default() -> Self {
        Self {
            manage_volume: false,
            volume: 0.5,
            manage_sensitivity: false,
            sensitivity: 1.0,
            telemetry_fps: false,
            telemetry_ping: false,
            telemetry_loss: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct GameInfoConfig {
    /// Включить автоуправление и защиту gameinfo.gi
    pub enabled: bool,
    /// Активный пресет из папки presets/
    pub active_preset: String,
    /// Проверка при запуске и фоновый мониторинг SHA-256 / mtime
    pub auto_watch_updates: bool,
    /// Проверять и адаптировать gameinfo.gi перед каждым запуском игры
    pub rebuild_before_launch: bool,
    /// Комментировать PGIVersion (по умолчанию false — сохраняет актуальный хэш Valve)
    pub comment_pgi_version: bool,
    /// Применять быстрые настройки (FOV, гамма, пул текстур, тени) поверх активного пресета
    pub use_quick_tweaks: bool,
    pub fov_aspect_ratio: f32,
    pub gamma: f32,
    pub texture_pool_mb: u32,
    pub texture_mip_bias: u32,
    pub shadows_enabled: bool,
    /// Переопределение FOV кнопкой быстрого выбора
    pub fov_preset: FovPreset,
    /// Скрыть имена над полосками здоровья
    pub hide_hp_names: bool,
    /// Яркая обводка героев (×1.5)
    pub bright_outlines: bool,
    /// Убрать вспышки выстрелов и свечение частиц
    pub no_flash_lights: bool,
    /// Убрать покачивание камеры и тряску от ближнего боя
    pub no_camera_shake: bool,
    /// Оптимизация потоков CPU и предкомпиляции шейдеров
    pub cpu_threads_boost: bool,
}

impl Default for GameInfoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            active_preset: "ShaurMeaw (Мой конфиг)".to_owned(),
            auto_watch_updates: true,
            rebuild_before_launch: true,
            comment_pgi_version: false,
            use_quick_tweaks: false,
            fov_aspect_ratio: 2.3,
            gamma: 1.4,
            texture_pool_mb: 256,
            texture_mip_bias: 16,
            shadows_enabled: false,
            fov_preset: FovPreset::FromConfig,
            hide_hp_names: false,
            bright_outlines: false,
            no_flash_lights: false,
            no_camera_shake: false,
            cpu_threads_boost: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct DiscordRpcConfig {
    pub enabled: bool,
    pub client_id: String,
    pub auto_condebug: bool,
    pub show_hero: bool,
    pub show_steam_name: bool,
    pub show_hero_icon: bool,
    pub hero_as_large_icon: bool,
    pub app_icon_url: String,
    pub show_timer: bool,
    #[serde(rename = "show_preset_v2", default)]
    pub show_preset: bool,
}

impl Default for DiscordRpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: String::new(),
            auto_condebug: true,
            show_hero: true,
            show_steam_name: true,
            show_hero_icon: true,
            hero_as_large_icon: false,
            app_icon_url: String::new(),
            show_timer: true,
            show_preset: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    // --- оформление и интерфейс
    #[serde(deserialize_with = "deserialize_theme")]
    pub theme: Theme,
    /// Масштаб интерфейса лаунчера (`0.85..=1.25`)
    pub ui_scale: f32,
    /// Плавные анимации переходов
    pub animations_enabled: bool,
    /// Размер шрифта в редакторе кода (`11.0..=17.0`)
    pub editor_font_size: f32,
    /// Сворачивать окно лаунчера в системный трей
    #[serde(rename = "minimize_to_tray_v2", default)]
    pub minimize_to_tray: bool,
    /// Автоматически переключать раскладку игры на английскую (EN) при запуске Deadlock
    pub auto_english_layout: bool,
    /// Включить лёгкие звуки отклика интерфейса
    pub ui_sound_enabled: bool,
    /// Громкость звуков интерфейса (`0.0..=1.0`)
    pub ui_sound_volume: f32,

    // --- пути
    pub steam_path: String,
    pub game_path: String,
    pub launch_mode: LaunchMode,

    // --- параметры запуска
    pub novid: bool,
    pub console: bool,
    pub dev: bool,
    pub high_priority: bool,
    /// Автоматически выставлять HIGH_PRIORITY_CLASS процессу Deadlock после запуска
    pub auto_high_priority: bool,
    /// Сжимать рабочий набор памяти лаунчера (RAM Trim) при сворачивании и запуске Deadlock
    pub ram_trim_on_minimize: bool,
    /// Автоматически обрезать разросшийся console.log (> 50 МБ) перед запуском игры
    pub auto_clean_console_log: bool,
    /// Включать системный таймер Windows 1 мс на время матча (Game Booster)
    pub boost_timer_1ms: bool,
    /// Автоматически включать схему питания «Высокая производительность» на время матча
    pub boost_power_plan: bool,
    /// Снимать CPU 0 с процесса Deadlock (только при >= 6 потоках ЦП)
    pub boost_disable_cpu0: bool,
    pub nojoy: bool,
    pub render_api: RenderApi,
    pub window_mode: WindowMode,
    pub custom_resolution: bool,
    pub width: u32,
    pub height: u32,
    /// 0 = авто
    pub threads: u32,
    /// пусто = язык по умолчанию
    pub language: String,
    pub extra_launch_args: String,
    pub exec_autoexec: bool,
    pub write_autoexec_on_launch: bool,

    // --- производительность
    /// 0 = без ограничения
    pub fps_max: u32,
    pub show_fps: bool,
    pub no_focus_sleep: bool,
    pub sound_on_lost_focus: bool,
    /// Режим низкой задержки (`r_low_latency`: 0 = Выкл, 1 = Вкл, 2 = Вкл + Boost)
    pub low_latency_mode: u8,
    /// `engine_low_latency_sleep_after_client_tick 1`
    pub low_latency_sleep_after_tick: bool,
    /// Оптимизация сетевого буфера и интерполяции (`rate 786432`, `cl_interp_ratio 1`)
    pub optimize_network: bool,
    /// Низкая задержка микшера звука (`snd_mixahead 0.025`)
    pub optimize_audio_latency: bool,
    /// Отключить физику рэгдоллов и декали крови/пуль
    pub disable_ragdolls_decals: bool,
    /// Отключить динамические тени в `autoexec.cfg`
    pub disable_shadows_autoexec: bool,
    /// Передавать `-forcenovsync` в параметры запуска игры
    pub force_no_vsync_arg: bool,

    pub graphics: Graphics,
    pub gameplay: Gameplay,
    pub gameinfo: GameInfoConfig,
    pub autoexec_preset: String,
    pub video_preset: String,
    pub write_video_on_launch: bool,
    pub video_preserve_gpu_ids: bool,
    pub video_preserve_resolution: bool,
    pub video_apply_all_accounts: bool,
    pub video_set_readonly: bool,
    pub crosshair: Crosshair,
    pub binds: Vec<Bind>,
    pub custom_commands: String,
    pub discord_rpc: DiscordRpcConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::ShaurMeaw,
            ui_scale: 1.0,
            animations_enabled: true,
            editor_font_size: 13.0,
            minimize_to_tray: false,
            auto_english_layout: false,
            ui_sound_enabled: true,
            ui_sound_volume: 0.45,

            steam_path: String::new(),
            game_path: String::new(),
            launch_mode: LaunchMode::Steam,

            novid: true,
            console: true,
            dev: false,
            high_priority: true,
            auto_high_priority: true,
            ram_trim_on_minimize: true,
            auto_clean_console_log: true,
            boost_timer_1ms: true,
            boost_power_plan: true,
            boost_disable_cpu0: false,
            nojoy: true,
            render_api: RenderApi::Default,
            window_mode: WindowMode::Default,
            custom_resolution: false,
            width: 1920,
            height: 1080,
            threads: 0,
            language: String::new(),
            extra_launch_args: String::new(),
            exec_autoexec: true,
            write_autoexec_on_launch: true,

            fps_max: 240,
            show_fps: false,
            no_focus_sleep: true,
            sound_on_lost_focus: true,
            low_latency_mode: 1,
            low_latency_sleep_after_tick: true,
            optimize_network: true,
            optimize_audio_latency: true,
            disable_ragdolls_decals: false,
            disable_shadows_autoexec: false,
            force_no_vsync_arg: false,

            graphics: Graphics::default(),
            gameplay: Gameplay::default(),
            gameinfo: GameInfoConfig::default(),
            autoexec_preset: "ShaurMeaw (Мой autoexec)".to_owned(),
            video_preset: "ShaurMeaw (Мой video.txt)".to_owned(),
            write_video_on_launch: false,
            video_preserve_gpu_ids: true,
            video_preserve_resolution: false,
            video_apply_all_accounts: false,
            video_set_readonly: false,
            crosshair: Crosshair::default(),
            binds: vec![Bind::new("F7", "toggleconsole")],
            custom_commands: String::new(),
            discord_rpc: DiscordRpcConfig::default(),
        }
    }
}

fn is_dir_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".shaurmeaw_write_probe_{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

impl Settings {
    /// Папка, где находится текущий исполняемый файл `ShaurMeaw.exe`.
    pub fn exe_dir() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }

    /// Проверяет, активен ли портативный режим (аргумент `--portable`, файл `portable.txt` или папка `ShaurMeaw_data` рядом с `.exe`).
    pub fn is_portable_mode() -> bool {
        if std::env::args().any(|a| a == "--portable") {
            return true;
        }
        Self::exe_dir().is_some_and(|dir| {
            dir.join(PORTABLE_MARKER_FILE).exists() || dir.join(PORTABLE_DIR_NAME).is_dir()
        })
    }

    fn default_appdata_dir() -> PathBuf {
        let appdata = std::env::var_os("APPDATA")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if let Some(base) = appdata {
            return base.join("ShaurMeaw");
        }
        let local_appdata = std::env::var_os("LOCALAPPDATA")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if let Some(base) = local_appdata {
            return base.join("ShaurMeaw");
        }
        if let Some(ed) = Self::exe_dir() {
            return ed.join(PORTABLE_DIR_NAME);
        }
        PathBuf::from(PORTABLE_DIR_NAME)
    }

    /// Чистая логика выбора папки данных для тестов и `app_dir()`.
    pub fn resolve_app_dir_for(
        exe_dir: Option<&Path>,
        force_portable: bool,
        appdata: Option<&Path>,
        local_appdata: Option<&Path>,
    ) -> PathBuf {
        if let Some(ed) = exe_dir {
            let portable_active = force_portable
                || ed.join(PORTABLE_MARKER_FILE).exists()
                || ed.join(PORTABLE_DIR_NAME).is_dir();
            if portable_active {
                return ed.join(PORTABLE_DIR_NAME);
            }
        }
        if let Some(ad) = appdata.filter(|p| !p.as_os_str().is_empty()) {
            return ad.join("ShaurMeaw");
        }
        if let Some(lad) = local_appdata.filter(|p| !p.as_os_str().is_empty()) {
            return lad.join("ShaurMeaw");
        }
        if let Some(ed) = exe_dir {
            return ed.join(PORTABLE_DIR_NAME);
        }
        PathBuf::from(PORTABLE_DIR_NAME)
    }

    fn appdata_base() -> PathBuf {
        std::env::var_os("APPDATA")
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var_os("LOCALAPPDATA").filter(|s| !s.is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Активная папка данных лаунчера (`%APPDATA%\ShaurMeaw` либо `<exe_dir>\ShaurMeaw_data` в портативном режиме).
    pub fn app_dir() -> PathBuf {
        let exe_dir = Self::exe_dir();
        let force_portable = std::env::args().any(|a| a == "--portable");
        let appdata = std::env::var_os("APPDATA")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let local_appdata = std::env::var_os("LOCALAPPDATA")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let primary = Self::resolve_app_dir_for(
            exe_dir.as_deref(),
            force_portable,
            appdata.as_deref(),
            local_appdata.as_deref(),
        );
        if is_dir_writable(&primary) {
            return primary;
        }
        let mut fallbacks = Vec::new();
        if let Some(ad) = appdata {
            fallbacks.push(ad.join("ShaurMeaw"));
        }
        if let Some(lad) = local_appdata {
            fallbacks.push(lad.join("ShaurMeaw"));
        }
        fallbacks.push(std::env::temp_dir().join("ShaurMeaw"));
        for cand in fallbacks {
            if cand != primary && is_dir_writable(&cand) {
                return cand;
            }
        }
        primary
    }

    /// Путь к логу критических ошибок (`crash.log`).
    pub fn crash_log_path() -> PathBuf {
        Self::app_dir().join("crash.log")
    }

    fn copy_dir_recursive_missing(src: &Path, dst: &Path) -> Result<(), String> {
        if !src.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("Не удалось создать папку {}: {e}", dst.display()))?;
        let entries = std::fs::read_dir(src)
            .map_err(|e| format!("Не удалось прочитать папку {}: {e}", src.display()))?;
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                Self::copy_dir_recursive_missing(&src_path, &dst_path)?;
            } else if src_path.is_file() && !dst_path.exists() {
                std::fs::copy(&src_path, &dst_path).map_err(|e| {
                    format!(
                        "Не удалось скопировать {} -> {}: {e}",
                        src_path.display(),
                        dst_path.display()
                    )
                })?;
            }
        }
        Ok(())
    }

    fn copy_user_data_between(src_root: &Path, dst_root: &Path) -> Result<(), String> {
        if src_root == dst_root || !src_root.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(dst_root)
            .map_err(|e| format!("Не удалось создать папку {}: {e}", dst_root.display()))?;

        for file_name in ["settings.json", "gameinfo_state.json"] {
            let src_file = src_root.join(file_name);
            let dst_file = dst_root.join(file_name);
            if src_file.is_file() && !dst_file.exists() {
                std::fs::copy(&src_file, &dst_file).map_err(|e| {
                    format!("Не удалось перенести {file_name}: {e}")
                })?;
            }
        }
        for dir_name in ["presets", "autoexec_presets", "video_presets", "profiles", "backups"] {
            Self::copy_dir_recursive_missing(&src_root.join(dir_name), &dst_root.join(dir_name))?;
        }
        Ok(())
    }

    /// Включает портативный режим рядом с `ShaurMeaw.exe`, перенося данные из `%APPDATA%\ShaurMeaw`.
    pub fn enable_portable_mode(&self) -> Result<PathBuf, String> {
        let exe_dir = Self::exe_dir()
            .ok_or_else(|| "Не удалось определить папку запускаемого ShaurMeaw.exe".to_owned())?;
        let portable_dir = exe_dir.join(PORTABLE_DIR_NAME);
        let src_dir = Self::default_appdata_dir();

        Self::copy_user_data_between(&src_dir, &portable_dir)?;
        std::fs::create_dir_all(&portable_dir)
            .map_err(|e| format!("Не удалось создать {}: {e}", portable_dir.display()))?;

        let marker_path = exe_dir.join(PORTABLE_MARKER_FILE);
        let marker_text = concat!(
            "Портативный режим ShaurMeaw включён.\r\n",
            "Все настройки, пресеты и бэкапы хранятся в папке ShaurMeaw_data рядом с ShaurMeaw.exe.\r\n",
            "Чтобы вернуть хранение в %APPDATA%\\ShaurMeaw, используйте кнопку в лаунчере или удалите этот файл и переименуйте ShaurMeaw_data.\r\n",
        );
        std::fs::write(&marker_path, marker_text)
            .map_err(|e| format!("Не удалось создать {}: {e}", marker_path.display()))?;

        self.save()?;
        Ok(portable_dir)
    }

    /// Отключает портативный режим: переносит данные в `%APPDATA%\ShaurMeaw` и убирает маркеры рядом с `.exe`.
    pub fn disable_portable_mode(&self) -> Result<PathBuf, String> {
        let exe_dir = Self::exe_dir()
            .ok_or_else(|| "Не удалось определить папку запускаемого ShaurMeaw.exe".to_owned())?;
        let portable_dir = exe_dir.join(PORTABLE_DIR_NAME);
        let target_dir = Self::default_appdata_dir();

        Self::copy_user_data_between(&portable_dir, &target_dir)?;

        let marker_path = exe_dir.join(PORTABLE_MARKER_FILE);
        if marker_path.exists() {
            std::fs::remove_file(&marker_path)
                .map_err(|e| format!("Не удалось удалить {}: {e}", marker_path.display()))?;
        }

        if portable_dir.exists() {
            let mut backup_dir = exe_dir.join(format!("{PORTABLE_DIR_NAME}_backup"));
            let mut idx = 1usize;
            while backup_dir.exists() {
                backup_dir = exe_dir.join(format!("{PORTABLE_DIR_NAME}_backup_{idx}"));
                idx += 1;
            }
            std::fs::rename(&portable_dir, &backup_dir).map_err(|e| {
                format!(
                    "Не удалось переименовать {} в {}: {e}",
                    portable_dir.display(),
                    backup_dir.display()
                )
            })?;
        }

        self.save()?;
        Ok(target_dir)
    }

    /// settings.json
    pub fn file_path() -> PathBuf {
        Self::app_dir().join("settings.json")
    }

    /// %APPDATA%\ShaurMeaw\profiles
    pub fn profiles_dir() -> PathBuf {
        Self::app_dir().join("profiles")
    }

    /// %APPDATA%\ShaurMeaw\presets
    pub fn gameinfo_presets_dir() -> PathBuf {
        Self::app_dir().join("presets")
    }

    /// %APPDATA%\ShaurMeaw\autoexec_presets
    pub fn autoexec_presets_dir() -> PathBuf {
        Self::app_dir().join("autoexec_presets")
    }

    /// %APPDATA%\ShaurMeaw\video_presets
    pub fn video_presets_dir() -> PathBuf {
        Self::app_dir().join("video_presets")
    }

    /// %APPDATA%\ShaurMeaw\backups
    pub fn gameinfo_backups_dir() -> PathBuf {
        Self::app_dir().join("backups")
    }

    /// %APPDATA%\ShaurMeaw\gameinfo_state.json
    pub fn gameinfo_state_path() -> PathBuf {
        Self::app_dir().join("gameinfo_state.json")
    }

    fn load_from_path(path: &Path) -> Option<Self> {
        let text = crate::fs_safe::read_text_lossy_with_retry(path).ok()?;
        match serde_json::from_str(&text) {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                let mut os = path.as_os_str().to_os_string();
                os.push(".corrupt");
                let corrupt_path = PathBuf::from(os);
                if !corrupt_path.exists() {
                    let _ = std::fs::copy(path, &corrupt_path);
                }
                None
            }
        }
    }

    fn save_to_path(path: &Path, settings: &Self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        crate::fs_safe::atomic_write_with_retry(path, json.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn load() -> Self {
        let new_path = Self::file_path();
        if let Some(parsed) = Self::load_from_path(&new_path) {
            return parsed;
        }
        // Мягкая миграция со старого имени DeadlockLauncher (только для обычного режима)
        if !Self::is_portable_mode() {
            let old_path = Self::appdata_base()
                .join("DeadlockLauncher")
                .join("settings.json");
            if let Some(migrated) = Self::load_from_path(&old_path) {
                return migrated;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        Self::save_to_path(&Self::file_path(), self)
    }

    pub fn apply_preset(&mut self, preset: Preset) {
        let g = &mut self.graphics;
        g.enabled = true;
        match preset {
            Preset::MaxFps => {
                g.ssao = false;
                g.fog = false;
                g.outlines = false;
                g.depth_of_field = false;
                g.farfield = false;
                g.dashboard_hq = false;
                g.particle_detail = 0;
            }
            Preset::Balanced => {
                g.ssao = false;
                g.fog = true;
                g.outlines = true;
                g.depth_of_field = false;
                g.farfield = true;
                g.dashboard_hq = false;
                g.particle_detail = 1;
            }
            Preset::Quality => {
                g.ssao = true;
                g.fog = true;
                g.outlines = true;
                g.depth_of_field = true;
                g.farfield = true;
                g.dashboard_hq = true;
                g.particle_detail = 3;
            }
        }
    }

    /// Применяет комплексный профиль оптимизации (Game Booster + Source 2 + запуск).
    pub fn apply_optimization_profile(&mut self, profile: OptimizationProfile) {
        match profile {
            OptimizationProfile::EsportsMaxFps => {
                self.apply_preset(Preset::MaxFps);
                self.auto_high_priority = true;
                self.ram_trim_on_minimize = true;
                self.auto_clean_console_log = true;
                self.boost_timer_1ms = true;
                self.boost_power_plan = true;
                self.low_latency_mode = 2;
                self.low_latency_sleep_after_tick = true;
                self.optimize_network = true;
                self.optimize_audio_latency = true;
                self.disable_ragdolls_decals = true;
                self.disable_shadows_autoexec = true;
                self.force_no_vsync_arg = true;
                self.novid = true;
                self.nojoy = true;
                self.no_focus_sleep = true;
            }
            OptimizationProfile::Balanced => {
                self.apply_preset(Preset::Balanced);
                self.auto_high_priority = true;
                self.ram_trim_on_minimize = true;
                self.auto_clean_console_log = true;
                self.boost_timer_1ms = true;
                self.boost_power_plan = true;
                self.boost_disable_cpu0 = false;
                self.low_latency_mode = 1;
                self.low_latency_sleep_after_tick = true;
                self.optimize_network = true;
                self.optimize_audio_latency = true;
                self.disable_ragdolls_decals = false;
                self.disable_shadows_autoexec = false;
                self.force_no_vsync_arg = false;
            }
            OptimizationProfile::StandardReset => {
                self.apply_preset(Preset::Quality);
                self.boost_timer_1ms = false;
                self.boost_power_plan = false;
                self.boost_disable_cpu0 = false;
                self.low_latency_mode = 0;
                self.low_latency_sleep_after_tick = false;
                self.optimize_network = false;
                self.optimize_audio_latency = false;
                self.disable_ragdolls_decals = false;
                self.disable_shadows_autoexec = false;
                self.force_no_vsync_arg = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_portable_mode_resolution_and_copy() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_portable_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let exe_dir = root.join("bin");
        let appdata = root.join("roaming");
        let local_appdata = root.join("local");
        std::fs::create_dir_all(&exe_dir).unwrap();

        // 1. Обычный режим -> %APPDATA%\ShaurMeaw
        assert_eq!(
            Settings::resolve_app_dir_for(Some(&exe_dir), false, Some(&appdata), Some(&local_appdata)),
            appdata.join("ShaurMeaw")
        );

        // 2. Fallback на %LOCALAPPDATA%, если APPDATA отсутствует
        assert_eq!(
            Settings::resolve_app_dir_for(Some(&exe_dir), false, None, Some(&local_appdata)),
            local_appdata.join("ShaurMeaw")
        );

        // 3. Флаг --portable -> <exe_dir>\ShaurMeaw_data
        assert_eq!(
            Settings::resolve_app_dir_for(Some(&exe_dir), true, Some(&appdata), Some(&local_appdata)),
            exe_dir.join(PORTABLE_DIR_NAME)
        );

        // 4. Файл portable.txt рядом с exe
        std::fs::write(exe_dir.join(PORTABLE_MARKER_FILE), "1").unwrap();
        assert_eq!(
            Settings::resolve_app_dir_for(Some(&exe_dir), false, Some(&appdata), Some(&local_appdata)),
            exe_dir.join(PORTABLE_DIR_NAME)
        );
        std::fs::remove_file(exe_dir.join(PORTABLE_MARKER_FILE)).unwrap();

        // 5. Папка ShaurMeaw_data рядом с exe + тест copy_user_data_between
        let portable_dir = exe_dir.join(PORTABLE_DIR_NAME);
        std::fs::create_dir_all(&portable_dir).unwrap();
        assert_eq!(
            Settings::resolve_app_dir_for(Some(&exe_dir), false, Some(&appdata), Some(&local_appdata)),
            portable_dir
        );

        let src_data = appdata.join("ShaurMeaw");
        std::fs::create_dir_all(src_data.join("presets")).unwrap();
        std::fs::write(src_data.join("settings.json"), "{\"fps_max\":144}").unwrap();
        std::fs::write(src_data.join("presets").join("test.gi"), "GameInfo").unwrap();
        std::fs::write(portable_dir.join("settings.json"), "{\"fps_max\":240}").unwrap();

        Settings::copy_user_data_between(&src_data, &portable_dir).unwrap();
        // Существующий settings.json не перезаписывается, а отсутствующий пресет переносится
        assert_eq!(
            std::fs::read_to_string(portable_dir.join("settings.json")).unwrap(),
            "{\"fps_max\":240}"
        );
        assert_eq!(
            std::fs::read_to_string(portable_dir.join("presets").join("test.gi")).unwrap(),
            "GameInfo"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_deserialize_unknown_theme_falls_back_to_amber() {
        let json = r#"{"theme": "LegacyUnknownTheme", "novid": false}"#;
        let loaded: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.theme, Theme::Amber);
        assert!(!loaded.novid);

        // При отсутствии ключа theme используется значение по умолчанию
        let loaded_default: Settings = serde_json::from_str(r#"{"fps_max": 120}"#).unwrap();
        assert_eq!(loaded_default.theme, Theme::ShaurMeaw);
        assert_eq!(loaded_default.fps_max, 120);
    }

    #[test]
    fn test_new_ui_settings_defaults_and_backward_compat() {
        let loaded: Settings =
            serde_json::from_str(r#"{"novid": false, "show_protection_bar": true}"#).unwrap();
        assert!(!loaded.novid);
        assert!((loaded.ui_scale - 1.0).abs() < f32::EPSILON);
        assert!(loaded.animations_enabled);
        assert!((loaded.editor_font_size - 13.0).abs() < f32::EPSILON);
        assert!(loaded.auto_high_priority);
        assert!(loaded.ram_trim_on_minimize);
        assert!(loaded.auto_clean_console_log);
        assert!(loaded.boost_timer_1ms);
        assert!(loaded.boost_power_plan);
        assert!(!loaded.boost_disable_cpu0);
        assert_eq!(loaded.low_latency_mode, 1);
        assert!(loaded.low_latency_sleep_after_tick);
        assert!(loaded.optimize_network);
        assert!(loaded.optimize_audio_latency);
        assert!(!loaded.disable_ragdolls_decals);
        assert!(!loaded.disable_shadows_autoexec);
        assert!(!loaded.force_no_vsync_arg);

        let mut prof = Settings::default();
        prof.apply_optimization_profile(OptimizationProfile::EsportsMaxFps);
        assert_eq!(prof.low_latency_mode, 2);
        assert!(prof.disable_ragdolls_decals && prof.disable_shadows_autoexec && prof.force_no_vsync_arg);
        prof.apply_optimization_profile(OptimizationProfile::StandardReset);
        assert_eq!(prof.low_latency_mode, 0);
        assert!(!prof.boost_timer_1ms && !prof.boost_power_plan && !prof.optimize_network);
        assert!(loaded.discord_rpc.enabled);
        assert!(loaded.discord_rpc.auto_condebug);
        assert!(loaded.discord_rpc.show_hero);
        assert!(loaded.discord_rpc.show_steam_name);
        assert!(loaded.discord_rpc.show_hero_icon);
        assert!(!loaded.discord_rpc.hero_as_large_icon);
        assert!(loaded.discord_rpc.app_icon_url.is_empty());
        assert!(loaded.discord_rpc.show_timer);
        assert!(!loaded.discord_rpc.show_preset);
        assert!(!loaded.minimize_to_tray);
        assert!(!loaded.auto_english_layout);
        assert!(loaded.ui_sound_enabled);
        assert!((loaded.ui_sound_volume - 0.45).abs() < f32::EPSILON);
    }

    #[test]
    fn test_atomic_save_and_corrupt_backup() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_settings_atomic_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("settings.json");

        let mut s = Settings::default();
        s.fps_max = 180;
        Settings::save_to_path(&path, &s).unwrap();
        assert!(path.is_file());
        assert!(!root.join("settings.json.tmp").exists());

        let loaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(loaded.fps_max, 180);

        // Повреждённый JSON сохраняется в settings.json.corrupt
        std::fs::write(&path, "{broken json").unwrap();
        assert!(Settings::load_from_path(&path).is_none());
        let corrupt = root.join("settings.json.corrupt");
        assert!(corrupt.is_file());
        assert_eq!(std::fs::read_to_string(&corrupt).unwrap(), "{broken json");

        let _ = std::fs::remove_dir_all(&root);
    }
}
