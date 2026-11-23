//! Ограничение на запуск единственного экземпляра лаунчера через именованный Mutex Windows.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const GW_OWNER: u32 = 4;
    const SW_SHOW: i32 = 5;
    const SW_RESTORE: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            lp_mutex_attributes: *mut c_void,
            b_initial_owner: i32,
            lp_name: *const u16,
        ) -> *mut c_void;
        fn GetLastError() -> u32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(lp_class_name: *const u16, lp_window_name: *const u16) -> *mut c_void;
        fn EnumWindows(
            lp_enum_func: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            l_param: isize,
        ) -> i32;
        fn GetWindow(h_wnd: *mut c_void, u_cmd: u32) -> *mut c_void;
        fn IsIconic(h_wnd: *mut c_void) -> i32;
        fn GetWindowTextW(h_wnd: *mut c_void, lp_string: *mut u16, n_max_count: i32) -> i32;
        fn ShowWindow(h_wnd: *mut c_void, n_cmd_show: i32) -> i32;
        fn BringWindowToTop(h_wnd: *mut c_void) -> i32;
        fn SetForegroundWindow(h_wnd: *mut c_void) -> i32;
        fn InvalidateRect(h_wnd: *mut c_void, lp_rect: *const c_void, b_erase: i32) -> i32;
    }

    fn to_wide_z(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// RAII-обёртка над именованным системным мьютексом Windows.
    pub struct SingleInstanceGuard {
        handle: *mut c_void,
    }

    // Безопасно: мьютекс принадлежит процессу и освобождается при закрытии хэндла в Drop.
    unsafe impl Send for SingleInstanceGuard {}

    impl SingleInstanceGuard {
        /// Захватывает именованный мьютекс. Возвращает `None`, если экземпляр уже запущен.
        pub fn acquire(mutex_name: &str) -> Option<Self> {
            let wide = to_wide_z(mutex_name);
            let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 1, wide.as_ptr()) };
            if handle.is_null() {
                // Fail-open: в нестандартной песочнице не блокируем старт приложения.
                return Some(Self {
                    handle: std::ptr::null_mut(),
                });
            }
            let last_err = unsafe { GetLastError() };
            if last_err == ERROR_ALREADY_EXISTS {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return None;
            }
            Some(Self { handle })
        }
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    let _ = CloseHandle(self.handle);
                }
                self.handle = std::ptr::null_mut();
            }
        }
    }

    struct EnumSearch<'a> {
        substr: &'a str,
        found: *mut c_void,
    }

    unsafe extern "system" fn enum_windows_cb(hwnd: *mut c_void, l_param: isize) -> i32 {
        let search = &mut *(l_param as *mut EnumSearch<'_>);
        if !GetWindow(hwnd, GW_OWNER).is_null() {
            return 1;
        }

        let mut buf = [0u16; 256];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
        if copied > 0 {
            if let Some(slice) = buf.get(..copied as usize) {
                let title = String::from_utf16_lossy(slice);
                if title.contains(search.substr) {
                    search.found = hwnd;
                    return 0;
                }
            }
        }
        1
    }

    /// Находит главное окно первого экземпляра по точному заголовку (или подстроке) и выводит его на передний план.
    pub fn focus_existing_window(exact_title: &str, fallback_substr: &str) -> bool {
        let wide_title = to_wide_z(exact_title);
        let mut hwnd = unsafe { FindWindowW(std::ptr::null(), wide_title.as_ptr()) };
        if hwnd.is_null() && !fallback_substr.is_empty() {
            let mut search = EnumSearch {
                substr: fallback_substr,
                found: std::ptr::null_mut(),
            };
            unsafe {
                let _ = EnumWindows(
                    enum_windows_cb,
                    std::ptr::from_mut(&mut search) as isize,
                );
            }
            hwnd = search.found;
        }
        if hwnd.is_null() {
            return false;
        }

        unsafe {
            if IsIconic(hwnd) != 0 {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(hwnd, std::ptr::null(), 0);
        }
        true
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct SingleInstanceGuard;

    impl SingleInstanceGuard {
        pub fn acquire(_mutex_name: &str) -> Option<Self> {
            Some(Self)
        }
    }

    pub fn focus_existing_window(_exact_title: &str, _fallback_substr: &str) -> bool {
        false
    }
}

pub use imp::{focus_existing_window, SingleInstanceGuard};

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_single_instance_mutex_blocks_duplicate_and_releases_on_drop() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1);
        let name = format!("Local\\ShaurMeaw_TestMutex_{}_{}", std::process::id(), nanos);

        let first = SingleInstanceGuard::acquire(&name);
        assert!(first.is_some(), "Первый захват мьютекса должен быть успешным");

        let second = SingleInstanceGuard::acquire(&name);
        assert!(second.is_none(), "Повторный захват того же мьютекса должен отклоняться");

        drop(first);

        let third = SingleInstanceGuard::acquire(&name);
        assert!(third.is_some(), "После Drop первого guard мьютекс должен освобождаться");
    }
}
