//! «Zero-Footprint Background Engine» и «Safe Game Booster» для Deadlock.
//!
//! Объединяет нативное управление рабочим набором памяти (`K32EmptyWorkingSet` /
//! `SetProcessWorkingSetSize`), телеметрию ОЗУ, безопасное переключение приоритетов
//! процессов (`deadlock.exe` / лаунчер), автообрезку разросшегося `console.log`
//! и событийное ожидание изменений ФС Windows (`FindFirstChangeNotificationW` + `WaitForMultipleObjects`).

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Порог автоматической и ручной обрезки `console.log` (50 МБ).
pub const CONSOLE_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Объём свежих последних строк `console.log`, сохраняемых при обрезке (256 КБ).
const CONSOLE_LOG_KEEP_TAIL_BYTES: u64 = 256 * 1024;

/// Маска уведомлений Win32 для отслеживания создания/удаления/изменения/записи файлов в папке.
pub const NOTIFY_FILE_AND_WRITE: u32 = 0x0000_0001 | 0x0000_0008 | 0x0000_0010;
/// Бесконечное ожидание на объектах ядра Windows (`INFINITE`).
pub const WAIT_INFINITE: u32 = 0xFFFF_FFFF;

static LAUNCHER_THROTTLED: AtomicBool = AtomicBool::new(false);
static TIMER_1MS_ACTIVE: AtomicBool = AtomicBool::new(false);
static POWER_PLAN_BOOSTED: AtomicBool = AtomicBool::new(false);

/// Состояние безопасных пользовательских (HKCU) оптимизаций Windows для Deadlock.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WindowsTweaksStatus {
    pub game_mode_enabled: bool,
    pub game_dvr_disabled: bool,
    pub gpu_high_perf: bool,
    pub fso_disabled: bool,
}

/// Возвращает число логических потоков процессора в системе.
pub fn logical_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

/// Вычисляет маску аффинности ЦП без нулевого ядра (`CPU 0`) при наличии ≥ 6 потоков.
pub fn compute_affinity_mask_without_cpu0(sys_mask: usize) -> Option<usize> {
    if sys_mask.count_ones() >= 6 {
        let trimmed = sys_mask & !1usize;
        if trimmed != 0 {
            return Some(trimmed);
        }
    }
    None
}

/// Показывает, активен ли сейчас 1 мс системный таймер Windows.
pub fn is_high_res_timer_active() -> bool {
    TIMER_1MS_ACTIVE.load(Ordering::Acquire)
}

/// Показывает, переключена ли схема питания на «Высокую производительность» лаунчером.
pub fn is_power_plan_boosted() -> bool {
    POWER_PLAN_BOOSTED.load(Ordering::Acquire)
}

/// Возвращает полный путь к `Deadlock\game\citadel\console.log`.
pub fn console_log_path(game_path: &Path) -> PathBuf {
    game_path.join("game").join("citadel").join("console.log")
}

