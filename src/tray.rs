//! Нативный системный трей Windows без внешних крейтов.
//!
//! Использует `Shell_NotifyIconW` и скрытое служебное окно сообщений в отдельном потоке,
//! позволяя сворачивать лаунчер в область уведомлений, быстро запускать Deadlock из меню ПКМ
//! и возвращать окно по клику.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use eframe::egui;

    const WM_NULL: u32 = 0x0000;
    const WM_DESTROY: u32 = 0x0002;
    const WM_CLOSE: u32 = 0x0010;
    const WM_COMMAND: u32 = 0x0111;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_APP_TRAY: u32 = 0x8001;
    const WM_APP_SHUTDOWN: u32 = 0x8002;

    const NIM_ADD: u32 = 0x0000_0000;
    const NIM_DELETE: u32 = 0x0000_0002;
    const NIM_SETVERSION: u32 = 0x0000_0004;
    const NIF_MESSAGE: u32 = 0x0000_0001;
    const NIF_ICON: u32 = 0x0000_0002;
    const NIF_TIP: u32 = 0x0000_0004;
    const NOTIFYICON_VERSION_4: u32 = 4;

    const IMAGE_ICON: u32 = 1;
    const LR_SHARED: u32 = 0x8000;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;

    const MF_STRING: u32 = 0x0000_0000;
    const MF_SEPARATOR: u32 = 0x0000_0800;
    const TPM_RIGHTBUTTON: u32 = 0x0002;
    const TPM_BOTTOMALIGN: u32 = 0x0020;

    const SW_HIDE: i32 = 0;
    const SW_SHOW: i32 = 5;
    const SW_SHOWMINNOACTIVE: i32 = 7;
    const SW_RESTORE: i32 = 9;
    const GWLP_USERDATA: i32 = -21;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
    const WS_EX_APPWINDOW: isize = 0x0004_0000;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    const CMD_OPEN: usize = 1001;
    const CMD_PLAY: usize = 1002;
    const CMD_EXIT: usize = 1003;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    struct NotifyIconDataW {
        cb_size: u32,
        h_wnd: *mut c_void,
        u_id: u32,
        u_flags: u32,
        u_callback_message: u32,
        h_icon: *mut c_void,
        sz_tip: [u16; 128],
        dw_state: u32,
        dw_state_mask: u32,
        sz_info: [u16; 256],
        u_version_or_timeout: u32,
        sz_info_title: [u16; 64],
        dw_info_flags: u32,
        guid_item: Guid,
        h_balloon_icon: *mut c_void,
    }

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        lpfn_wnd_proc: unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: *mut c_void,
        h_icon: *mut c_void,
        h_cursor: *mut c_void,
        hbr_background: *mut c_void,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: *mut c_void,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: *mut c_void,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
        l_private: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> *mut c_void;
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(lpwcx: *const WndClassExW) -> u16;
        fn UnregisterClassW(lp_class_name: *const u16, h_instance: *mut c_void) -> i32;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: *mut c_void,
            h_menu: *mut c_void,
            h_instance: *mut c_void,
            lp_param: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(h_wnd: *mut c_void) -> i32;
        fn DefWindowProcW(h_wnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> isize;
        fn GetMessageW(
            lp_msg: *mut Msg,
            h_wnd: *mut c_void,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> i32;
        fn TranslateMessage(lp_msg: *const Msg) -> i32;
        fn DispatchMessageW(lp_msg: *const Msg) -> isize;
        fn PostQuitMessage(n_exit_code: i32);
        fn PostMessageW(h_wnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> i32;
        fn SetWindowLongPtrW(h_wnd: *mut c_void, n_index: i32, dw_new_long: isize) -> isize;
        fn GetWindowLongPtrW(h_wnd: *mut c_void, n_index: i32) -> isize;
        fn GetSystemMetrics(n_index: i32) -> i32;
        fn LoadImageW(
            h_inst: *mut c_void,
            name: *const u16,
            type_: u32,
            cx: i32,
            cy: i32,
            fu_load: u32,
        ) -> *mut c_void;
        fn ShowWindow(h_wnd: *mut c_void, n_cmd_show: i32) -> i32;
        fn ShowWindowAsync(h_wnd: *mut c_void, n_cmd_show: i32) -> i32;
        fn SetWindowPos(
            h_wnd: *mut c_void,
            h_wnd_insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            u_flags: u32,
        ) -> i32;
        fn IsWindowVisible(h_wnd: *mut c_void) -> i32;
        fn IsIconic(h_wnd: *mut c_void) -> i32;
        fn InvalidateRect(h_wnd: *mut c_void, lp_rect: *const c_void, b_erase: i32) -> i32;
        fn SetForegroundWindow(h_wnd: *mut c_void) -> i32;
        fn BringWindowToTop(h_wnd: *mut c_void) -> i32;
        fn CreatePopupMenu() -> *mut c_void;
        fn AppendMenuW(
            h_menu: *mut c_void,
            u_flags: u32,
            u_id_new_item: usize,
            lp_new_item: *const u16,
        ) -> i32;
        fn TrackPopupMenu(
            h_menu: *mut c_void,
            u_flags: u32,
            x: i32,
            y: i32,
            n_reserved: i32,
            h_wnd: *mut c_void,
            prc_rect: *const c_void,
        ) -> i32;
        fn DestroyMenu(h_menu: *mut c_void) -> i32;
        fn GetCursorPos(lp_point: *mut Point) -> i32;
        fn RegisterWindowMessageW(lp_string: *const u16) -> u32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(dw_message: u32, lp_data: *mut NotifyIconDataW) -> i32;
    }

    fn to_wide_z(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn fill_tip(buf: &mut [u16; 128], tip: &str) {
        buf.fill(0);
        for (idx, code_unit) in std::ffi::OsStr::new(tip)
            .encode_wide()
            .take(buf.len().saturating_sub(1))
            .enumerate()
        {
            if let Some(slot) = buf.get_mut(idx) {
                *slot = code_unit;
            }
        }
    }

    fn make_nid(tray_hwnd: *mut c_void, hicon: *mut c_void) -> NotifyIconDataW {
        let mut tip = [0u16; 128];
        fill_tip(&mut tip, "ShaurMeaw — Deadlock Launcher");
        NotifyIconDataW {
            cb_size: size_of::<NotifyIconDataW>() as u32,
            h_wnd: tray_hwnd,
            u_id: 1,
            u_flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            u_callback_message: WM_APP_TRAY,
            h_icon: hicon,
            sz_tip: tip,
            dw_state: 0,
            dw_state_mask: 0,
            sz_info: [0u16; 256],
            u_version_or_timeout: NOTIFYICON_VERSION_4,
            sz_info_title: [0u16; 64],
            dw_info_flags: 0,
            guid_item: Guid {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0u8; 8],
            },
            h_balloon_icon: std::ptr::null_mut(),
        }
    }

    fn add_tray_icon(tray_hwnd: *mut c_void, hicon: *mut c_void) {
        let mut nid = make_nid(tray_hwnd, hicon);
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &raw mut nid) != 0 {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &raw mut nid);
            }
        }
    }

    fn remove_tray_icon(tray_hwnd: *mut c_void) {
        let mut nid = make_nid(tray_hwnd, std::ptr::null_mut());
        nid.u_flags = 0;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &raw mut nid);
        }
    }

    /// Проверяет по реальному Win32-состоянию HWND, скрыто ли окно или свёрнуто.
    pub fn is_window_really_hidden(main_hwnd: usize) -> bool {
        if main_hwnd == 0 {
            return false;
        }
        unsafe {
            let hwnd = main_hwnd as *mut c_void;
            IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0
        }
    }

    fn ex_style_for_tray_hide(ex: isize) -> isize {
        (ex | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW
    }

    fn ex_style_for_tray_restore(ex: isize) -> isize {
        (ex & !WS_EX_TOOLWINDOW) | WS_EX_APPWINDOW
    }

    /// Прячет главное окно в системный трей из основного UI-потока без `WS_VISIBLE = 0`.
    /// Временный `SW_HIDE` убирает кнопку с панели задач после смены стиля на `WS_EX_TOOLWINDOW`,
    /// а `SW_SHOWMINNOACTIVE` и увод за экран возвращают `WS_VISIBLE = 1` в свёрнутом виде,
    /// чтобы `eframe`/`winit` не зацикливал `RedrawRequested` вхолостую на скрытом окне.
    pub fn hide_window_to_tray_ui_thread(main_hwnd: usize) {
        if main_hwnd == 0 {
            return;
        }
        unsafe {
            let hwnd = main_hwnd as *mut c_void;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let ex_tool = ex_style_for_tray_hide(ex);
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_tool);
            let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
            let _ = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                -32000,
                -32000,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    /// Выводит главное окно на передний план из основного UI-потока и принудительно перерисовывает его.
    pub fn bring_window_to_front_ui_thread(main_hwnd: usize) {
        if main_hwnd == 0 {
            return;
        }
        unsafe {
            let hwnd = main_hwnd as *mut c_void;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let had_tool_style = (ex & WS_EX_TOOLWINDOW) != 0 || (ex & WS_EX_APPWINDOW) == 0;
            if had_tool_style {
                let ex_app = ex_style_for_tray_restore(ex);
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_app);
            }
            if IsIconic(hwnd) != 0 {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            if had_tool_style {
                let _ = SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }

    fn restore_main_window(main_hwnd: usize, pending_restore: &AtomicBool, ctx: &egui::Context) {
        if main_hwnd == 0 {
            return;
        }
        pending_restore.store(true, Ordering::Release);
        unsafe {
            let hwnd = main_hwnd as *mut c_void;
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }

    struct TrayWindowState {
        main_hwnd: usize,
        hicon: *mut c_void,
        taskbar_created_msg: u32,
        ctx: egui::Context,
        pending_play: Arc<AtomicBool>,
        pending_restore: Arc<AtomicBool>,
    }

    fn show_context_menu(tray_hwnd: *mut c_void) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let open_lbl = to_wide_z("Открыть ShaurMeaw");
            let play_lbl = to_wide_z("Играть в Deadlock");
            let exit_lbl = to_wide_z("Выход");

            let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, open_lbl.as_ptr());
            let _ = AppendMenuW(menu, MF_STRING, CMD_PLAY, play_lbl.as_ptr());
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, exit_lbl.as_ptr());

            let mut pt = Point::default();
            let _ = GetCursorPos(&raw mut pt);
            let _ = SetForegroundWindow(tray_hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                pt.x,
                pt.y,
                0,
                tray_hwnd,
                std::ptr::null(),
            );
            let _ = PostMessageW(tray_hwnd, WM_NULL, 0, 0);
            let _ = DestroyMenu(menu);
        }
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: *mut c_void,
        msg: u32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayWindowState;
        if let Some(state) = state_ptr.as_ref() {
            if state.taskbar_created_msg != 0 && msg == state.taskbar_created_msg {
                add_tray_icon(hwnd, state.hicon);
                return 0;
            }

            match msg {
                WM_APP_TRAY => {
                    let event = (l_param as u32) & 0xFFFF;
                    if event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
                        restore_main_window(state.main_hwnd, &state.pending_restore, &state.ctx);
                    } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                        show_context_menu(hwnd);
                    }
                    return 0;
                }
                WM_COMMAND => {
                    let cmd = w_param & 0xFFFF;
                    if cmd == CMD_OPEN {
                        restore_main_window(state.main_hwnd, &state.pending_restore, &state.ctx);
                    } else if cmd == CMD_PLAY {
                        state.pending_play.store(true, Ordering::Release);
                        state.ctx.request_repaint();
                    } else if cmd == CMD_EXIT {
                        remove_tray_icon(hwnd);
                        if state.main_hwnd != 0 {
                            let main_w = state.main_hwnd as *mut c_void;
                            let _ = PostMessageW(main_w, WM_CLOSE, 0, 0);
                        }
                        state.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        state.ctx.request_repaint();
                    }
                    return 0;
                }
                WM_APP_SHUTDOWN => {
                    remove_tray_icon(hwnd);
                    let _ = DestroyWindow(hwnd);
                    return 0;
                }
                WM_DESTROY => {
                    remove_tray_icon(hwnd);
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, w_param, l_param)
    }

    /// Контроллер нативной иконки в области уведомлений Windows (System Tray).
    pub struct TrayController {
        tray_hwnd: Arc<AtomicUsize>,
        pending_play: Arc<AtomicBool>,
        pending_restore: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl TrayController {
        /// Запускает фоновый поток системного трея для главного окна `main_hwnd`.
        pub fn spawn(ctx: egui::Context, main_hwnd: usize) -> Option<Self> {
            if main_hwnd == 0 {
                return None;
            }
            let tray_hwnd = Arc::new(AtomicUsize::new(0));
            let pending_play = Arc::new(AtomicBool::new(false));
            let pending_restore = Arc::new(AtomicBool::new(false));

            let th_hwnd = Arc::clone(&tray_hwnd);
            let th_play = Arc::clone(&pending_play);
            let th_restore = Arc::clone(&pending_restore);
            let th_ctx = ctx;

            let worker = std::thread::Builder::new()
                .name("shaurmeaw-tray".to_owned())
                .spawn(move || unsafe {
                    let h_inst = GetModuleHandleW(std::ptr::null());
                    if h_inst.is_null() {
                        return;
                    }
                    let class_name = to_wide_z("ShaurMeawTrayClass_v1");
                    let wc = WndClassExW {
                        cb_size: size_of::<WndClassExW>() as u32,
                        style: 0,
                        lpfn_wnd_proc: tray_wnd_proc,
                        cb_cls_extra: 0,
                        cb_wnd_extra: 0,
                        h_instance: h_inst,
                        h_icon: std::ptr::null_mut(),
                        h_cursor: std::ptr::null_mut(),
                        hbr_background: std::ptr::null_mut(),
                        lpsz_menu_name: std::ptr::null(),
                        lpsz_class_name: class_name.as_ptr(),
                        h_icon_sm: std::ptr::null_mut(),
                    };
                    let _ = RegisterClassExW(&raw const wc);

                    let wnd_name = to_wide_z("");
                    let hwnd = CreateWindowExW(
                        0,
                        class_name.as_ptr(),
                        wnd_name.as_ptr(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        h_inst,
                        std::ptr::null_mut(),
                    );
                    if hwnd.is_null() {
                        let _ = UnregisterClassW(class_name.as_ptr(), h_inst);
                        return;
                    }

                    let sm_w = GetSystemMetrics(SM_CXSMICON).max(16);
                    let sm_h = GetSystemMetrics(SM_CYSMICON).max(16);
                    let hicon =
                        LoadImageW(h_inst, 1usize as *const u16, IMAGE_ICON, sm_w, sm_h, LR_SHARED);
                    let taskbar_msg_name = to_wide_z("TaskbarCreated");
                    let taskbar_created_msg = RegisterWindowMessageW(taskbar_msg_name.as_ptr());

                    let state = Box::new(TrayWindowState {
                        main_hwnd,
                        hicon,
                        taskbar_created_msg,
                        ctx: th_ctx,
                        pending_play: th_play,
                        pending_restore: th_restore,
                    });
                    let state_ptr = Box::into_raw(state);
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);

                    add_tray_icon(hwnd, hicon);
                    th_hwnd.store(hwnd as usize, Ordering::Release);

                    let mut msg = std::mem::zeroed::<Msg>();
                    while GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        let _ = TranslateMessage(&raw const msg);
                        let _ = DispatchMessageW(&raw const msg);
                    }

                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state_ptr));
                    let _ = UnregisterClassW(class_name.as_ptr(), h_inst);
                })
                .ok()?;

            Some(Self {
                tray_hwnd,
                pending_play,
                pending_restore,
                worker: Some(worker),
            })
        }

        /// Забирает отложенный запрос «Играть в Deadlock» из меню трея.
        pub fn take_pending_play(&self) -> bool {
            self.pending_play.swap(false, Ordering::AcqRel)
        }

        /// Забирает отложенный сигнал о разворачивании окна из трея.
        pub fn take_pending_restore(&self) -> bool {
            self.pending_restore.swap(false, Ordering::AcqRel)
        }

        /// Корректно удаляет иконку трея и завершает поток цикла сообщений.
        pub fn shutdown(&mut self) {
            let hwnd = self.tray_hwnd.swap(0, Ordering::AcqRel);
            if hwnd != 0 {
                unsafe {
                    let _ = PostMessageW(hwnd as *mut c_void, WM_APP_SHUTDOWN, 0, 0);
                }
            }
            if let Some(handle) = self.worker.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for TrayController {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_notify_icon_data_size_matches_win64_abi() {
            assert_eq!(size_of::<NotifyIconDataW>(), 976);
        }

        #[test]
        fn test_is_window_really_hidden_zero_hwnd_is_false() {
            assert!(!is_window_really_hidden(0));
            hide_window_to_tray_ui_thread(0);
        }

        #[test]
        fn test_tray_ex_style_roundtrip() {
            let hidden = ex_style_for_tray_hide(0x40110);
            assert_eq!(hidden, 0x190);
            assert_eq!(ex_style_for_tray_restore(hidden), 0x40110);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use eframe::egui;

    pub fn is_window_really_hidden(_main_hwnd: usize) -> bool {
        false
    }

    pub fn hide_window_to_tray_ui_thread(_main_hwnd: usize) {}

    pub fn bring_window_to_front_ui_thread(_main_hwnd: usize) {}

    pub struct TrayController;

    impl TrayController {
        pub fn spawn(_ctx: egui::Context, _main_hwnd: usize) -> Option<Self> {
            None
        }
        pub fn take_pending_play(&self) -> bool {
            false
        }
        pub fn take_pending_restore(&self) -> bool {
            false
        }
        pub fn shutdown(&mut self) {}
    }
}

pub use imp::{
    bring_window_to_front_ui_thread, hide_window_to_tray_ui_thread, is_window_really_hidden,
    TrayController,
};
