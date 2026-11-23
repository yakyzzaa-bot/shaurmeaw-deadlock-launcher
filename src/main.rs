// В release-сборке не показываем чёрное окно консоли
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_cache;
mod autoexec_presets;
mod config_editor;
mod config_gen;
mod discord_rpc;
mod embedded;
mod fs_safe;
mod game_booster;
mod gameinfo_presets;
mod gameinfo_sync;
mod launcher;
mod map_timers;
mod settings;
mod sfx;
mod single_instance;
mod steam;
mod theme;
mod tools;
mod tray;
mod updater;
mod video_presets;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const _: (log::LevelFilter, tracing::level_filters::LevelFilter) = (
    log::STATIC_MAX_LEVEL,
    tracing::level_filters::STATIC_MAX_LEVEL,
);

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

static SILENT_MODE: AtomicBool = AtomicBool::new(false);

fn write_crash_log(text: &str) -> Option<PathBuf> {
    let stamp = gameinfo_presets::format_timestamp_now();
    let payload = format!(
        "[{stamp}] ShaurMeaw v{} ({} / {})\r\n{}\r\n----------------------------------------\r\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        text.trim()
    );

    let candidates = [
        settings::Settings::crash_log_path(),
        std::env::temp_dir().join("ShaurMeaw_crash.log"),
    ];
    for path in candidates {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let existing_bytes = fs::read(&path).unwrap_or_default();
        let kept_bytes = if existing_bytes.len() > 64 * 1024 {
            let start = existing_bytes.len().saturating_sub(32 * 1024);
            let tail = existing_bytes.get(start..).unwrap_or(&[]);
            if let Some(nl) = tail.iter().position(|&b| b == b'\n') {
                tail.get(nl + 1..).unwrap_or(tail)
            } else {
                tail
            }
        } else {
            existing_bytes.as_slice()
        };
        let mut combined = String::from_utf8_lossy(kept_bytes).into_owned();
        combined.push_str(&payload);
        if fs_safe::atomic_write_with_retry(&path, combined.as_bytes()).is_ok() {
            return Some(path);
        }
    }
    None
}

fn show_error_dialog(message: &str) {
    if SILENT_MODE.load(Ordering::Relaxed) {
        return;
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("ShaurMeaw — ошибка запуска")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Неизвестная ошибка выполнения".to_owned()
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "неизвестное место".to_owned());

        let log_body = format!("Критическая ошибка ({location}): {payload}");
        let log_path = write_crash_log(&log_body);

        let mut user_msg = format!(
            "Произошла критическая ошибка приложения:\n{payload}\n({location})\n\nПопробуйте запустить ShaurMeaw.exe --software или обновить драйвер видеокарты."
        );
        if let Some(p) = log_path {
            user_msg.push_str(&format!("\n\nЛог ошибки сохранён:\n{}", p.display()));
        }
        show_error_dialog(&user_msg);
    }));
}

fn fit_window_to_work_area(work_w: f32, work_h: f32) -> [f32; 2] {
    if work_w <= 100.0 || work_h <= 100.0 {
        return [1024.0, 680.0];
    }
    let w = (work_w * 0.92).min(1080.0).max(640.0).min(work_w);
    let h = (work_h * 0.90).min(720.0).max(440.0).min(work_h);
    [w, h]
}

