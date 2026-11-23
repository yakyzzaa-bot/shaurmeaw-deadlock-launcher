//! Утилиты: мониторинг процессов Steam/Deadlock, очистка кэша/дампов, бэкап и профили.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::game_booster::{self, wait_event_or_dirs, WaitOutcome, WinEvent};
use crate::settings::Settings;
use crate::steam;

/// Возвращает корневую папку Windows (`%SystemRoot%` / `%WINDIR%`), обычно `C:\Windows` или `E:\Windows`.
pub fn windows_root() -> PathBuf {
    for var in ["SystemRoot", "WINDIR"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    PathBuf::from(r"C:\Windows")
}

/// Возвращает полный путь к утилите в корне Windows (например, `explorer.exe`).
pub fn windir_exe(name: &str) -> PathBuf {
    let candidate = windows_root().join(name);
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from(name)
    }
}

/// Фоновый мониторинг запущенных процессов Steam и Deadlock (`deadlock.exe` + fallback `project8.exe`).
pub struct ProcessMonitor {
    steam_running: Arc<AtomicBool>,
    game_running: Arc<AtomicBool>,
    game_pid: Arc<AtomicU32>,
    boosted_pid: Arc<AtomicU32>,
    affinity_pid: Arc<AtomicU32>,
    auto_high_priority: Arc<AtomicBool>,
    ram_trim_on_game: Arc<AtomicBool>,
    boost_timer_1ms: Arc<AtomicBool>,
    boost_power_plan: Arc<AtomicBool>,
    boost_disable_cpu0: Arc<AtomicBool>,
    auto_english_layout: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    wake: Option<Arc<WinEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ProcessMonitor {
    pub fn spawn(
        ctx: eframe::egui::Context,
        initial: &Settings,
        window_minimized: Arc<AtomicBool>,
        game_running: Arc<AtomicBool>,
        extra_wakes: Vec<Arc<WinEvent>>,
    ) -> Self {
        let steam_running = Arc::new(AtomicBool::new(false));
        let game_pid = Arc::new(AtomicU32::new(0));
        let boosted_pid = Arc::new(AtomicU32::new(0));
        let affinity_pid = Arc::new(AtomicU32::new(0));
        let auto_high_priority = Arc::new(AtomicBool::new(initial.auto_high_priority));
        let ram_trim_on_game = Arc::new(AtomicBool::new(initial.ram_trim_on_minimize));
        let boost_timer_1ms = Arc::new(AtomicBool::new(initial.boost_timer_1ms));
        let boost_power_plan = Arc::new(AtomicBool::new(initial.boost_power_plan));
        let boost_disable_cpu0 = Arc::new(AtomicBool::new(initial.boost_disable_cpu0));
        let auto_english_layout = Arc::new(AtomicBool::new(initial.auto_english_layout));
        let shutdown = Arc::new(AtomicBool::new(false));
        let wake = WinEvent::new().map(Arc::new);

        let s_flag = Arc::clone(&steam_running);
        let g_flag = Arc::clone(&game_running);
        let pid_flag = Arc::clone(&game_pid);
        let boost_flag = Arc::clone(&boosted_pid);
        let affinity_flag = Arc::clone(&affinity_pid);
        let prio_flag = Arc::clone(&auto_high_priority);
        let trim_flag = Arc::clone(&ram_trim_on_game);
        let timer_flag = Arc::clone(&boost_timer_1ms);
        let power_flag = Arc::clone(&boost_power_plan);
        let cpu0_flag = Arc::clone(&boost_disable_cpu0);
        let en_flag = Arc::clone(&auto_english_layout);
        let min_flag = Arc::clone(&window_minimized);
        let stop_flag = Arc::clone(&shutdown);
        let wake_bg = wake.as_ref().map(Arc::clone);

        let worker = std::thread::Builder::new()
            .name("shaurmeaw-procs".to_owned())
            .spawn(move || {
                let mut prev_steam = false;
                let mut prev_game = false;
                let mut cached_steam_pid = 0u32;
                let mut cached_game_pid = 0u32;
                let mut last_boosted_pid = 0u32;
                let mut last_affinity_pid = 0u32;
                let mut power_checked_pid = 0u32;
                let mut layout_switched_pid = 0u32;
                let mut prev_want_power = false;

                loop {
                    if stop_flag.load(Ordering::Relaxed) {
                        if last_affinity_pid != 0 {
                            let _ = game_booster::restore_process_affinity(last_affinity_pid);
                        }
                        game_booster::restore_all_runtime_boosts();
                        break;
                    }

                    let (steam, pid_opt) =
                        check_pids_or_scan(&mut cached_steam_pid, &mut cached_game_pid);
                    let game = pid_opt.is_some();
                    let pid = pid_opt.unwrap_or(0);
                    let want_boost = prio_flag.load(Ordering::Relaxed);
                    let want_timer = timer_flag.load(Ordering::Relaxed);
                    let want_power = power_flag.load(Ordering::Relaxed);
                    let want_no_cpu0 = cpu0_flag.load(Ordering::Relaxed);

                    let mut boost_changed = false;
                    if pid == 0 {
                        layout_switched_pid = 0;
                        if last_boosted_pid != 0
                            || last_affinity_pid != 0
                            || power_checked_pid != 0
                            || game_booster::is_launcher_throttled()
                            || game_booster::is_high_res_timer_active()
                            || game_booster::is_power_plan_boosted()
                        {
                            last_boosted_pid = 0;
                            last_affinity_pid = 0;
                            power_checked_pid = 0;
                            prev_want_power = false;
                            boost_flag.store(0, Ordering::Relaxed);
                            affinity_flag.store(0, Ordering::Relaxed);
                            game_booster::restore_all_runtime_boosts();
                            boost_changed = true;
                        } else {
                            prev_want_power = false;
                        }
                    } else {
                        if want_boost {
                            if pid != last_boosted_pid && game_booster::set_process_high_priority(pid)
                            {
                                last_boosted_pid = pid;
                                boost_flag.store(pid, Ordering::Relaxed);
                                game_booster::set_launcher_low_priority();
                                boost_changed = true;
                            } else if last_boosted_pid == pid
                                && !game_booster::is_launcher_throttled()
                            {
                                game_booster::set_launcher_low_priority();
                                boost_changed = true;
                            }
                        } else if last_boosted_pid != 0 || game_booster::is_launcher_throttled() {
                            last_boosted_pid = 0;
                            boost_flag.store(0, Ordering::Relaxed);
                            game_booster::restore_launcher_priority();
                            boost_changed = true;
                        }

                        if want_timer {
                            if !game_booster::is_high_res_timer_active()
                                && game_booster::enable_high_res_timer()
                            {
                                boost_changed = true;
                            }
                        } else if game_booster::is_high_res_timer_active() {
                            game_booster::disable_high_res_timer();
                            boost_changed = true;
                        }

                        if want_power {
                            if power_checked_pid != pid || !prev_want_power {
                                let before_power = game_booster::is_power_plan_boosted();
                                let _ = game_booster::enable_high_perf_power_plan();
                                power_checked_pid = pid;
                                if game_booster::is_power_plan_boosted() != before_power {
                                    boost_changed = true;
                                }
                            }
                        } else {
                            power_checked_pid = 0;
                            if game_booster::is_power_plan_boosted() {
                                let _ = game_booster::restore_power_plan();
                                boost_changed = true;
                            }
                        }
                        prev_want_power = want_power;

                        if want_no_cpu0 {
                            if pid != last_affinity_pid
                                && game_booster::optimize_process_affinity_no_cpu0(pid)
                            {
                                last_affinity_pid = pid;
                                affinity_flag.store(pid, Ordering::Relaxed);
                                boost_changed = true;
                            }
                        } else if last_affinity_pid != 0 {
                            let _ = game_booster::restore_process_affinity(last_affinity_pid);
                            last_affinity_pid = 0;
                            affinity_flag.store(0, Ordering::Relaxed);
                            boost_changed = true;
                        }

                        if en_flag.load(Ordering::Relaxed)
                            && layout_switched_pid != pid
                            && game_booster::switch_process_window_to_english_layout(pid)
                        {
                            layout_switched_pid = pid;
                        }
                    }

                    // Сразу после запуска игры сжимаем рабочий набор ОЗУ лаунчера
                    if !prev_game && game && trim_flag.load(Ordering::Relaxed) {
                        game_booster::trim_memory();
                    }

                    s_flag.store(steam, Ordering::Relaxed);
                    g_flag.store(game, Ordering::Release);
                    pid_flag.store(pid, Ordering::Relaxed);

                    if game != prev_game {
                        for w in &extra_wakes {
                            w.signal();
                        }
                    }

                    if steam != prev_steam || game != prev_game || boost_changed {
                        prev_steam = steam;
                        prev_game = game;
                        if !min_flag.load(Ordering::Relaxed) {
                            ctx.request_repaint();
                        }
                    } else {
                        prev_game = game;
                    }

                    if stop_flag.load(Ordering::Relaxed) {
                        if last_affinity_pid != 0 {
                            let _ = game_booster::restore_process_affinity(last_affinity_pid);
                        }
                        game_booster::restore_all_runtime_boosts();
                        break;
                    }

                    // Адаптивный сон: 3 с во время матча (O(1) PID check), 4 с в простое, 6 с при свёрнутом окне
                    let sleep_ms = if game {
                        3000
                    } else if min_flag.load(Ordering::Relaxed) {
                        6000
                    } else {
                        4000
                    };
                    if let Some(ref ev) = wake_bg {
                        if matches!(wait_event_or_dirs(ev, &[], sleep_ms), WaitOutcome::Error) {
                            std::thread::sleep(Duration::from_millis(u64::from(sleep_ms)));
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(u64::from(sleep_ms)));
                    }
                }
            })
            .ok();

        Self {
            steam_running,
            game_running,
            game_pid,
            boosted_pid,
            affinity_pid,
            auto_high_priority,
            ram_trim_on_game,
            boost_timer_1ms,
            boost_power_plan,
            boost_disable_cpu0,
            auto_english_layout,
            shutdown,
            wake,
            worker,
        }
    }

    pub fn set_booster_options(
        &self,
        auto_prio: bool,
        ram_trim: bool,
        timer_1ms: bool,
        power_plan: bool,
        disable_cpu0: bool,
    ) {
        self.auto_high_priority.store(auto_prio, Ordering::Relaxed);
        self.ram_trim_on_game.store(ram_trim, Ordering::Relaxed);
        self.boost_timer_1ms.store(timer_1ms, Ordering::Relaxed);
        self.boost_power_plan.store(power_plan, Ordering::Relaxed);
        self.boost_disable_cpu0.store(disable_cpu0, Ordering::Relaxed);
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    /// Обновляет флаг автопереключения раскладки игры на английскую (EN).
    pub fn set_auto_english_layout(&self, enabled: bool) {
        self.auto_english_layout.store(enabled, Ordering::Relaxed);
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    /// Будит монитор сразу после изменения состояния окна / запуска игры.
    pub fn wake_now(&self) {
        if let Some(ref w) = self.wake {
            w.signal();
        }
    }

    pub fn is_steam_running(&self) -> bool {
        self.steam_running.load(Ordering::Relaxed)
    }

    pub fn is_game_running(&self) -> bool {
        self.game_running.load(Ordering::Relaxed)
    }

    pub fn is_priority_boosted(&self) -> bool {
        let cur = self.game_pid.load(Ordering::Relaxed);
        cur != 0 && self.boosted_pid.load(Ordering::Relaxed) == cur
    }

    pub fn is_affinity_optimized(&self) -> bool {
        let cur = self.game_pid.load(Ordering::Relaxed);
        cur != 0 && self.affinity_pid.load(Ordering::Relaxed) == cur
    }

    pub fn game_pid(&self) -> Option<u32> {
        let pid = self.game_pid.load(Ordering::Relaxed);
        (pid != 0).then_some(pid)
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(ref w) = self.wake {
            w.signal();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        game_booster::restore_all_runtime_boosts();
    }
}

impl Drop for ProcessMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(windows)]
mod win_procs {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const INVALID_HANDLE_VALUE: *mut c_void = (-1isize) as *mut c_void;

    #[repr(C)]
    pub(super) struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
        fn Process32FirstW(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> *mut c_void;
        fn GetExitCodeProcess(h_process: *mut c_void, lp_exit_code: *mut u32) -> i32;
        fn TerminateProcess(h_process: *mut c_void, u_exit_code: u32) -> i32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
    }

    struct WinHandle(*mut c_void);

    impl WinHandle {
        fn new(raw: *mut c_void) -> Option<Self> {
            (!raw.is_null() && raw != INVALID_HANDLE_VALUE).then_some(Self(raw))
        }
    }

    impl Drop for WinHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// O(1)-проверка, жив ли процесс с данным PID без перебора всех процессов в системе.
    pub(super) fn is_pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let Some(h) = WinHandle::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))
            else {
                return false;
            };
            let mut exit_code = 0u32;
            GetExitCodeProcess(h.0, &raw mut exit_code) != 0 && exit_code == STILL_ACTIVE
        }
    }

    /// O(tlen) zero-allocation сравнение UTF-16 имени (до NUL) с ASCII-строкой в нижнем регистре.
    pub(super) fn eq_utf16_ascii(wide: &[u16], target_ascii_lower: &[u8]) -> bool {
        let tlen = target_ascii_lower.len();
        if wide.len() < tlen || wide.get(tlen).is_some_and(|&c| c != 0) {
            return false;
        }
        wide[..tlen]
            .iter()
            .zip(target_ascii_lower)
            .all(|(&w, &b)| w <= 0x7F && (w as u8).to_ascii_lowercase() == b)
    }

    /// Перебирает запущенные процессы через Toolhelp32 API; остановка при возврате `false` из `visitor`.
    fn for_each_process(mut visitor: impl FnMut(&ProcessEntry32W) -> bool) -> bool {
        unsafe {
            let Some(snap) = WinHandle::new(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0))
            else {
                return false;
            };

            let mut entry: ProcessEntry32W = zeroed();
            entry.dw_size = size_of::<ProcessEntry32W>() as u32;

            if Process32FirstW(snap.0, &mut entry) != 0 {
                loop {
                    if !visitor(&entry) || Process32NextW(snap.0, &mut entry) == 0 {
                        break;
                    }
                }
            }
        }
        true
    }

    /// Опрашивает процессы через Toolhelp32: возвращает `(steam_pid, deadlock_or_project8_pid)`.
    pub(super) fn query_processes_pids() -> (Option<u32>, Option<u32>) {
        let mut steam_pid: Option<u32> = None;
        let mut deadlock_pid: Option<u32> = None;
        let mut fallback_pid: Option<u32> = None;

        for_each_process(|entry| {
            if steam_pid.is_none() && eq_utf16_ascii(&entry.sz_exe_file, b"steam.exe") {
                steam_pid = Some(entry.th32_process_id);
            } else if deadlock_pid.is_none()
                && eq_utf16_ascii(&entry.sz_exe_file, b"deadlock.exe")
            {
                deadlock_pid = Some(entry.th32_process_id);
            } else if fallback_pid.is_none()
                && eq_utf16_ascii(&entry.sz_exe_file, b"project8.exe")
            {
                fallback_pid = Some(entry.th32_process_id);
            }
            !(steam_pid.is_some() && deadlock_pid.is_some())
        });

        (steam_pid, deadlock_pid.or(fallback_pid))
    }

    /// Опрашивает процессы: `deadlock.exe` в приоритете и `project8.exe` как fallback.
    pub(super) fn query_processes() -> (bool, Option<u32>) {
        let (steam_pid, game_pid) = query_processes_pids();
        (steam_pid.is_some(), game_pid)
    }

    /// Пока игра запущена, проверяет её PID за O(1) без дорогого `CreateToolhelp32Snapshot`.
    pub(super) fn check_pids_or_scan(
        steam_pid: &mut u32,
        game_pid: &mut u32,
    ) -> (bool, Option<u32>) {
        if *game_pid != 0 && is_pid_alive(*game_pid) {
            let steam_alive = *steam_pid != 0 && is_pid_alive(*steam_pid);
            if !steam_alive {
                *steam_pid = 0;
            }
            return (steam_alive, Some(*game_pid));
        }
        let (found_steam, found_game) = query_processes_pids();
        *steam_pid = found_steam.unwrap_or(0);
        *game_pid = found_game.unwrap_or(0);
        (found_steam.is_some(), found_game)
    }

    /// Нативно завершает все найденные процессы с заданным именем через `OpenProcess` + `TerminateProcess`.
    pub(super) fn terminate_process_by_name(target_ascii_lower: &[u8]) -> Result<usize, String> {
        let mut killed = 0usize;
        if !for_each_process(|entry| {
            if eq_utf16_ascii(&entry.sz_exe_file, target_ascii_lower) && entry.th32_process_id != 0
            {
                unsafe {
                    if let Some(h) = WinHandle::new(OpenProcess(
                        PROCESS_TERMINATE,
                        0,
                        entry.th32_process_id,
                    )) {
                        if TerminateProcess(h.0, 1) != 0 {
                            killed += 1;
                        }
                    }
                }
            }
            true
        }) {
            return Err("Не удалось получить снимок процессов Windows".into());
        }

        if killed > 0 {
            Ok(killed)
        } else {
            Err("Не удалось закрыть Deadlock (возможно, игра уже закрыта)".into())
        }
    }
}

