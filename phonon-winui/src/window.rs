//! Native window base class.
//!
//! `NativeWindow` handles the boilerplate of creating a borderless,
//! per-monitor-DPI-aware Win32 window and routes messages to a `Page`
//! trait object.  Applications implement `Page` to define their UI
//! and behavior.
//!
//! # Navigation
//!
//! Pages can switch to another page by calling [`Navigator::navigate`]
//! with the new page.  The swap happens asynchronously on the next
//! message loop iteration (via `PostMessage`).

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::dpi::{init_dpi_awareness, s};
use crate::gdi::*;
use crate::theme::Theme;

/// Custom message constants (WM_USER range).
pub const WM_NAVIGATE: u32 = WM_USER;       // LPARAM = *mut Box<dyn Page> (thin pointer)
pub const WM_CUSTOM_START: u32 = WM_USER + 1; // Pages can use from here

/// Force a full repaint of the window.
pub fn invalidate_window(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

/// Configuration for a native window.
pub struct WindowConfig {
    pub class_name: &'static str,
    pub title: &'static str,
    pub width: i32,
    pub height: i32,
}

/// A page (screen) of UI content.
pub trait Page {
    /// Called once when the page becomes active, before first paint.
    fn on_enter(&mut self, _hwnd: HWND) {}

    /// Called when the page is about to be replaced.
    fn on_leave(&mut self, _hwnd: HWND) {}

    /// Paint the page content into the content area.
    fn paint(&mut self, hdc: HDC, content_rc: &RECT, theme: &Theme);

    /// Left mouse button pressed.  Coordinates are window-relative.
    fn on_lbutton_down(&mut self, _hwnd: HWND, _x: i32, _y: i32) {}

    /// Mouse moved.  Coordinates are window-relative.
    fn on_mouse_move(&mut self, _hwnd: HWND, _x: i32, _y: i32) {}

    /// Left mouse button released.
    fn on_lbutton_up(&mut self, _hwnd: HWND, _x: i32, _y: i32) {}

    /// Mouse wheel scrolled. delta is the wheel delta (typically ±120).
    fn on_mouse_wheel(&mut self, _hwnd: HWND, _x: i32, _y: i32, _delta: i32) {}

    /// Timer tick.
    fn on_timer(&mut self, _hwnd: HWND, _timer_id: usize) {}

    /// Custom message (for cross-thread communication).
    ///
    /// Message IDs >= `WM_USER + 1` are forwarded to this handler.
    fn on_user_message(&mut self, _hwnd: HWND, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) {}
}

/// Page navigation helper.
pub struct Navigator;

impl Navigator {
    /// Switch to a new page.
    ///
    /// The swap is posted to the message queue. The current page's
    /// `on_leave` will be called, then the new page's `on_enter`.
    ///
    /// Must be called from the UI thread.
    pub fn navigate(hwnd: HWND, new_page: Box<dyn Page>) {
        // Wrap in another Box to get a thin pointer (Box<dyn Page> is fat,
        // but Box<Box<dyn Page>> is a thin pointer to a heap allocation
        // that holds the fat pointer).  This fits in LPARAM.
        let wrapped = Box::new(new_page);
        let raw = Box::into_raw(wrapped);
        unsafe {
            let _ = PostMessageW(
                hwnd,
                WM_NAVIGATE,
                WPARAM(0),
                LPARAM(raw as isize),
            );
        }
    }

    /// Close the window.
    pub fn close(hwnd: HWND) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// A native borderless window that hosts a `Page`.
pub struct NativeWindow;

struct WindowState {
    page: Box<dyn Page>,
    theme: Theme,
    config: WindowConfig,
    dragging: bool,
    drag_start: (i32, i32),
    hover_close: bool,
}

impl NativeWindow {
    /// Create and show the window.  Does not return until the window closes.
    pub fn run(config: WindowConfig, initial_page: Box<dyn Page>) -> i32 {
        init_dpi_awareness();

        let hinstance = unsafe { GetModuleHandleW(None) }.expect("GetModuleHandleW");

        let wcex = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap(),
            hbrBackground: unsafe { HBRUSH(GetStockObject(NULL_BRUSH).0) },
            lpszClassName: w!("PhononWinBase"),
            ..Default::default()
        };

        let atom = unsafe { RegisterClassExW(&wcex) };
        assert!(atom != 0, "RegisterClassExW failed");

        // Also register the specific class name for this app.
        let app_wcex = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap(),
            hbrBackground: unsafe { HBRUSH(GetStockObject(NULL_BRUSH).0) },
            lpszClassName: windows_str(config.class_name),
            ..Default::default()
        };
        let _ = unsafe { RegisterClassExW(&app_wcex) };

        let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let w = s(config.width);
        let h = s(config.height);
        let x = (sw - w) / 2;
        let y = (sh - h) / 2;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_APPWINDOW | WS_EX_LAYERED,
                windows_str(config.class_name),
                windows_str(config.title),
                WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
                x, y, w, h,
                None, None, hinstance, None,
            )
        }
        .expect("CreateWindowExW");

        // Set up state
        let mut state = Box::new(WindowState {
            page: initial_page,
            theme: Theme::new(),
            config,
            dragging: false,
            drag_start: (0, 0),
            hover_close: false,
        });

        // Notify page it's entering (before first paint)
        state.page.on_enter(hwnd);

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

            // Layered window for transparency (shadow keying)
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0x00000000), 255, LWA_COLORKEY);
            let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
            let _ = UpdateWindow(hwnd);
        }

        // Message loop
        let mut msg = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if !ret.as_bool() {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        msg.wParam.0 as i32
    }
}

