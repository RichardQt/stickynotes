//! Win32 pieces the UI toolkit does not cover: the notification-area icon
//! and its menu, the system-wide quick-note key, the single-instance guard,
//! starting with Windows, and hiding / showing / placing top-level windows.
//!
//! Window handles travel as plain `isize` values so they can cross threads
//! and sit in the app state.

/// What the user asked for through the notification-area icon, its
/// right-click menu or the system-wide shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click, the menu or the shortcut: open the quick-note window.
    QuickNote,
    /// The menu — or the app was started a second time: show the main
    /// window.
    ShowMain,
    /// The menu: show the main window with the settings open.
    OpenSettings,
    /// The menu: quit the app.
    Quit,
}

#[cfg(windows)]
pub use imp::*;
#[cfg(not(windows))]
pub use stub::*;

#[cfg(windows)]
mod imp {
    use super::TrayEvent;
    use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::time::Duration;
    use stickynotes_core::shortcuts::Combo;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, VkKeyScanW, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
        MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_F1,
    };
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_LARGE_ICON,
        NIIF_USER, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW,
        GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        LoadIconW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SendMessageTimeoutW, SetForegroundWindow, SetMenuDefaultItem,
        SetWindowPos, ShowWindow, ShowWindowAsync, TrackPopupMenu, TranslateMessage, HICON,
        IDI_APPLICATION, IMAGE_ICON, LR_SHARED, MF_SEPARATOR, MF_STRING, MSG, SMTO_ABORTIFHUNG,
        SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, SYSTEM_METRICS_INDEX, TPM_BOTTOMALIGN,
        TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_STYLE, WM_APP, WM_DESTROY,
        WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
    };
    use windows_registry::CURRENT_USER;

    const CONTROL_CLASS: PCWSTR = w!("StickyNotesControl");

    /// The notification-area icon reports mouse messages through this one.
    const MSG_TRAY: u32 = WM_APP + 1;
    const MSG_SHOW_ICON: u32 = WM_APP + 2;
    const MSG_HIDE_ICON: u32 = WM_APP + 3;
    const MSG_HINT: u32 = WM_APP + 4;
    /// Posted by a second instance of the app before it exits.
    const MSG_SHOW_MAIN: u32 = WM_APP + 5;
    const MSG_SHUTDOWN: u32 = WM_APP + 6;
    /// Register the system-wide key in wParam (modifiers) and lParam (key
    /// code, 0 for none) in place of the current one. Answers 1 when done.
    const MSG_SET_HOTKEY: u32 = WM_APP + 7;

    const TRAY_ICON_ID: u32 = 1;
    const TRAY_TIP: &str = "便签 · 左键快速记事，右键打开菜单";
    const HINT_TITLE: &str = "便签仍在后台运行";
    const HINT_TEXT: &str =
        "左键点击托盘图标快速记事，右键打开菜单。可在“设置 › 通用”里改为关闭窗口时直接退出。";

    /// The entries of the icon's right-click menu.
    const MENU_SHOW_MAIN: usize = 1;
    const MENU_QUICK_NOTE: usize = 2;
    const MENU_SETTINGS: usize = 3;
    const MENU_QUIT: usize = 4;

    /// The one system-wide key of the control window.
    const HOTKEY_ID: i32 = 1;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// Copy `text` into a fixed, NUL-terminated UTF-16 field, cutting it short
    /// when it does not fit.
    fn fill<const N: usize>(field: &mut [u16; N], text: &str) {
        let mut len = 0;
        for unit in text.encode_utf16() {
            if len + 1 >= N {
                break;
            }
            field[len] = unit;
            len += 1;
        }
        field[len] = 0;
    }

    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// Names the profile (the settings folder) this process works on. Two
    /// processes started with different `LOCALAPPDATA` folders hold different
    /// notes and must not treat each other as "already running".
    fn profile_key() -> String {
        let dir = stickynotes_core::config::config_dir();
        let text = dir.to_string_lossy().to_lowercase();
        // FNV-1a: the same value in every run and every build.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    fn control_window_name() -> String {
        format!("stickynotes-{}", profile_key())
    }

    // --- Single instance -------------------------------------------------

    /// Take the per-profile instance lock.
    ///
    /// Returns `false` when another instance already works on this profile:
    /// it has been asked to show its main window and this process should
    /// exit. Two instances would write the same database and sync it twice.
    pub fn claim_single_instance() -> bool {
        let name = wide(&format!("Local\\StickyNotes-{}", profile_key()));
        // The handle is never closed: the lock lasts as long as the process.
        let already_running = unsafe {
            match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
                Ok(_) => GetLastError() == ERROR_ALREADY_EXISTS,
                // Cannot tell. Starting is better than refusing to start.
                Err(_) => false,
            }
        };
        if already_running {
            show_running_instance();
        }
        !already_running
    }

    /// Ask the instance that holds the lock to come to the front. It may
    /// still be starting, so its control window is looked for a few times.
    fn show_running_instance() {
        let name = wide(&control_window_name());
        for _ in 0..25 {
            let found = unsafe { FindWindowW(CONTROL_CLASS, PCWSTR(name.as_ptr())) };
            if let Ok(control) = found {
                unsafe {
                    let mut process = 0u32;
                    GetWindowThreadProcessId(control, Some(&mut process));
                    // This process was just started by the user, so it may
                    // hand its right to take the foreground to the other one.
                    let _ = AllowSetForegroundWindow(process);
                    let _ = PostMessageW(Some(control), MSG_SHOW_MAIN, WPARAM(0), LPARAM(0));
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // --- Starting with Windows -------------------------------------------

    /// The programs Windows starts when this user signs in.
    fn run_key() -> String {
        format!(r"{}\Run", run_root())
    }

    /// Where Task Manager and the Settings app note which of them the user
    /// switched off there.
    fn run_approved_key() -> String {
        format!(r"{}\Explorer\StartupApproved\Run", run_root())
    }

    /// The key under `HKEY_CURRENT_USER` both of them sit in.
    /// `STICKYNOTES_RUN_ROOT` names another one, to try the switch out
    /// without registering anything with Windows.
    fn run_root() -> String {
        std::env::var("STICKYNOTES_RUN_ROOT")
            .unwrap_or_else(|_| r"Software\Microsoft\Windows\CurrentVersion".into())
    }

    const RUN_VALUE: &str = "StickyNotes";
    /// Tells a start by Windows at sign-in from one by the user.
    const AUTOSTART_ARG: &str = "--autostart";

    /// The command Windows is given to run at sign-in: this executable.
    fn autostart_command() -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        Some(format!("\"{}\" {AUTOSTART_ARG}", exe.to_str()?))
    }

    /// This process was started by Windows at sign-in.
    pub fn launched_at_login() -> bool {
        std::env::args_os().skip(1).any(|arg| arg == AUTOSTART_ARG)
    }

    /// Whether Windows starts this executable at sign-in. Read from the
    /// system every time: the entry can also be switched off in Task Manager,
    /// and it stops counting once the executable has moved.
    pub fn autostart_enabled() -> bool {
        let Some(command) = autostart_command() else {
            return false;
        };
        let registered = CURRENT_USER
            .open(run_key())
            .and_then(|key| key.get_string(RUN_VALUE))
            .is_ok_and(|stored| stored.eq_ignore_ascii_case(&command));
        // An odd first byte means "disabled"; no value at all, "enabled".
        let switched_off = CURRENT_USER
            .open(run_approved_key())
            .and_then(|key| key.get_value(RUN_VALUE))
            .is_ok_and(|flags| flags.first().is_some_and(|byte| byte & 1 == 1));
        registered && !switched_off
    }

    /// Add this executable to the programs started at sign-in, or take it
    /// out again. The error is ready to be shown.
    pub fn set_autostart(on: bool) -> Result<(), String> {
        // A "disabled" noted by Task Manager outlives the entry it was about
        // and would keep a new one from starting.
        if let Ok(approved) = CURRENT_USER.options().write().open(run_approved_key()) {
            let _ = approved.remove_value(RUN_VALUE);
        }
        let run = CURRENT_USER.create(run_key()).map_err(|e| e.message())?;
        if on {
            let command = autostart_command().ok_or("无法确定程序所在的位置")?;
            run.set_string(RUN_VALUE, command).map_err(|e| e.message())
        } else {
            match run.remove_value(RUN_VALUE) {
                // Nothing there to remove is just as good.
                Err(_) if run.get_type(RUN_VALUE).is_err() => Ok(()),
                result => result.map_err(|e| e.message()),
            }
        }
    }

    // --- Notification-area icon ------------------------------------------

    /// The hidden control window and the notification-area icon. Both live on
    /// a thread of their own, so the icon answers even while the UI is busy.
    pub struct Tray {
        control: isize,
        /// The control window has been told to go; its handle is stale.
        closed: Cell<bool>,
    }

    impl Tray {
        /// Start the control thread. The icon itself only appears after
        /// [`Tray::show_icon`]. Events arrive on the returned channel.
        pub fn start() -> Option<(Self, UnboundedReceiver<TrayEvent>)> {
            let (events, receiver) = unbounded();
            let (ready, started) = std::sync::mpsc::channel::<Option<isize>>();
            std::thread::Builder::new()
                .name("stickynotes-tray".into())
                .spawn(move || run_control_thread(events, ready))
                .ok()?;
            let control = started.recv().ok().flatten()?;
            Some((Self { control, closed: Cell::new(false) }, receiver))
        }

        pub fn show_icon(&self) {
            self.post(MSG_SHOW_ICON);
        }

        pub fn hide_icon(&self) {
            self.post(MSG_HIDE_ICON);
        }

        /// One-off balloon telling the user the app is still running.
        pub fn show_hint(&self) {
            self.post(MSG_HINT);
        }

        /// Make `combo` open the quick note from any program, in place of
        /// the key that did so far; `None` leaves no such key. The error is
        /// ready to be shown.
        pub fn set_hotkey(&self, combo: Option<&Combo>) -> Result<(), &'static str> {
            if self.closed.get() {
                return Ok(());
            }
            let (modifiers, code) = match combo {
                Some(combo) => {
                    hotkey_parts(combo).ok_or("这个按键不能注册为全局快捷键，请换一个组合")?
                }
                None => (0, 0),
            };
            // Hot keys belong to the thread that registers them, and they are
            // reported to the control window: its thread has to do it.
            let mut registered = 0usize;
            unsafe {
                SendMessageTimeoutW(
                    hwnd(self.control),
                    MSG_SET_HOTKEY,
                    WPARAM(modifiers as usize),
                    LPARAM(code as isize),
                    SMTO_ABORTIFHUNG,
                    1500,
                    Some(&mut registered),
                );
            }
            if code == 0 || registered != 0 {
                Ok(())
            } else {
                Err("这个组合键已被其他程序占用，当前不会生效，请换一个组合")
            }
        }

        /// Remove the icon and end the control thread. Waits (briefly) until
        /// the icon is gone, so no dead icon is left behind when the process
        /// exits right afterwards.
        pub fn shutdown(&self) {
            if self.closed.replace(true) {
                return;
            }
            unsafe {
                SendMessageTimeoutW(
                    hwnd(self.control),
                    MSG_SHUTDOWN,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG,
                    1500,
                    None,
                );
            }
        }

        fn post(&self, message: u32) {
            if self.closed.get() {
                return;
            }
            unsafe {
                let _ = PostMessageW(Some(hwnd(self.control)), message, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// However the app ends, the icon must not stay behind as a dead one.
    impl Drop for Tray {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    /// State of the control thread, reached from its window procedure.
    struct Control {
        events: RefCell<Option<UnboundedSender<TrayEvent>>>,
        /// The icon belongs in the notification area. It is put back when
        /// Explorer restarts and announces a new taskbar.
        icon_wanted: Cell<bool>,
        /// Id of the registered "TaskbarCreated" message; 0 when unknown.
        taskbar_created: Cell<u32>,
    }

    thread_local! {
        static CONTROL: Control = const {
            Control {
                events: RefCell::new(None),
                icon_wanted: Cell::new(false),
                taskbar_created: Cell::new(0),
            }
        };
    }

    fn run_control_thread(
        events: UnboundedSender<TrayEvent>,
        ready: std::sync::mpsc::Sender<Option<isize>>,
    ) {
        CONTROL.with(|control| {
            *control.events.borrow_mut() = Some(events);
            control
                .taskbar_created
                .set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
        });
        let name = wide(&control_window_name());
        let created = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(control_proc),
                lpszClassName: CONTROL_CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            // Top-level (so a second instance can find it by name) but never
            // shown: a tool window has no taskbar button either.
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CONTROL_CLASS,
                PCWSTR(name.as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
        };
        let Ok(window) = created else {
            let _ = ready.send(None);
            return;
        };
        let _ = ready.send(Some(window.0 as isize));

        let mut message = MSG::default();
        // 0 is WM_QUIT, -1 an error: both end the loop.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    fn emit(event: TrayEvent) {
        CONTROL.with(|control| {
            if let Some(events) = control.events.borrow().as_ref() {
                let _ = events.unbounded_send(event);
            }
        });
    }

    /// The app icon (resource 1, embedded by the build script) at the size
    /// the given system metrics ask for.
    fn app_icon(width: SYSTEM_METRICS_INDEX, height: SYSTEM_METRICS_INDEX) -> HICON {
        unsafe {
            let own = GetModuleHandleW(None).ok().and_then(|module| {
                LoadImageW(
                    Some(HINSTANCE(module.0)),
                    PCWSTR(1 as _),
                    IMAGE_ICON,
                    GetSystemMetrics(width),
                    GetSystemMetrics(height),
                    LR_SHARED,
                )
                .ok()
            });
            match own {
                Some(handle) => HICON(handle.0),
                None => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
            }
        }
    }

    fn icon_data(window: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: TRAY_ICON_ID,
            ..Default::default()
        }
    }

    fn add_icon(window: HWND) {
        let mut data = icon_data(window);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = MSG_TRAY;
        data.hIcon = app_icon(SM_CXSMICON, SM_CYSMICON);
        fill(&mut data.szTip, TRAY_TIP);
        CONTROL.with(|control| control.icon_wanted.set(true));
        unsafe {
            // Adding fails when the icon is already there; refresh it then.
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    fn remove_icon(window: HWND) {
        CONTROL.with(|control| control.icon_wanted.set(false));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(window));
        }
    }

    fn show_hint(window: HWND) {
        if !CONTROL.with(|control| control.icon_wanted.get()) {
            return;
        }
        let mut data = icon_data(window);
        data.uFlags = NIF_INFO;
        fill(&mut data.szInfoTitle, HINT_TITLE);
        fill(&mut data.szInfo, HINT_TEXT);
        let large = app_icon(SM_CXICON, SM_CYICON);
        if large.is_invalid() {
            data.dwInfoFlags = NIIF_INFO;
        } else {
            data.hBalloonIcon = large;
            data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Modifier flags and virtual-key code of `combo`, as Windows registers
    /// hot keys. `None` for a key the current keyboard layout does not have.
    fn hotkey_parts(combo: &Combo) -> Option<(u32, u32)> {
        let key = combo.key();
        let function_key = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok());
        let code = match function_key {
            Some(n @ 1..=24) => u32::from(VK_F1.0) + n - 1,
            _ => {
                let mut units = key.encode_utf16();
                let (Some(unit), None) = (units.next(), units.next()) else {
                    return None;
                };
                // The key that types this character; its code is the low byte.
                let scan = unsafe { VkKeyScanW(unit) };
                if scan == -1 {
                    return None;
                }
                u32::from(scan as u16 & 0xFF)
            }
        };
        // Holding the keys down opens the quick note once, not over and over.
        let mut modifiers = MOD_NOREPEAT;
        for (held, flag) in [
            (combo.ctrl, MOD_CONTROL),
            (combo.alt, MOD_ALT),
            (combo.shift, MOD_SHIFT),
            (combo.win, MOD_WIN),
        ] {
            if held {
                modifiers |= flag;
            }
        }
        Some((modifiers.0, code))
    }

    /// The icon's right-click menu, at the mouse pointer. Returns once an
    /// entry was chosen or the menu was dismissed.
    fn tray_menu(window: HWND) -> Option<TrayEvent> {
        let chosen = unsafe {
            let menu = CreatePopupMenu().ok()?;
            for (id, text) in [
                (MENU_SHOW_MAIN, "打开主面板"),
                (MENU_QUICK_NOTE, "快速记事"),
                (MENU_SETTINGS, "设置…"),
            ] {
                let text = wide(text);
                let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(text.as_ptr()));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let quit = wide("退出便签");
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit.as_ptr()));
            // Shown in bold: what a double click on such an icon usually does.
            let _ = SetMenuDefaultItem(menu, MENU_SHOW_MAIN as u32, 0);

            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            // A menu whose owner is not in the foreground stays open when the
            // user clicks somewhere else. This also lets the UI thread bring
            // its window forward afterwards.
            let _ = SetForegroundWindow(window);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                cursor.x,
                cursor.y,
                None,
                window,
                None,
            );
            // Without a message to follow, the menu may not close properly
            // the next time it is dismissed.
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            chosen.0 as usize
        };
        match chosen {
            MENU_SHOW_MAIN => Some(TrayEvent::ShowMain),
            MENU_QUICK_NOTE => Some(TrayEvent::QuickNote),
            MENU_SETTINGS => Some(TrayEvent::OpenSettings),
            MENU_QUIT => Some(TrayEvent::Quit),
            _ => None,
        }
    }

    unsafe extern "system" fn control_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            MSG_TRAY => {
                // Without NOTIFYICON_VERSION_4 the low word of lParam is the
                // mouse message itself.
                let event = match lparam.0 as u32 & 0xFFFF {
                    WM_LBUTTONUP => {
                        // The click entitles this process to the foreground
                        // right now. Take it here so the UI thread, which
                        // reacts a moment later, can bring its own window
                        // forward.
                        let _ = unsafe { SetForegroundWindow(window) };
                        Some(TrayEvent::QuickNote)
                    }
                    WM_RBUTTONUP => tray_menu(window),
                    _ => None,
                };
                if let Some(event) = event {
                    emit(event);
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                if wparam.0 as i32 == HOTKEY_ID {
                    // As with a click on the icon.
                    let _ = unsafe { SetForegroundWindow(window) };
                    emit(TrayEvent::QuickNote);
                }
                LRESULT(0)
            }
            MSG_SET_HOTKEY => {
                let _ = unsafe { UnregisterHotKey(Some(window), HOTKEY_ID) };
                let code = lparam.0 as u32;
                let registered = code == 0
                    || unsafe {
                        let modifiers = HOT_KEY_MODIFIERS(wparam.0 as u32);
                        RegisterHotKey(Some(window), HOTKEY_ID, modifiers, code).is_ok()
                    };
                LRESULT(registered as isize)
            }
            MSG_SHOW_MAIN => {
                let _ = unsafe { SetForegroundWindow(window) };
                emit(TrayEvent::ShowMain);
                LRESULT(0)
            }
            MSG_SHOW_ICON => {
                add_icon(window);
                LRESULT(0)
            }
            MSG_HIDE_ICON => {
                remove_icon(window);
                LRESULT(0)
            }
            MSG_HINT => {
                show_hint(window);
                LRESULT(0)
            }
            MSG_SHUTDOWN => {
                let _ = unsafe { UnregisterHotKey(Some(window), HOTKEY_ID) };
                remove_icon(window);
                let _ = unsafe { DestroyWindow(window) };
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => {
                let (taskbar_created, icon_wanted) = CONTROL
                    .with(|control| (control.taskbar_created.get(), control.icon_wanted.get()));
                if taskbar_created != 0 && message == taskbar_created && icon_wanted {
                    // Explorer restarted and forgot every icon.
                    add_icon(window);
                }
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
    }

    // --- Windows of the UI toolkit ---------------------------------------

    /// The native handle of a toolkit window.
    pub fn hwnd_of(window: &gpui_kit::Window) -> Option<isize> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }

    /// Hide a window. The request is only queued, so this is safe to call
    /// while the toolkit is in the middle of handling an event of that window.
    pub fn hide(window: isize) {
        unsafe {
            let _ = ShowWindowAsync(hwnd(window), SW_HIDE);
        }
    }

    /// Show a hidden or minimized window and bring it to the front.
    ///
    /// The window is messaged synchronously: call this from a task, never
    /// from inside one of the toolkit's update callbacks.
    pub fn show_and_activate(window: isize) {
        unsafe {
            let window = hwnd(window);
            let command = if IsIconic(window).as_bool() { SW_RESTORE } else { SW_SHOW };
            let _ = ShowWindow(window, command);
            let _ = SetForegroundWindow(window);
        }
    }

    pub fn is_visible(window: isize) -> bool {
        unsafe { IsWindowVisible(hwnd(window)).as_bool() }
    }

    /// Ask for rounded corners (Windows 11; ignored by older systems).
    pub fn round_corners(window: isize) {
        unsafe {
            let preference = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd(window),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const c_void,
                std::mem::size_of_val(&preference) as u32,
            );
        }
    }

    /// The monitor under the mouse pointer, as the id the UI toolkit uses
    /// for its displays.
    pub fn cursor_monitor() -> Option<u64> {
        unsafe {
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor).ok()?;
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            (!monitor.is_invalid()).then_some(monitor.0 as u64)
        }
    }

    /// Move a window into the bottom-right corner of the work area of the
    /// monitor under the mouse pointer — next to the notification area on a
    /// default taskbar. Meant for a hidden window; same calling rule as
    /// [`show_and_activate`].
    pub fn place_bottom_right(window: isize) {
        unsafe {
            let window = hwnd(window);
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_err() {
                return;
            }
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let work = info.rcWork;
            let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            // First onto that monitor: the window adopts its scale factor and
            // only then has the size the final position is computed from.
            let _ = SetWindowPos(window, None, work.left, work.top, 0, 0, flags);
            let mut rect = RECT::default();
            if GetWindowRect(window, &mut rect).is_err() {
                return;
            }
            let margin = (GetDpiForWindow(window) as i32).max(96) * 14 / 96;
            let x = (work.right - (rect.right - rect.left) - margin).max(work.left);
            let y = (work.bottom - (rect.bottom - rect.top) - margin).max(work.top);
            let _ = SetWindowPos(window, None, x, y, 0, 0, flags);
        }
    }
}

/// Other platforms have no notification-area icon here: closing the main
/// window always quits and the quick-note window is never asked for.
#[cfg(not(windows))]
mod stub {
    use super::TrayEvent;
    use futures::channel::mpsc::UnboundedReceiver;
    use stickynotes_core::shortcuts::Combo;

    pub struct Tray;

    impl Tray {
        pub fn start() -> Option<(Self, UnboundedReceiver<TrayEvent>)> {
            None
        }
        pub fn show_icon(&self) {}
        pub fn hide_icon(&self) {}
        pub fn show_hint(&self) {}
        pub fn set_hotkey(&self, _combo: Option<&Combo>) -> Result<(), &'static str> {
            Ok(())
        }
        pub fn shutdown(&self) {}
    }

    pub fn claim_single_instance() -> bool {
        true
    }
    pub fn launched_at_login() -> bool {
        false
    }
    pub fn autostart_enabled() -> bool {
        false
    }
    pub fn set_autostart(_on: bool) -> Result<(), String> {
        Err("这个系统上不支持开机自启动".into())
    }
    pub fn hwnd_of(_window: &gpui_kit::Window) -> Option<isize> {
        None
    }
    pub fn hide(_window: isize) {}
    pub fn show_and_activate(_window: isize) {}
    pub fn is_visible(_window: isize) -> bool {
        true
    }
    pub fn round_corners(_window: isize) {}
    pub fn cursor_monitor() -> Option<u64> {
        None
    }
    pub fn place_bottom_right(_window: isize) {}
}