#[cfg(windows)]
fn query_processes() -> (bool, Option<u32>) {
    win_procs::query_processes()
}

#[cfg(not(windows))]
fn query_processes() -> (bool, Option<u32>) {
    (false, None)
}

#[cfg(windows)]
fn check_pids_or_scan(steam_pid: &mut u32, game_pid: &mut u32) -> (bool, Option<u32>) {
    win_procs::check_pids_or_scan(steam_pid, game_pid)
}

#[cfg(not(windows))]
fn check_pids_or_scan(_steam_pid: &mut u32, _game_pid: &mut u32) -> (bool, Option<u32>) {
    (false, None)
}

/// Проверяет, запущен ли Deadlock прямо сейчас (`deadlock.exe` или `project8.exe`).
pub fn is_deadlock_running() -> bool {
    query_processes().1.is_some()
}

/// Нативно завершает процесс Deadlock (`deadlock.exe` с fallback на `project8.exe`).
pub fn kill_deadlock() -> Result<String, String> {
    #[cfg(windows)]
    {
        if win_procs::terminate_process_by_name(b"deadlock.exe").is_ok() {
            return Ok("Процесс Deadlock (deadlock.exe) завершён".into());
        }
        if win_procs::terminate_process_by_name(b"project8.exe").is_ok() {
            return Ok("Процесс Deadlock (project8.exe) завершён".into());
        }
        Err("Не удалось закрыть Deadlock (возможно, игра уже закрыта)".into())
    }
    #[cfg(not(windows))]
    {
        Err("Завершение процесса поддерживается только в Windows".into())
    }
}