// Helper: convert a &'static str to a PCWSTR (leaks — called once at startup).
fn windows_str(s: &'static str) -> windows::core::PCWSTR {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    let leaked = v.leak();
    windows::core::PCWSTR(leaked.as_ptr())
}

// ============ Window Procedure ============

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NAVIGATE => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);

                // Call on_leave on the old page
                state.page.on_leave(hwnd);

                // Unwrap the outer Box to get the new page
                let outer = Box::from_raw(lparam.0 as *mut Box<dyn Page>);
                state.page = *outer;

                // Call on_enter on the new page
                state.page.on_enter(hwnd);

                // Force repaint
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_PAINT => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                on_paint(hwnd, state);
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                let x = lparam_x(lparam.0);
                let y = lparam_y(lparam.0);

                // Close button hit
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let w = rc.right - rc.left;
                let close_rc = close_button_rect(w);

                if point_in_rect(x, y, &close_rc) {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }

                // Title bar drag
                let title_rc = title_bar_rect(w);
                if point_in_rect(x, y, &title_rc) {
                    state.dragging = true;
                    state.drag_start = (x, y);
                    let _ = ReleaseCapture();
                    return LRESULT(0);
                }

                state.page.on_lbutton_down(hwnd, x, y);
            }
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                if state.dragging {
                    state.dragging = false;
                    let _ = ReleaseCapture();
                }
                let x = lparam_x(lparam.0);
                let y = lparam_y(lparam.0);
                state.page.on_lbutton_up(hwnd, x, y);
            }
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                let x = lparam_x(lparam.0);
                let y = lparam_y(lparam.0);

                // Dragging
                if state.dragging {
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut rect);
                    let dx = x - state.drag_start.0;
                    let dy = y - state.drag_start.1;
                    let _ = SetWindowPos(
                        hwnd, None,
                        rect.left + dx, rect.top + dy,
                        0, 0,
                        SWP_NOSIZE | SWP_NOZORDER,
                    );
                    return LRESULT(0);
                }

                // Close button hover
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let w = rc.right - rc.left;
                let close_rc = close_button_rect(w);
                let hovering_close = point_in_rect(x, y, &close_rc);
                if hovering_close != state.hover_close {
                    state.hover_close = hovering_close;
                    let _ = InvalidateRect(hwnd, None, false);
                }

                state.page.on_mouse_move(hwnd, x, y);
            }
            LRESULT(0)
        }

        WM_MOUSEWHEEL => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let mut pt = POINT {
                    x: lparam_x(lparam.0),
                    y: lparam_y(lparam.0),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                state.page.on_mouse_wheel(hwnd, pt.x, pt.y, delta);
            }
            LRESULT(0)
        }

        WM_TIMER => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                state.page.on_timer(hwnd, wparam.0 as usize);
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let mut state = Box::from_raw(ptr as *mut WindowState);
                state.page.on_leave(hwnd);
                state.theme.destroy();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }

        // Forward WM_USER+1 and above to the page
        m if m > WM_NAVIGATE => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if state_ptr != 0 {
                let state = &mut *(state_ptr as *mut WindowState);
                state.page.on_user_message(hwnd, m, wparam, lparam);
            }
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_paint(hwnd: HWND, state: &mut WindowState) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;

        with_double_buffer(hdc, &rc, |mem_dc| {
            // Clear with transparent black for layered window
            let clear_br = CreateSolidBrush(COLORREF(0x00000000));
            let _ = FillRect(mem_dc, &rc, clear_br);
            let _ = DeleteObject(clear_br);

            // Window frame + title bar + close button
            draw_window_frame(mem_dc, &state.theme, state.config.title, w, h);

            // Content area
            let content = content_rect(w, h);
            state.page.paint(mem_dc, &content, &state.theme);
        });

        let _ = EndPaint(hwnd, &ps);
    }
}