/// Возвращает текущий размер `console.log` в байтах (или `0`, если файл не существует).
pub fn console_log_size(game_path: &Path) -> u64 {
    fs::metadata(console_log_path(game_path))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Форматирует объём ОЗУ в мегабайтах (например, `6.4 МБ`).
pub fn format_ram_mb(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    format!("{mb:.1} МБ")
}

/// Сообщает, понижен ли сейчас приоритет лаунчера на время активного матча Deadlock.
pub fn is_launcher_throttled() -> bool {
    LAUNCHER_THROTTLED.load(Ordering::Relaxed)
}

/// Безопасно обрезает `console.log`, если его размер превышает `max_bytes` и игра не запущена.
/// Возвращает `Ok(Some(freed_bytes))` при выполнении обрезки либо `Ok(None)`, если файл в норме.
pub fn clean_console_log_if_oversized(
    log_path: &Path,
    max_bytes: u64,
    game_running: bool,
) -> Result<Option<u64>, String> {
    if game_running {
        return Ok(None);
    }
    let Ok(meta) = fs::metadata(log_path) else {
        return Ok(None);
    };
    let total_len = meta.len();
    if total_len <= max_bytes {
        return Ok(None);
    }

    let keep_len = total_len.min(CONSOLE_LOG_KEEP_TAIL_BYTES);
    let start_pos = total_len.saturating_sub(keep_len);

    let mut tail = Vec::with_capacity(keep_len as usize);
    {
        let mut f = File::open(log_path)
            .map_err(|e| format!("Не удалось открыть {}: {e}", log_path.display()))?;
        f.seek(SeekFrom::Start(start_pos))
            .map_err(|e| format!("Ошибка позиционирования в {}: {e}", log_path.display()))?;
        f.read_to_end(&mut tail)
            .map_err(|e| format!("Ошибка чтения {}: {e}", log_path.display()))?;
    }

    let aligned_slice = if start_pos > 0 {
        if let Some(pos) = tail.iter().position(|&b| b == b'\n') {
            &tail[pos + 1..]
        } else {
            &tail[..]
        }
    } else {
        &tail[..]
    };

    crate::fs_safe::atomic_write_with_retry(log_path, aligned_slice)
        .map_err(|e| format!("Не удалось обновить {}: {e}", log_path.display()))?;

    Ok(Some(total_len.saturating_sub(aligned_slice.len() as u64)))
}

#[cfg(windows)]
mod win_impl {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};

    use super::{
        compute_affinity_mask_without_cpu0, WindowsTweaksStatus, LAUNCHER_THROTTLED,
        NOTIFY_FILE_AND_WRITE, POWER_PLAN_BOOSTED, TIMER_1MS_ACTIVE,
    };

    const PROCESS_SET_INFORMATION: u32 = 0x0200;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
    const INVALID_HANDLE_VALUE: *mut c_void = (-1isize) as *mut c_void;
    const WAIT_OBJECT_0: u32 = 0x0000_0000;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const MAXIMUM_WAIT_OBJECTS: usize = 4;
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct WinGuid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    /// Стандартный GUID схемы питания Windows «Высокая производительность» (`8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c`).
    const GUID_HIGH_PERFORMANCE: WinGuid = WinGuid {
        data1: 0x8c5e_7fda,
        data2: 0xe8bf,
        data3: 0x4a96,
        data4: [0x9a, 0x85, 0xa6, 0xe2, 0x3a, 0x8c, 0x63, 0x5c],
    };

    static SAVED_POWER_PLAN: Mutex<Option<WinGuid>> = Mutex::new(None);

    #[repr(C)]
    pub(super) struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        pub(super) working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> *mut c_void;
        fn SetPriorityClass(h_process: *mut c_void, dw_priority_class: u32) -> i32;
        fn GetProcessAffinityMask(
            h_process: *mut c_void,
            lp_process_affinity_mask: *mut usize,
            lp_system_affinity_mask: *mut usize,
        ) -> i32;
        fn SetProcessAffinityMask(h_process: *mut c_void, dw_process_affinity_mask: usize) -> i32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
        fn LoadLibraryExW(
            lp_lib_file_name: *const u16,
            h_file: *mut c_void,
            dw_flags: u32,
        ) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;

        fn K32EmptyWorkingSet(h_process: *mut c_void) -> i32;
        fn K32GetProcessMemoryInfo(
            h_process: *mut c_void,
            ppsmem_counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
        fn SetProcessWorkingSetSize(
            h_process: *mut c_void,
            dw_minimum_working_set_size: usize,
            dw_maximum_working_set_size: usize,
        ) -> i32;

        fn CreateEventW(
            lp_event_attributes: *mut c_void,
            b_manual_reset: i32,
            b_initial_state: i32,
            lp_name: *const u16,
        ) -> *mut c_void;
        fn SetEvent(h_event: *mut c_void) -> i32;
        fn WaitForMultipleObjects(
            n_count: u32,
            lp_handles: *const *mut c_void,
            b_wait_all: i32,
            dw_milliseconds: u32,
        ) -> u32;

        fn FindFirstChangeNotificationW(
            lp_path_name: *const u16,
            b_watch_subtree: i32,
            dw_notify_filter: u32,
        ) -> *mut c_void;
        fn FindNextChangeNotification(h_change_handle: *mut c_void) -> i32;
        fn FindCloseChangeNotification(h_change_handle: *mut c_void) -> i32;
    }

    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(u_period: u32) -> u32;
        fn timeEndPeriod(u_period: u32) -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            lp_enum_func: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            l_param: isize,
        ) -> i32;
        fn GetWindow(h_wnd: *mut c_void, u_cmd: u32) -> *mut c_void;
        fn IsWindowVisible(h_wnd: *mut c_void) -> i32;
        fn GetWindowThreadProcessId(h_wnd: *mut c_void, lpdw_process_id: *mut u32) -> u32;
        fn LoadKeyboardLayoutW(pwsz_klid: *const u16, flags: u32) -> *mut c_void;
        fn PostMessageW(h_wnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> i32;
    }

    type PowerGetActiveSchemeFn = unsafe extern "system" fn(*mut c_void, *mut *mut WinGuid) -> u32;
    type PowerSetActiveSchemeFn = unsafe extern "system" fn(*mut c_void, *const WinGuid) -> u32;

    struct PowrProfApi {
        get_active_scheme: PowerGetActiveSchemeFn,
        set_active_scheme: PowerSetActiveSchemeFn,
    }

    fn powrprof_api() -> Option<&'static PowrProfApi> {
        static API: OnceLock<Option<PowrProfApi>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let wide: Vec<u16> = "powrprof.dll\0".encode_utf16().collect();
            let mut module = LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if module.is_null() {
                module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), 0);
            }
            if module.is_null() {
                return None;
            }
            let get_raw = GetProcAddress(module, b"PowerGetActiveScheme\0".as_ptr());
            let set_raw = GetProcAddress(module, b"PowerSetActiveScheme\0".as_ptr());
            if get_raw.is_null() || set_raw.is_null() {
                return None;
            }
            Some(PowrProfApi {
                get_active_scheme: std::mem::transmute::<*const c_void, PowerGetActiveSchemeFn>(
                    get_raw,
                ),
                set_active_scheme: std::mem::transmute::<*const c_void, PowerSetActiveSchemeFn>(
                    set_raw,
                ),
            })
        })
        .as_ref()
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

    pub fn trim_memory() -> bool {
        unsafe {
            let h = GetCurrentProcess();
            let a = K32EmptyWorkingSet(h) != 0;
            let b = SetProcessWorkingSetSize(h, usize::MAX, usize::MAX) != 0;
            a || b
        }
    }

    pub fn current_working_set_bytes() -> u64 {
        unsafe {
            let mut pmc: ProcessMemoryCounters = zeroed();
            pmc.cb = size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
                pmc.working_set_size as u64
            } else {
                0
            }
        }
    }

    pub fn set_process_high_priority(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let Some(h) = WinHandle::new(OpenProcess(PROCESS_SET_INFORMATION, 0, pid)) else {
                return false;
            };
            SetPriorityClass(h.0, HIGH_PRIORITY_CLASS) != 0
        }
    }

    pub fn set_launcher_low_priority() -> bool {
        let ok = unsafe { SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) != 0 };
        if ok {
            LAUNCHER_THROTTLED.store(true, Ordering::Relaxed);
        }
        ok
    }

    pub fn restore_launcher_priority() -> bool {
        let ok = unsafe { SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS) != 0 };
        if ok {
            LAUNCHER_THROTTLED.store(false, Ordering::Relaxed);
        }
        ok
    }

    pub fn enable_high_res_timer() -> bool {
        if TIMER_1MS_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return true;
        }
        if unsafe { timeBeginPeriod(1) } == 0 {
            true
        } else {
            TIMER_1MS_ACTIVE.store(false, Ordering::Release);
            false
        }
    }

    pub fn disable_high_res_timer() {
        if TIMER_1MS_ACTIVE
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            unsafe {
                let _ = timeEndPeriod(1);
            }
        }
    }

    pub fn enable_high_perf_power_plan() -> bool {
        if POWER_PLAN_BOOSTED.load(Ordering::Acquire) {
            return true;
        }
        let Some(api) = powrprof_api() else {
            return false;
        };
        unsafe {
            let mut cur_ptr: *mut WinGuid = std::ptr::null_mut();
            if (api.get_active_scheme)(std::ptr::null_mut(), &mut cur_ptr) != 0 || cur_ptr.is_null()
            {
                return false;
            }
            let cur_guid = *cur_ptr;
            LocalFree(cur_ptr.cast::<c_void>());
            if cur_guid == GUID_HIGH_PERFORMANCE {
                return true;
            }
            if (api.set_active_scheme)(std::ptr::null_mut(), &GUID_HIGH_PERFORMANCE) == 0 {
                if let Ok(mut saved) = SAVED_POWER_PLAN.lock() {
                    *saved = Some(cur_guid);
                }
                POWER_PLAN_BOOSTED.store(true, Ordering::Release);
                true
            } else {
                false
            }
        }
    }

    pub fn restore_power_plan() -> bool {
        if !POWER_PLAN_BOOSTED.swap(false, Ordering::AcqRel) {
            return false;
        }
        let prev = SAVED_POWER_PLAN.lock().ok().and_then(|mut g| g.take());
        if let Some(guid) = prev {
            let Some(api) = powrprof_api() else {
                return false;
            };
            unsafe { (api.set_active_scheme)(std::ptr::null_mut(), &guid) == 0 }
        } else {
            false
        }
    }

    pub fn optimize_process_affinity_no_cpu0(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let Some(h) =
                WinHandle::new(OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION, 0, pid))
            else {
                return false;
            };
            let mut proc_mask: usize = 0;
            let mut sys_mask: usize = 0;
            if GetProcessAffinityMask(h.0, &mut proc_mask, &mut sys_mask) != 0 {
                if let Some(target) = compute_affinity_mask_without_cpu0(sys_mask) {
                    if proc_mask == target {
                        true
                    } else {
                        SetProcessAffinityMask(h.0, target) != 0
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
    }

    pub fn restore_process_affinity(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let Some(h) =
                WinHandle::new(OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION, 0, pid))
            else {
                return false;
            };
            let mut proc_mask: usize = 0;
            let mut sys_mask: usize = 0;
            if GetProcessAffinityMask(h.0, &mut proc_mask, &mut sys_mask) != 0 && sys_mask != 0 {
                if proc_mask == sys_mask {
                    true
                } else {
                    SetProcessAffinityMask(h.0, sys_mask) != 0
                }
            } else {
                false
            }
        }
    }

    fn game_exe_key_string(game_path: &Path) -> Option<String> {
        let exe = crate::steam::game_exe(game_path);
        exe.exists().then(|| exe.to_string_lossy().into_owned())
    }

    pub fn inspect_windows_tweaks(game_path: &Path) -> WindowsTweaksStatus {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let game_mode_enabled = hkcu
            .open_subkey(r"Software\Microsoft\GameBar")
            .ok()
            .and_then(|k| k.get_value::<u32, _>("AutoGameModeEnabled").ok())
            .is_some_and(|v| v == 1);

        let dvr_store_off = hkcu
            .open_subkey(r"System\GameConfigStore")
            .ok()
            .and_then(|k| k.get_value::<u32, _>("GameDVR_Enabled").ok())
            .is_some_and(|v| v == 0);
        let dvr_cap_off = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\GameDVR")
            .ok()
            .and_then(|k| k.get_value::<u32, _>("AppCaptureEnabled").ok())
            .is_some_and(|v| v == 0);

        let exe_key = game_exe_key_string(game_path);
        let gpu_high_perf = exe_key.as_deref().is_some_and(|key_name| {
            hkcu.open_subkey(r"Software\Microsoft\DirectX\UserGpuPreferences")
                .ok()
                .and_then(|k| k.get_value::<String, _>(key_name).ok())
                .is_some_and(|val| val.contains("GpuPreference=2"))
        });

        let fso_disabled = exe_key.as_deref().is_some_and(|key_name| {
            hkcu.open_subkey(r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers")
                .ok()
                .and_then(|k| k.get_value::<String, _>(key_name).ok())
                .is_some_and(|val| val.contains("DISABLEDXMAXIMIZEDWINDOWEDMODE"))
        });

        WindowsTweaksStatus {
            game_mode_enabled,
            game_dvr_disabled: dvr_store_off && dvr_cap_off,
            gpu_high_perf,
            fso_disabled,
        }
    }

    pub fn set_windows_game_mode(enable: bool) -> Result<(), String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(r"Software\Microsoft\GameBar")
            .map_err(|e| format!("Не удалось открыть GameBar в реестре: {e}"))?;
        let val: u32 = u32::from(enable);
        key.set_value("AutoGameModeEnabled", &val)
            .map_err(|e| format!("Не удалось обновить AutoGameModeEnabled: {e}"))?;
        Ok(())
    }

    pub fn set_disable_game_dvr(disable: bool) -> Result<(), String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let val: u32 = if disable { 0 } else { 1 };
        let (store, _) = hkcu
            .create_subkey(r"System\GameConfigStore")
            .map_err(|e| format!("Не удалось открыть GameConfigStore: {e}"))?;
        store
            .set_value("GameDVR_Enabled", &val)
            .map_err(|e| format!("Не удалось обновить GameDVR_Enabled: {e}"))?;

        let (dvr, _) = hkcu
            .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\GameDVR")
            .map_err(|e| format!("Не удалось открыть GameDVR в реестре: {e}"))?;
        dvr.set_value("AppCaptureEnabled", &val)
            .map_err(|e| format!("Не удалось обновить AppCaptureEnabled: {e}"))?;
        Ok(())
    }

    pub fn set_deadlock_high_perf_gpu(game_path: &Path, enable: bool) -> Result<(), String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let exe_key = game_exe_key_string(game_path)
            .ok_or_else(|| "deadlock.exe не найден — укажите папку игры во вкладке «Пути»".to_owned())?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(r"Software\Microsoft\DirectX\UserGpuPreferences")
            .map_err(|e| format!("Не удалось открыть UserGpuPreferences: {e}"))?;
        if enable {
            key.set_value(&exe_key, &"GpuPreference=2;")
                .map_err(|e| format!("Не удалось задать приоритет GPU для Deadlock: {e}"))?;
        } else {
            let _ = key.delete_value(&exe_key);
        }
        Ok(())
    }

    pub fn set_deadlock_disable_fso(game_path: &Path, disable: bool) -> Result<(), String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let exe_key = game_exe_key_string(game_path)
            .ok_or_else(|| "deadlock.exe не найден — укажите папку игры во вкладке «Пути»".to_owned())?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers")
            .map_err(|e| format!("Не удалось открыть AppCompatFlags\\Layers: {e}"))?;
        if disable {
            key.set_value(&exe_key, &"~ DISABLEDXMAXIMIZEDWINDOWEDMODE")
                .map_err(|e| format!("Не удалось отключить Fullscreen Optimizations: {e}"))?;
        } else if let Ok(existing) = key.get_value::<String, _>(&exe_key) {
            let cleaned: Vec<&str> = existing
                .split_whitespace()
                .filter(|t| *t != "DISABLEDXMAXIMIZEDWINDOWEDMODE" && *t != "~")
                .collect();
            if cleaned.is_empty() {
                let _ = key.delete_value(&exe_key);
            } else {
                let next = format!("~ {}", cleaned.join(" "));
                let _ = key.set_value(&exe_key, &next);
            }
        }
        Ok(())
    }

    pub fn apply_all_windows_tweaks(game_path: &Path) -> Result<String, String> {
        set_windows_game_mode(true)?;
        set_disable_game_dvr(true)?;
        if game_exe_key_string(game_path).is_some() {
            set_deadlock_high_perf_gpu(game_path, true)?;
            set_deadlock_disable_fso(game_path, true)?;
            Ok("Все 4 системные оптимизации Windows применены для Deadlock".to_owned())
        } else {
            Ok("Game Mode и отключение Game DVR применены (укажите папку игры для GPU и FSO)".to_owned())
        }
    }

    pub fn revert_all_windows_tweaks(game_path: &Path) -> Result<String, String> {
        set_windows_game_mode(false)?;
        set_disable_game_dvr(false)?;
        if game_exe_key_string(game_path).is_some() {
            set_deadlock_high_perf_gpu(game_path, false)?;
            set_deadlock_disable_fso(game_path, false)?;
        }
        Ok("Системные оптимизации Windows возвращены в стандартное состояние".to_owned())
    }

    /// Результат ожидания события пробуждения или изменения отслеживаемых каталогов ФС.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum WaitOutcome {
        Woken,
        DirChanged(usize),
        DirClosed(usize),
        Timeout,
        Error,
    }

    /// Нативное событие пробуждения фонового потока (`CreateEventW`, auto-reset).
    pub struct WinEvent {
        handle: *mut c_void,
    }

    // Безопасно: Win32 Event handle потокобезопасен для SetEvent / WaitForMultipleObjects / CloseHandle.
    unsafe impl Send for WinEvent {}
    unsafe impl Sync for WinEvent {}

    impl WinEvent {
        pub fn new() -> Option<Self> {
            let h = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null()) };
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                None
            } else {
                Some(Self { handle: h })
            }
        }

        pub fn signal(&self) {
            unsafe {
                let _ = SetEvent(self.handle);
            }
        }
    }

    impl Drop for WinEvent {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    /// Нативный наблюдатель изменений каталога Windows (`FindFirstChangeNotificationW`).
    pub struct DirChangeHandle {
        handle: *mut c_void,
    }

    // Безопасно: Win32 notification handle допускает перенос в рабочий поток и закрытие там.
    unsafe impl Send for DirChangeHandle {}

    impl DirChangeHandle {
        pub fn open(dir: &Path, watch_subtree: bool, filter: u32) -> Option<Self> {
            if !dir.is_dir() {
                return None;
            }
            let wide: Vec<u16> = dir
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let h = unsafe {
                FindFirstChangeNotificationW(
                    wide.as_ptr(),
                    i32::from(watch_subtree),
                    if filter == 0 {
                        NOTIFY_FILE_AND_WRITE
                    } else {
                        filter
                    },
                )
            };
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                None
            } else {
                Some(Self { handle: h })
            }
        }

        pub fn arm_next(&self) -> bool {
            unsafe { FindNextChangeNotification(self.handle) != 0 }
        }
    }

    impl Drop for DirChangeHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = FindCloseChangeNotification(self.handle);
            }
        }
    }

    /// Спит в ядре без расхода CPU до сигнала `wake`, изменения любого каталога `dirs` или истечения `timeout_ms`.
    pub fn wait_event_or_dirs(
        wake: &WinEvent,
        dirs: &[&DirChangeHandle],
        timeout_ms: u32,
    ) -> WaitOutcome {
        let mut handles: [*mut c_void; MAXIMUM_WAIT_OBJECTS] = [std::ptr::null_mut(); MAXIMUM_WAIT_OBJECTS];
        handles[0] = wake.handle;
        let dir_count = dirs.len().min(MAXIMUM_WAIT_OBJECTS - 1);
        for (idx, dir) in dirs.iter().take(dir_count).enumerate() {
            handles[idx + 1] = dir.handle;
        }
        let total = 1 + dir_count;

        let res = unsafe {
            WaitForMultipleObjects(total as u32, handles.as_ptr(), 0, timeout_ms)
        };
        if res == WAIT_OBJECT_0 {
            WaitOutcome::Woken
        } else if res > WAIT_OBJECT_0 && res < WAIT_OBJECT_0 + total as u32 {
            let dir_idx = (res - WAIT_OBJECT_0 - 1) as usize;
            if dirs[dir_idx].arm_next() {
                WaitOutcome::DirChanged(dir_idx)
            } else {
                WaitOutcome::DirClosed(dir_idx)
            }
        } else if res == WAIT_TIMEOUT {
            WaitOutcome::Timeout
        } else {
            WaitOutcome::Error
        }
    }

    /// Переключает раскладку главного окна процесса `pid` на английскую (US `00000409`).
    pub fn switch_process_window_to_english_layout(pid: u32) -> bool {
        const GW_OWNER: u32 = 4;
        const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;
        const EN_KLID: [u16; 9] = [0x30, 0x30, 0x30, 0x30, 0x30, 0x34, 0x30, 0x39, 0];

        struct Search {
            pid: u32,
            found: *mut c_void,
        }

        unsafe extern "system" fn enum_cb(hwnd: *mut c_void, l_param: isize) -> i32 {
            let s = &mut *(l_param as *mut Search);
            if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
                return 1;
            }
            let mut win_pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, &raw mut win_pid);
            if win_pid == s.pid {
                s.found = hwnd;
                return 0;
            }
            1
        }

        if pid == 0 {
            return false;
        }
        let mut search = Search {
            pid,
            found: std::ptr::null_mut(),
        };
        unsafe {
            let _ = EnumWindows(enum_cb, (&raw mut search) as isize);
            if search.found.is_null() {
                return false;
            }
            let hkl = LoadKeyboardLayoutW(EN_KLID.as_ptr(), 0);
            if hkl.is_null() {
                return false;
            }
            PostMessageW(search.found, WM_INPUTLANGCHANGEREQUEST, 0, hkl as isize) != 0
        }
    }
}