// ============================================================ кэш и дампы

#[derive(Default, Clone, Debug)]
pub struct CacheStats {
    pub shader_bytes: u64,
    pub shader_files: usize,
    pub dump_bytes: u64,
    pub dump_files: usize,
    pub shader_vpk_updated: Option<String>,
}

/// Возвращает папки кэша шейдеров Deadlock во всех библиотеках Steam и рядом с игрой.
fn shader_cache_dirs(steam_path: &Path, game_path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_exists = |dir: PathBuf| {
        if dir.is_dir() && !out.contains(&dir) {
            out.push(dir);
        }
    };

    if steam_path.exists() {
        for lib in steam::library_folders(steam_path) {
            push_if_exists(lib.join("steamapps").join("shadercache").join(steam::APP_ID));
        }
    }
    if let Some(steamapps) = game_path.parent().and_then(|p| p.parent()) {
        push_if_exists(steamapps.join("shadercache").join(steam::APP_ID));
    }
    out
}

/// Возвращает дату последнего обновления `shaders_pc_000.vpk` / `shaders_vulkan_000.vpk`.
fn latest_shader_vpk_label(game_path: &Path) -> Option<String> {
    let citadel = game_path.join("game").join("citadel");
    let mut latest: Option<SystemTime> = None;
    for name in ["shaders_pc_000.vpk", "shaders_vulkan_000.vpk"] {
        if let Ok(meta) = fs::metadata(citadel.join(name)) {
            if let Ok(mod_t) = meta.modified() {
                if latest.is_none_or(|prev| mod_t > prev) {
                    latest = Some(mod_t);
                }
            }
        }
    }
    latest.map(crate::gameinfo_presets::format_system_time_human)
}