fn initial_window_size() -> [f32; 2] {
    #[cfg(windows)]
    {
        use std::ffi::c_void;

        #[repr(C)]
        #[derive(Default)]
        struct WinRect {
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
        }

        const SPI_GETWORKAREA: u32 = 0x0030;
        const LOGPIXELSX: i32 = 88;

        #[link(name = "user32")]
        extern "system" {
            fn SystemParametersInfoW(
                ui_action: u32,
                ui_param: u32,
                pv_param: *mut c_void,
                f_win_ini: u32,
            ) -> i32;
            fn GetDC(h_wnd: *mut c_void) -> *mut c_void;
            fn ReleaseDC(h_wnd: *mut c_void, h_dc: *mut c_void) -> i32;
        }
        #[link(name = "gdi32")]
        extern "system" {
            fn GetDeviceCaps(hdc: *mut c_void, index: i32) -> i32;
        }

        unsafe {
            let mut rect = WinRect::default();
            if SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                std::ptr::from_mut(&mut rect).cast::<c_void>(),
                0,
            ) != 0
            {
                let hdc = GetDC(std::ptr::null_mut());
                let dpi = if !hdc.is_null() {
                    let raw = GetDeviceCaps(hdc, LOGPIXELSX);
                    ReleaseDC(std::ptr::null_mut(), hdc);
                    if raw >= 72 {
                        raw as f32
                    } else {
                        96.0
                    }
                } else {
                    96.0
                };
                let scale = dpi / 96.0;
                let work_w = (rect.right - rect.left) as f32 / scale;
                let work_h = (rect.bottom - rect.top) as f32 / scale;
                return fit_window_to_work_area(work_w, work_h);
            }
        }
    }

    fit_window_to_work_area(0.0, 0.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RendererMode {
    GlowHw,
    GlowGl120,
    GlowSw,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RendererAttempt {
    label: &'static str,
    mode: RendererMode,
}

fn renderer_plan_from_args<'a>(args: impl Iterator<Item = &'a str>) -> Vec<RendererAttempt> {
    let mut software = false;
    let mut gl120 = false;

    for arg in args {
        match arg {
            "--software" => software = true,
            "--gl120" => gl120 = true,
            _ => {}
        }
    }

    let glow_hw = RendererAttempt {
        label: "OpenGL (аппаратный)",
        mode: RendererMode::GlowHw,
    };
    let glow_120 = RendererAttempt {
        label: "OpenGL (совместимый Gl120)",
        mode: RendererMode::GlowGl120,
    };
    let glow_sw = RendererAttempt {
        label: "OpenGL (программный)",
        mode: RendererMode::GlowSw,
    };

    if software {
        vec![glow_sw, glow_120, glow_hw]
    } else if gl120 {
        vec![glow_120, glow_hw, glow_sw]
    } else {
        vec![glow_hw, glow_120, glow_sw]
    }
}

fn renderer_plan() -> Vec<RendererAttempt> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    renderer_plan_from_args(args.iter().map(String::as_str))
}

fn load_and_resolve_settings() -> settings::Settings {
    let mut s = settings::Settings::load();

    if !steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
        if let Some(p) = steam::find_steam_path() {
            s.steam_path = p.display().to_string();
        }
    }
    if !steam::is_valid_game_dir(Path::new(&s.game_path)) {
        let steam_hint = Path::new(&s.steam_path);
        let hint = (!s.steam_path.trim().is_empty() && steam_hint.exists()).then_some(steam_hint);
        if let Some(g) = steam::find_deadlock_anywhere(hint) {
            s.game_path = g.display().to_string();
            if !steam::is_valid_steam_dir(Path::new(&s.steam_path)) {
                if let Some(inferred) = steam::infer_steam_from_game(&g) {
                    s.steam_path = inferred.display().to_string();
                }
            }
        }
    }
    s
}