#[cfg(not(windows))]
mod win_impl {
    use std::path::Path;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum WaitOutcome {
        Woken,
        DirChanged(usize),
        DirClosed(usize),
        Timeout,
        Error,
    }

    pub struct WinEvent {
        state: (Mutex<bool>, Condvar),
    }

    impl WinEvent {
        pub fn new() -> Option<Self> {
            Some(Self {
                state: (Mutex::new(false), Condvar::new()),
            })
        }

        pub fn signal(&self) {
            let (ref lock, ref cvar) = self.state;
            if let Ok(mut g) = lock.lock() {
                *g = true;
                cvar.notify_all();
            }
        }
    }

    pub struct DirChangeHandle;

    impl DirChangeHandle {
        pub fn open(_dir: &Path, _watch_subtree: bool, _filter: u32) -> Option<Self> {
            None
        }
    }

    pub fn trim_memory() -> bool {
        false
    }

    pub fn current_working_set_bytes() -> u64 {
        0
    }

    pub fn set_process_high_priority(_pid: u32) -> bool {
        false
    }

    pub fn set_launcher_low_priority() -> bool {
        false
    }

    pub fn restore_launcher_priority() -> bool {
        false
    }

    pub fn enable_high_res_timer() -> bool {
        false
    }

    pub fn disable_high_res_timer() {}