/// Возвращает список файлов краш-дампов (`*.mdmp`) в корне игры и `game\bin\win64`.
fn crash_dump_files(game_path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [
        game_path.to_path_buf(),
        game_path.join("game").join("bin").join("win64"),
    ] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("mdmp"))
                {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn dir_size(path: &Path) -> (u64, usize) {
    let mut bytes = 0u64;
    let mut files = 0usize;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let (b, f) = dir_size(&p);
                bytes += b;
                files += f;
            } else if let Ok(meta) = entry.metadata() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// Подсчитывает размер кэша шейдеров, краш-дампов и дату обновления VPK-шейдеров Valve.
pub fn scan_cache(steam_path: &Path, game_path: &Path) -> CacheStats {
    let ((shader_vpk_updated, (shader_bytes, shader_files)), (dump_bytes, dump_files)) =
        std::thread::scope(|s| {
            let shader_job = s.spawn(|| {
                let label = latest_shader_vpk_label(game_path);
                let mut bytes = 0u64;
                let mut files = 0usize;
                for dir in shader_cache_dirs(steam_path, game_path) {
                    let (b, f) = dir_size(&dir);
                    bytes += b;
                    files += f;
                }
                (label, (bytes, files))
            });
            let dumps = {
                let mut bytes = 0u64;
                let mut files = 0usize;
                for dump in crash_dump_files(game_path) {
                    if let Ok(meta) = fs::metadata(&dump) {
                        bytes += meta.len();
                        files += 1;
                    }
                }
                (bytes, files)
            };
            (shader_job.join().unwrap_or((None, (0, 0))), dumps)
        });

    CacheStats {
        shader_bytes,
        shader_files,
        dump_bytes,
        dump_files,
        shader_vpk_updated,
    }
}

fn clear_shader_cache_in_dirs(dirs: &[PathBuf], game_running: bool) -> Result<String, String> {
    if game_running {
        return Err(
            "Нельзя очищать кэш шейдеров, пока запущен Deadlock. Закройте игру и повторите попытку."
                .into(),
        );
    }
    if dirs.is_empty() {
        return Ok("Кэш шейдеров уже пуст".into());
    }

    let mut freed = 0u64;
    for dir in dirs {
        let (b, _) = dir_size(dir);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let res = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
                if let Err(e) = res {
                    return Err(format!("Не удалось удалить {}: {e}", p.display()));
                }
            }
        }
        freed += b;
    }

    Ok(format!(
        "Кэш шейдеров Vulkan/DX11 очищен (освобождено {})",
        format_size(freed)
    ))
}

/// Безопасно очищает кэш шейдеров Deadlock (проверяет, что игра не запущена).
pub fn clear_shader_cache(steam_path: &Path, game_path: &Path) -> Result<String, String> {
    let dirs = shader_cache_dirs(steam_path, game_path);
    clear_shader_cache_in_dirs(&dirs, is_deadlock_running())
}