fn main() {
    install_panic_hook();
    updater::cleanup_old_binary();

    if std::env::args().any(|a| a == "--sync-gameinfo") {
        SILENT_MODE.store(true, Ordering::Relaxed);
        gameinfo_presets::ensure_default_presets();
        let s = load_and_resolve_settings();
        let _ = gameinfo_sync::check_and_adapt_if_patched(&s);
        return;
    }

    if std::env::args().any(|a| a == "--play") {
        let s = load_and_resolve_settings();

        if s.gameinfo.enabled && s.gameinfo.rebuild_before_launch {
            gameinfo_presets::ensure_default_presets();
            if let Err(e) = gameinfo_sync::check_and_adapt_if_patched(&s) {
                show_error_dialog(&e);
                std::process::exit(1);
            }
        }
        if s.write_autoexec_on_launch {
            autoexec_presets::ensure_default_presets();
            if let Err(e) = launcher::write_autoexec(&s) {
                show_error_dialog(&e);
                std::process::exit(1);
            }
        }
        if s.write_video_on_launch {
            video_presets::ensure_default_presets();
            if let Err(e) = video_presets::apply_video_preset(&s, &s.video_preset) {
                show_error_dialog(&e);
                std::process::exit(1);
            }
        }
        if s.auto_clean_console_log {
            let clog = game_booster::console_log_path(Path::new(&s.game_path));
            let _ = game_booster::clean_console_log_if_oversized(
                &clog,
                game_booster::CONSOLE_LOG_MAX_BYTES,
                tools::is_deadlock_running(),
            );
        }
        if let Err(e) = launcher::launch(&s) {
            show_error_dialog(&e);
            std::process::exit(1);
        }
        return;
    }

    let is_post_update = std::env::args().any(|a| a == "--post-update");
    let mutex_name = "Local\\ShaurMeaw_DeadlockLauncher_SingleInstance_v1";
    let mut single_guard = single_instance::SingleInstanceGuard::acquire(mutex_name);
    if single_guard.is_none() && is_post_update {
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            single_guard = single_instance::SingleInstanceGuard::acquire(mutex_name);
            if single_guard.is_some() {
                break;
            }
        }
    }
    let _single_instance = match single_guard {
        Some(guard) => guard,
        None => {
            single_instance::focus_existing_window("ShaurMeaw — Deadlock Launcher", "ShaurMeaw");
            return;
        }
    };

    let icon = egui::IconData {
        rgba: embedded::icon_128_rgba().to_vec(),
        width: 128,
        height: 128,
    };

    let win_size = initial_window_size();
    let plan = renderer_plan();
    let mut errors = Vec::new();
    let mut icon_opt = Some(icon);

    for (idx, attempt) in plan.iter().enumerate() {
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("ShaurMeaw — Deadlock Launcher")
            .with_inner_size(win_size)
            .with_min_inner_size([640.0, 440.0]);
        let next_icon = if idx + 1 == plan.len() {
            icon_opt.take()
        } else {
            icon_opt.clone()
        };
        if let Some(ic) = next_icon {
            viewport = viewport.with_icon(ic);
        }

        let mut options = eframe::NativeOptions {
            viewport,
            centered: true,
            multisampling: 0,
            depth_buffer: 0,
            stencil_buffer: 0,
            dithering: false,
            run_and_return: true,
            ..Default::default()
        };

        match attempt.mode {
            RendererMode::GlowHw => {
                options.renderer = eframe::Renderer::Glow;
                options.hardware_acceleration = eframe::HardwareAcceleration::Preferred;
                options.shader_version = None;
            }
            RendererMode::GlowGl120 => {
                options.renderer = eframe::Renderer::Glow;
                options.hardware_acceleration = eframe::HardwareAcceleration::Preferred;
                options.shader_version = Some(eframe::egui_glow::ShaderVersion::Gl120);
            }
            RendererMode::GlowSw => {
                options.renderer = eframe::Renderer::Glow;
                options.hardware_acceleration = eframe::HardwareAcceleration::Off;
                options.shader_version = None;
            }
        }

        match eframe::run_native(
            "ShaurMeaw",
            options,
            Box::new(|cc| Ok(Box::new(app::LauncherApp::new(cc)))),
        ) {
            Ok(()) => return,
            Err(e) => errors.push(format!("{}: {e}", attempt.label)),
        }
    }

    let details = errors.join("\n");
    let log_path = write_crash_log(&format!(
        "Не удалось инициализировать ни один графический бэкенд:\n{details}"
    ));

    let mut msg = format!(
        "Не удалось открыть окно ShaurMeaw ни через один графический режим (OpenGL / Gl120 / программный):\n\n{details}\n\nРекомендации:\n1. Обновите драйвер видеокарты.\n2. Попробуйте запуск с ключом: ShaurMeaw.exe --software или --gl120"
    );
    if let Some(p) = log_path {
        msg.push_str(&format!("\n\nПодробный лог:\n{}", p.display()));
    }
    show_error_dialog(&msg);
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fit_window_to_work_area_laptop_and_desktop() {
        assert_eq!(fit_window_to_work_area(1920.0, 1040.0), [1080.0, 720.0]);
        assert_eq!(fit_window_to_work_area(1366.0, 728.0), [1080.0, 728.0 * 0.90]);
        assert_eq!(fit_window_to_work_area(1092.0, 582.0), [1004.64, 582.0 * 0.90]);
        assert_eq!(fit_window_to_work_area(910.0, 480.0), [837.2, 440.0]);
        assert_eq!(fit_window_to_work_area(0.0, 0.0), [1024.0, 680.0]);
    }

    #[test]
    fn test_renderer_plan_order_for_flags() {
        let def: Vec<_> = renderer_plan_from_args(std::iter::empty())
            .into_iter()
            .map(|a| a.mode)
            .collect();
        assert_eq!(
            def,
            vec![
                RendererMode::GlowHw,
                RendererMode::GlowGl120,
                RendererMode::GlowSw,
            ]
        );

        let sw: Vec<_> = renderer_plan_from_args(["--software"].into_iter())
            .into_iter()
            .map(|a| a.mode)
            .collect();
        assert_eq!(
            sw,
            vec![
                RendererMode::GlowSw,
                RendererMode::GlowGl120,
                RendererMode::GlowHw,
            ]
        );

        let gl120: Vec<_> = renderer_plan_from_args(["--gl120"].into_iter())
            .into_iter()
            .map(|a| a.mode)
            .collect();
        assert_eq!(
            gl120,
            vec![
                RendererMode::GlowGl120,
                RendererMode::GlowHw,
                RendererMode::GlowSw,
            ]
        );
    }
}