    pub fn enable_high_perf_power_plan() -> bool {
        false
    }

    pub fn restore_power_plan() -> bool {
        false
    }

    pub fn optimize_process_affinity_no_cpu0(_pid: u32) -> bool {
        false
    }

    pub fn restore_process_affinity(_pid: u32) -> bool {
        false
    }

    pub fn inspect_windows_tweaks(_game_path: &Path) -> super::WindowsTweaksStatus {
        super::WindowsTweaksStatus::default()
    }

    pub fn set_windows_game_mode(_enable: bool) -> Result<(), String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn set_disable_game_dvr(_disable: bool) -> Result<(), String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn set_deadlock_high_perf_gpu(_game_path: &Path, _enable: bool) -> Result<(), String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn set_deadlock_disable_fso(_game_path: &Path, _disable: bool) -> Result<(), String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn apply_all_windows_tweaks(_game_path: &Path) -> Result<String, String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn revert_all_windows_tweaks(_game_path: &Path) -> Result<String, String> {
        Err("Поддерживается только в Windows".into())
    }

    pub fn switch_process_window_to_english_layout(_pid: u32) -> bool {
        false
    }

    pub fn wait_event_or_dirs(
        wake: &WinEvent,
        _dirs: &[&DirChangeHandle],
        timeout_ms: u32,
    ) -> WaitOutcome {
        let (ref lock, ref cvar) = wake.state;
        let Ok(mut g) = lock.lock() else {
            return WaitOutcome::Error;
        };
        if *g {
            *g = false;
            return WaitOutcome::Woken;
        }
        if timeout_ms == super::WAIT_INFINITE {
            while !*g {
                g = cvar.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            *g = false;
            return WaitOutcome::Woken;
        }
        let (next, res) = cvar
            .wait_timeout(g, Duration::from_millis(timeout_ms as u64))
            .unwrap_or_else(|e| e.into_inner());
        g = next;
        if *g {
            *g = false;
            WaitOutcome::Woken
        } else if res.timed_out() {
            WaitOutcome::Timeout
        } else {
            WaitOutcome::Error
        }
    }
}

pub use win_impl::{
    apply_all_windows_tweaks, current_working_set_bytes, disable_high_res_timer,
    enable_high_perf_power_plan, enable_high_res_timer, inspect_windows_tweaks,
    optimize_process_affinity_no_cpu0, restore_launcher_priority, restore_power_plan,
    restore_process_affinity, revert_all_windows_tweaks, set_deadlock_disable_fso,
    set_deadlock_high_perf_gpu, set_disable_game_dvr, set_launcher_low_priority,
    set_process_high_priority, set_windows_game_mode, switch_process_window_to_english_layout,
    trim_memory, wait_event_or_dirs, DirChangeHandle, WaitOutcome, WinEvent,
};

/// Безопасно откатывает все временные системные бусты времени матча.
pub fn restore_all_runtime_boosts() {
    restore_launcher_priority();
    disable_high_res_timer();
    restore_power_plan();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shaurmeaw_gb_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(windows)]
    #[test]
    fn test_working_set_and_trim_memory_smoke() {
        assert_eq!(
            size_of::<win_impl::ProcessMemoryCounters>(),
            72,
            "Размер PROCESS_MEMORY_COUNTERS на Win64 должен быть 72 байта"
        );
        let ws = current_working_set_bytes();
        assert!(ws > 0, "WorkingSetSize должен быть > 0");
        assert!(trim_memory(), "Сброс рабочего набора памяти должен выполниться без ошибки");
        assert!(format_ram_mb(6_710_886).contains("6.4 МБ"));
    }