/// Удаляет файлы краш-дампов (`*.mdmp`).
pub fn clear_crash_dumps(game_path: &Path) -> Result<String, String> {
    let dumps = crash_dump_files(game_path);
    if dumps.is_empty() {
        return Ok("Краш-дампы не найдены".into());
    }

    let mut freed = 0u64;
    let mut count = 0usize;
    for dump in dumps {
        if let Ok(meta) = fs::metadata(&dump) {
            freed += meta.len();
        }
        fs::remove_file(&dump)
            .map_err(|e| format!("Не удалось удалить {}: {e}", dump.display()))?;
        count += 1;
    }

    Ok(format!(
        "Удалено дампов: {count} (освобождено {})",
        format_size(freed)
    ))
}

pub fn format_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} МБ", bytes as f64 / MB)
    } else if bytes as f64 >= KB {
        format!("{:.0} КБ", bytes as f64 / KB)
    } else {
        format!("{bytes} Б")
    }
}

// ============================================================ бэкап autoexec

fn find_autoexec_backup(game_path: &Path) -> Option<PathBuf> {
    let in_cfg = steam::cfg_dir(game_path).join("autoexec.cfg.bak");
    if in_cfg.is_file() {
        return Some(in_cfg);
    }
    let in_backups = Settings::gameinfo_backups_dir().join("autoexec.cfg.bak");
    in_backups.is_file().then_some(in_backups)
}

pub fn has_autoexec_backup(game_path: &Path) -> bool {
    find_autoexec_backup(game_path).is_some()
}

pub fn restore_autoexec_backup(game_path: &Path) -> Result<String, String> {
    let dir = steam::cfg_dir(game_path);
    let dst = dir.join("autoexec.cfg");
    let bak = find_autoexec_backup(game_path)
        .ok_or_else(|| "Файл резервной копии autoexec.cfg.bak не найден".to_owned())?;
    let bytes = crate::fs_safe::read_bytes_with_retry(&bak)
        .map_err(|e| format!("Не удалось прочитать бэкап: {e}"))?;
    crate::fs_safe::atomic_write_with_retry(&dst, &bytes)
        .map_err(|e| format!("Не удалось восстановить бэкап: {e}"))?;
    Ok("Оригинальный autoexec.cfg восстановлен из бэкапа".into())
}

// ============================================================ профили и экспорт

fn sanitize_profile_name(raw: &str) -> Result<String, String> {
    crate::fs_safe::sanitize_filename(raw)
}

pub fn list_profiles() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(Settings::profiles_dir()) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_owned());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn save_profile(name: &str, settings: &Settings) -> Result<String, String> {
    let clean = sanitize_profile_name(name)?;
    let path = Settings::profiles_dir().join(format!("{clean}.json"));
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    crate::fs_safe::atomic_write_with_retry(&path, json.as_bytes())
        .map_err(|e| format!("Не удалось сохранить профиль: {e}"))?;
    Ok(clean)
}

pub fn load_profile(name: &str, current: &Settings) -> Result<Settings, String> {
    let clean = sanitize_profile_name(name)?;
    let path = Settings::profiles_dir().join(format!("{clean}.json"));
    let text = crate::fs_safe::read_text_lossy_with_retry(&path)
        .map_err(|e| format!("Не удалось прочитать профиль: {e}"))?;
    let mut loaded: Settings =
        serde_json::from_str(&text).map_err(|e| format!("Повреждённый файл профиля: {e}"))?;
    // Сохраняем текущие пути к Steam и игре, если в профиле они пустые или недействительные
    if loaded.steam_path.is_empty() || !Path::new(&loaded.steam_path).exists() {
        loaded.steam_path = current.steam_path.clone();
    }
    if loaded.game_path.is_empty() || !steam::is_valid_game_dir(Path::new(&loaded.game_path)) {
        loaded.game_path = current.game_path.clone();
    }
    Ok(loaded)
}

pub fn delete_profile(name: &str) -> Result<(), String> {
    let clean = sanitize_profile_name(name)?;
    let path = Settings::profiles_dir().join(format!("{clean}.json"));
    fs::remove_file(&path).map_err(|e| format!("Не удалось удалить профиль: {e}"))
}

pub fn export_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    crate::fs_safe::atomic_write_with_retry(path, json.as_bytes())
        .map_err(|e| format!("Не удалось экспортировать файл: {e}"))
}

pub fn import_settings(path: &Path, current: &Settings) -> Result<Settings, String> {
    let text = crate::fs_safe::read_text_lossy_with_retry(path)
        .map_err(|e| format!("Не удалось прочитать файл: {e}"))?;
    let mut loaded: Settings =
        serde_json::from_str(&text).map_err(|e| format!("Некорректный JSON настроек: {e}"))?;
    if loaded.steam_path.is_empty() || !Path::new(&loaded.steam_path).exists() {
        loaded.steam_path = current.steam_path.clone();
    }
    if loaded.game_path.is_empty() || !steam::is_valid_game_dir(Path::new(&loaded.game_path)) {
        loaded.game_path = current.game_path.clone();
    }
    Ok(loaded)
}

// ============================================================ ярлык рабочего стола (--play)

pub fn desktop_shortcut_path() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)?;
    let desktop = if profile.join("Desktop").is_dir() {
        profile.join("Desktop")
    } else if profile.join("OneDrive").join("Desktop").is_dir() {
        profile.join("OneDrive").join("Desktop")
    } else {
        profile.join("Desktop")
    };
    Some(desktop.join("Deadlock (ShaurMeaw).lnk"))
}

pub fn has_desktop_shortcut() -> bool {
    desktop_shortcut_path().is_some_and(|p| p.is_file())
}

