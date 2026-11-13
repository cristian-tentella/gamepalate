use iced::window::Window;

#[cfg(windows)]
#[allow(unsafe_code, reason = "the Win32 calls that set the window icon")]
pub fn use_exe_icon(window: &dyn Window) {
    use iced::window::raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::{
        Foundation::HWND,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, SM_CXICON, SM_CXSMICON,
            SendMessageW, WM_SETICON,
        },
    };

    const EXE_ICON: usize = 32512;

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;

    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetrics(metric);
            let icon = LoadImageW(module, EXE_ICON as *const u16, IMAGE_ICON, size, size, 0);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn use_exe_icon(_window: &dyn Window) {}