    #[cfg(windows)]
    #[test]
    fn test_launcher_priority_roundtrip() {
        assert!(set_launcher_low_priority());
        assert!(is_launcher_throttled());
        assert!(restore_launcher_priority());
        assert!(!is_launcher_throttled());
    }

    #[test]
    fn test_clean_console_log_oversized_and_game_running_guard() {
        let dir = temp_dir("clog");
        let log = dir.join("console.log");

        // Генерируем > 300 КБ текста, порог задаём 280 КБ (чтобы оставить 256 КБ хвоста по границе строки)
        let mut content = String::with_capacity(320 * 1024);
        for i in 0..8000 {
            content.push_str(&format!("11/22 03:00:00 line-{i:05}-deadlock-console-log\n"));
        }
        let orig_len = content.len() as u64;
        assert!(orig_len > CONSOLE_LOG_KEEP_TAIL_BYTES);
        fs::write(&log, &content).unwrap();

        // Пока игра запущена — не трогаем файл
        assert_eq!(
            clean_console_log_if_oversized(&log, 1024, true).unwrap(),
            None
        );
        assert_eq!(fs::metadata(&log).unwrap().len(), orig_len);

        // Если размер <= порога — тоже не трогаем
        assert_eq!(
            clean_console_log_if_oversized(&log, orig_len + 1, false).unwrap(),
            None
        );

        // При превышении порога и закрытой игре — обрезаем до <= 256 КБ по границе строки
        let threshold = CONSOLE_LOG_KEEP_TAIL_BYTES + 1024;
        let freed = clean_console_log_if_oversized(&log, threshold, false)
            .unwrap()
            .expect("должна выполниться обрезка console.log");
        assert!(freed > 0);
        let after = fs::read_to_string(&log).unwrap();
        assert!((after.len() as u64) <= CONSOLE_LOG_KEEP_TAIL_BYTES);
        assert!(after.starts_with("11/22 "));
        assert!(after.contains("line-07999-deadlock-console-log"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn test_win_event_and_dir_watcher_detects_write() {
        let dir = temp_dir("watch");
        let wake = WinEvent::new().expect("WinEvent::new");
        let dir_h =
            DirChangeHandle::open(&dir, false, NOTIFY_FILE_AND_WRITE).expect("DirChangeHandle");

        // 1. Сигнал события будит поток мгновенно
        wake.signal();
        assert_eq!(wait_event_or_dirs(&wake, &[&dir_h], 1000), WaitOutcome::Woken);

        // 2. Запись файла в папку срабатывает через DirChanged(0)
        let f = dir.join("test.log");
        fs::write(&f, "hello").unwrap();
        let outcome = wait_event_or_dirs(&wake, &[&dir_h], 1000);
        assert_eq!(outcome, WaitOutcome::DirChanged(0));

        drop(dir_h);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn test_high_res_timer_roundtrip() {
        assert!(enable_high_res_timer());
        assert!(is_high_res_timer_active());
        disable_high_res_timer();
        assert!(!is_high_res_timer_active());
    }

    #[test]
    fn test_compute_affinity_mask_without_cpu0() {
        assert_eq!(compute_affinity_mask_without_cpu0(0b0011), None);
        assert_eq!(compute_affinity_mask_without_cpu0(0b1111), None);
        assert_eq!(compute_affinity_mask_without_cpu0(0b11_1111), Some(0b11_1110));
        assert_eq!(compute_affinity_mask_without_cpu0(0b1111_1111), Some(0b1111_1110));
        assert_eq!(compute_affinity_mask_without_cpu0(0xFFFF), Some(0xFFFE));
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_tweaks_inspect_smoke() {
        let _status = inspect_windows_tweaks(Path::new("."));
    }
}