#[cfg(windows)]
mod win_shortcut {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    // Системные идентификаторы COM из shlguid.h / objidl.h
    const CLSID_SHELL_LINK: Guid = Guid {
        data1: 0x0002_1401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_ISHELL_LINK_W: Guid = Guid {
        data1: 0x0002_14F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_IPERSIST_FILE: Guid = Guid {
        data1: 0x0000_010B,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[repr(C)]
    struct IShellLinkW {
        lp_vtbl: *const IShellLinkWVtbl,
    }

    #[repr(C)]
    struct IShellLinkWVtbl {
        query_interface: unsafe extern "system" fn(
            *mut IShellLinkW,
            *const Guid,
            *mut *mut c_void,
        ) -> i32,
        _add_ref: unsafe extern "system" fn(*mut IShellLinkW) -> u32,
        release: unsafe extern "system" fn(*mut IShellLinkW) -> u32,
        _get_path: *const c_void,
        _get_id_list: *const c_void,
        _set_id_list: *const c_void,
        _get_description: *const c_void,
        set_description: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> i32,
        _get_working_directory: *const c_void,
        set_working_directory: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> i32,
        _get_arguments: *const c_void,
        set_arguments: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> i32,
        _get_hotkey: *const c_void,
        _set_hotkey: *const c_void,
        _get_show_cmd: *const c_void,
        _set_show_cmd: *const c_void,
        _get_icon_location: *const c_void,
        set_icon_location: unsafe extern "system" fn(*mut IShellLinkW, *const u16, i32) -> i32,
        _set_relative_path: *const c_void,
        _resolve: *const c_void,
        set_path: unsafe extern "system" fn(*mut IShellLinkW, *const u16) -> i32,
    }

    #[repr(C)]
    struct IPersistFile {
        lp_vtbl: *const IPersistFileVtbl,
    }

    #[repr(C)]
    struct IPersistFileVtbl {
        _query_interface: *const c_void,
        _add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut IPersistFile) -> u32,
        _get_class_id: *const c_void,
        _is_dirty: *const c_void,
        _load: *const c_void,
        save: unsafe extern "system" fn(*mut IPersistFile, *const u16, i32) -> i32,
        _save_completed: *const c_void,
        _get_cur_file: *const c_void,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *mut c_void, dw_co_init: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const Guid,
            p_unk_outer: *mut c_void,
            dw_cls_context: u32,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> i32;
    }

    struct CoGuard {
        should_uninit: bool,
    }

    impl CoGuard {
        fn init() -> Result<Self, String> {
            let hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED) };
            // В GUI-потоке eframe/rfd COM уже может быть инициализирован с другим режимом (MTA/STA)
            if hr == RPC_E_CHANGED_MODE {
                return Ok(Self { should_uninit: false });
            }
            if hr < 0 {
                return Err(format!("CoInitializeEx ошибка: 0x{:08X}", hr as u32));
            }
            Ok(Self { should_uninit: true })
        }
    }

    impl Drop for CoGuard {
        fn drop(&mut self) {
            if self.should_uninit {
                unsafe { CoUninitialize() };
            }
        }
    }

    struct ShellLinkGuard(*mut IShellLinkW);

    impl Drop for ShellLinkGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { ((*(*self.0).lp_vtbl).release)(self.0) };
            }
        }
    }

    struct PersistFileGuard(*mut IPersistFile);

    impl Drop for PersistFileGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { ((*(*self.0).lp_vtbl).release)(self.0) };
            }
        }
    }

    fn to_wide_z(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    fn check_hr(hr: i32, stage: &str) -> Result<(), String> {
        if hr < 0 {
            Err(format!("Не удалось создать ярлык ({stage}: 0x{:08X})", hr as u32))
        } else {
            Ok(())
        }
    }

    /// Нативно создаёт `.lnk` через COM-интерфейсы `IShellLinkW` и `IPersistFile` без запуска PowerShell/WSH.
    pub(super) fn create_shortcut_at(
        lnk: &Path,
        exe: &Path,
        args: &str,
        wd: &Path,
        description: &str,
    ) -> Result<(), String> {
        let _co = CoGuard::init()?;

        let mut link_raw: *mut c_void = null_mut();
        check_hr(
            unsafe {
                CoCreateInstance(
                    &CLSID_SHELL_LINK,
                    null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_ISHELL_LINK_W,
                    &mut link_raw,
                )
            },
            "CoCreateInstance",
        )?;
        if link_raw.is_null() {
            return Err("COM вернул пустой указатель IShellLinkW".into());
        }
        let link = ShellLinkGuard(link_raw as *mut IShellLinkW);

        let exe_w = to_wide_z(exe.as_os_str());
        let args_w = to_wide_z(OsStr::new(args));
        let wd_w = to_wide_z(wd.as_os_str());
        let desc_w = to_wide_z(OsStr::new(description));
        let lnk_w = to_wide_z(lnk.as_os_str());

        unsafe {
            let vt = &*(*link.0).lp_vtbl;
            check_hr((vt.set_path)(link.0, exe_w.as_ptr()), "SetPath")?;
            check_hr((vt.set_arguments)(link.0, args_w.as_ptr()), "SetArguments")?;
            check_hr((vt.set_working_directory)(link.0, wd_w.as_ptr()), "SetWorkingDirectory")?;
            check_hr((vt.set_icon_location)(link.0, exe_w.as_ptr(), 0), "SetIconLocation")?;
            check_hr((vt.set_description)(link.0, desc_w.as_ptr()), "SetDescription")?;

            let mut pf_raw: *mut c_void = null_mut();
            check_hr((vt.query_interface)(link.0, &IID_IPERSIST_FILE, &mut pf_raw), "QueryInterface")?;
            if pf_raw.is_null() {
                return Err("COM вернул пустой указатель IPersistFile".into());
            }
            let pf = PersistFileGuard(pf_raw as *mut IPersistFile);
            let pf_vt = &*(*pf.0).lp_vtbl;
            check_hr((pf_vt.save)(pf.0, lnk_w.as_ptr(), 1), "IPersistFile::Save")?;
        }

        Ok(())
    }
}

pub fn create_desktop_shortcut() -> Result<PathBuf, String> {
    let lnk = desktop_shortcut_path()
        .ok_or_else(|| "Не удалось определить папку Рабочего стола (%USERPROFILE%)".to_owned())?;
    let exe = std::env::current_exe()
        .map_err(|e| format!("Не удалось определить путь к ShaurMeaw.exe: {e}"))?;
    let wd = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(parent) = lnk.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Не удалось создать папку Рабочего стола: {e}"))?;
    }

    #[cfg(windows)]
    {
        win_shortcut::create_shortcut_at(
            &lnk,
            &exe,
            "--play",
            &wd,
            "Запуск Deadlock через ShaurMeaw (автообновление конфигов)",
        )?;
        Ok(lnk)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, wd);
        Err("Создание ярлыков .lnk поддерживается только в Windows".into())
    }
}

pub fn remove_desktop_shortcut() -> Result<(), String> {
    if let Some(lnk) = desktop_shortcut_path() {
        if lnk.exists() {
            fs::remove_file(&lnk)
                .map_err(|e| format!("Не удалось удалить ярлык {}: {e}", lnk.display()))?;
        }
    }
    Ok(())
}

// ============================================================ инспектор модов (addons/*.vpk)

#[derive(Clone, Debug)]
pub struct AddonModEntry {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub enabled: bool,
}

pub fn addons_dir(game_path: &Path) -> PathBuf {
    game_path.join("game").join("citadel").join("addons")
}

pub fn list_addon_mods(game_path: &Path) -> Vec<AddonModEntry> {
    let dir = addons_dir(game_path);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut list = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(fname) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = fname.to_ascii_lowercase();
        let (enabled, base_name) = if let Some(prefix_len) = lower
            .strip_suffix(".vpk.disabled")
            .map(|s| s.len())
        {
            let Some(stem) = fname.get(..prefix_len) else {
                continue;
            };
            (false, format!("{stem}.vpk"))
        } else if lower.ends_with(".vpk") {
            (true, fname.to_owned())
        } else {
            continue;
        };

        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        list.push(AddonModEntry {
            name: base_name,
            path: p,
            size_bytes,
            enabled,
        });
    }

    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

/// Переключает состояние VPK-мода (`.vpk` <-> `.vpk.disabled`). Возвращает `true`, если мод включён.
pub fn toggle_addon_mod(path: &Path) -> Result<bool, String> {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "Некорректное имя файла мода".to_owned())?;
    let lower = fname.to_ascii_lowercase();

    let (new_name, target_enabled) = if lower.ends_with(".vpk.disabled") {
        let trimmed_len = fname.len().saturating_sub(".disabled".len());
        let base = fname
            .get(..trimmed_len)
            .ok_or_else(|| "Некорректная UTF-8 граница имени мода".to_owned())?;
        (base.to_owned(), true)
    } else if lower.ends_with(".vpk") {
        (format!("{fname}.disabled"), false)
    } else {
        return Err("Ожидался файл с расширением .vpk или .vpk.disabled".into());
    };

    let dst = path.with_file_name(new_name);
    fs::rename(path, &dst).map_err(|e| format!("Не удалось переключить мод: {e}"))?;
    Ok(target_enabled)
}

/// Параллельно считает потоковый SHA-256 (чанки по 64 КБ через `std::thread::scope`) для всех модов `addons/`.
pub fn hash_addon_mods_parallel(mods: &[AddonModEntry]) -> Vec<(PathBuf, Result<String, String>)> {
    if mods.is_empty() {
        return Vec::new();
    }
    let workers = mods
        .len()
        .min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .max(1);
    let next_idx = AtomicUsize::new(0);

    let per_worker = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            handles.push(s.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    let Some(entry) = mods.get(idx) else {
                        break;
                    };
                    let res = crate::gameinfo_sync::sha256_file_streaming(&entry.path)
                        .map_err(|e| format!("Ошибка SHA-256 ({}): {e}", entry.name));
                    local.push((idx, entry.path.clone(), res));
                }
                local
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });

    let mut ordered: Vec<(usize, PathBuf, Result<String, String>)> = per_worker;
    ordered.sort_by_key(|(idx, _, _)| *idx);
    ordered
        .into_iter()
        .map(|(_, path, res)| (path, res))
        .collect()
}

pub fn read_dmm_conflicts_count(game_path: &Path) -> usize {
    let path = addons_dir(game_path).join(".dmm-conflicts.json");
    let Ok(text) = fs::read_to_string(path) else {
        return 0;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    match val {
        serde_json::Value::Array(arr) => arr.len(),
        serde_json::Value::Object(obj) => obj.len(),
        _ => 0,
    }
}

// ============================================================ установка модов (.vpk) через Drag & Drop

pub fn install_dropped_files(paths: &[PathBuf], game_path: &Path) -> Result<String, String> {
    if !steam::is_valid_game_dir(game_path) {
        return Err("Папка Deadlock не найдена — сначала укажите её в Настройках".into());
    }
    let mut vpk_installed = 0usize;
    let mut last_name = String::new();

    for src in paths {
        if !src.is_file() {
            continue;
        }
        let is_vpk = src
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("vpk"));
        if !is_vpk {
            continue;
        }
        let raw_stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("custom");
        let clean_stem = crate::fs_safe::sanitize_filename(raw_stem)?;

        let dir = addons_dir(game_path);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Не удалось создать папку addons: {e}"))?;
        let fname = format!("{clean_stem}.vpk");
        let dst = dir.join(&fname);
        crate::fs_safe::clear_readonly(&dst);
        fs::copy(src, &dst)
            .map_err(|e| format!("Не удалось скопировать мод {fname}: {e}"))?;
        vpk_installed += 1;
        last_name = fname;
    }

    if vpk_installed == 0 {
        return Err("Поддерживаются только файлы модов (.vpk)".into());
    }

    if vpk_installed == 1 {
        Ok(format!("Установлен мод: {last_name}"))
    } else {
        Ok(format!("Установлено модов (.vpk): {vpk_installed}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[cfg(windows)]
    fn test_windows_system_tools_resolve() {
        assert!(windir_exe("explorer.exe").is_file());
    }

    #[test]
    #[cfg(windows)]
    fn test_native_com_shortcut_creates_valid_lnk() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_lnk_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let lnk = root.join("test_shortcut.lnk");
        let exe = std::env::current_exe().unwrap();
        let wd = exe.parent().unwrap();

        win_shortcut::create_shortcut_at(&lnk, &exe, "--play", wd, "Test Shortcut").unwrap();
        let bytes = fs::read(&lnk).unwrap();
        assert!(bytes.len() >= 76, "Файл .lnk слишком короткий");
        assert_eq!(&bytes[0..4], &[0x4C, 0x00, 0x00, 0x00], "Неверный заголовок MS-SHLLINK");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(windows)]
    fn test_win32_process_snapshot_queries_cleanly() {
        #[cfg(target_arch = "x86_64")]
        assert_eq!(size_of::<win_procs::ProcessEntry32W>(), 568);

        let steam_w: Vec<u16> = "Steam.EXE\0".encode_utf16().collect();
        let game_w: Vec<u16> = "DEADLOCK.EXE\0".encode_utf16().collect();
        let fallback_w: Vec<u16> = "PROJECT8.EXE\0".encode_utf16().collect();
        let helper_w: Vec<u16> = "steamwebhelper.exe\0".encode_utf16().collect();

        assert!(win_procs::eq_utf16_ascii(&steam_w, b"steam.exe"));
        assert!(win_procs::eq_utf16_ascii(&game_w, b"deadlock.exe"));
        assert!(win_procs::eq_utf16_ascii(&fallback_w, b"project8.exe"));
        assert!(!win_procs::eq_utf16_ascii(&helper_w, b"steam.exe"));
        assert!(!win_procs::eq_utf16_ascii(&steam_w, b"steamwebhelper.exe"));

        // O(1) проверка собственного процесса
        assert!(win_procs::is_pid_alive(std::process::id()));
        assert!(!win_procs::is_pid_alive(0));

        // Проверяем, что нативный опрос процессов Windows выполняется без паники
        let _ = query_processes();
    }

    #[test]
    fn test_clear_shader_cache_blocks_when_game_running_and_cleans_dirs() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_shadercache_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sc_dir = root.join("steamapps").join("shadercache").join("1422450");
        let vulkan_dir = sc_dir.join("FOSSILIZE_VULKAN");
        let dx_dir = sc_dir.join("DX11_CACHE");
        fs::create_dir_all(&vulkan_dir).unwrap();
        fs::create_dir_all(&dx_dir).unwrap();
        let vk_file = vulkan_dir.join("pipeline.foz");
        let dx_file = dx_dir.join("shader.bin");
        fs::write(&vk_file, b"vulkan_cache_data").unwrap();
        fs::write(&dx_file, b"dx11_cache_data").unwrap();

        // Пока игра запущена, очистка отклоняется, файлы не удаляются
        let blocked = clear_shader_cache_in_dirs(std::slice::from_ref(&sc_dir), true);
        assert!(blocked.is_err());
        assert!(vk_file.is_file());
        assert!(dx_file.is_file());

        // Когда игра закрыта, очистка удаляет всё содержимое подпапок
        let msg = clear_shader_cache_in_dirs(std::slice::from_ref(&sc_dir), false).unwrap();
        assert!(msg.contains("Vulkan/DX11"));
        assert!(!vk_file.exists());
        assert!(!dx_file.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_addons_list_toggle_and_conflicts() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_addons_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let addons = addons_dir(&root);
        fs::create_dir_all(&addons).unwrap();

        let vpk1 = addons.join("pak01_dir.vpk");
        let vpk2 = addons.join("pak02_dir.vpk.disabled");
        fs::write(&vpk1, b"123").unwrap();
        fs::write(&vpk2, b"4567").unwrap();
        fs::write(addons.join(".dmm-conflicts.json"), "[{\"a\":1},{\"b\":2}]").unwrap();

        let mods = list_addon_mods(&root);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].name, "pak01_dir.vpk");
        assert!(mods[0].enabled);
        assert_eq!(mods[1].name, "pak02_dir.vpk");
        assert!(!mods[1].enabled);
        assert_eq!(read_dmm_conflicts_count(&root), 2);

        // Выключаем pak01_dir.vpk
        assert!(!toggle_addon_mod(&vpk1).unwrap());
        assert!(addons.join("pak01_dir.vpk.disabled").is_file());
        // Включаем pak02_dir.vpk.disabled
        assert!(toggle_addon_mod(&vpk2).unwrap());
        assert!(addons.join("pak02_dir.vpk").is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_hash_addon_mods_parallel_computes_all_vpk_hashes() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_addons_hash_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let addons = addons_dir(&root);
        fs::create_dir_all(&addons).unwrap();

        let payloads: [(&str, Vec<u8>); 4] = [
            ("pak01_dir.vpk", b"alpha mod".to_vec()),
            ("pak02_dir.vpk", vec![0x5Au8; 70 * 1024]),
            ("pak03_dir.vpk.disabled", b"disabled mod bytes".to_vec()),
            ("pak04_dir.vpk", vec![0x11u8; 130 * 1024 + 7]),
        ];
        for (name, data) in &payloads {
            fs::write(addons.join(name), data).unwrap();
        }

        let mods = list_addon_mods(&root);
        assert_eq!(mods.len(), 4);

        let hashes = hash_addon_mods_parallel(&mods);
        assert_eq!(hashes.len(), 4);
        for (entry, (path, res)) in mods.iter().zip(hashes.iter()) {
            assert_eq!(&entry.path, path);
            let raw = fs::read(&entry.path).unwrap();
            let expected = crate::gameinfo_sync::sha256_hex(&raw);
            assert_eq!(res.as_ref().unwrap(), &expected);
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_install_dropped_vpk_roundtrip() {
        let root = std::env::temp_dir().join(format!(
            "shaurmeaw_drop_install_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("Deadlock");
        let citadel = game.join("game").join("citadel");
        let staging = root.join("incoming");
        fs::create_dir_all(&citadel).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(citadel.join("gameinfo.gi"), "GameInfo {}").unwrap();

        let src_vpk = staging.join("skin_pack.vpk");
        fs::write(&src_vpk, b"CUSTOM_VPK_BYTES").unwrap();

        let msg = install_dropped_files(&[src_vpk], &game).unwrap();
        assert!(msg.contains("skin_pack.vpk"));

        assert_eq!(
            fs::read(addons_dir(&game).join("skin_pack.vpk")).unwrap(),
            b"CUSTOM_VPK_BYTES"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
