#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::*;

// ============ Colors (COLORREF = 0x00BBGGRR) ============

const CLR_BG: u32 = 0x00140A0A;            // #0a0a14
const CLR_BG_LIGHT: u32 = 0x001E1010;       // #10101e
const CLR_CARD: u32 = 0x001D1111;           // #11111d
const CLR_CARD_HOVER: u32 = 0x002D1A1A;     // #1a1a2d
const CLR_CARD_SELECTED: u32 = 0x00281616;  // #161628
const CLR_BORDER: u32 = 0x003A2020;         // #20203a
const CLR_BORDER_GLOW: u32 = 0x00803C3C;    // #3c3c80
const CLR_WINDOW_BORDER: u32 = 0x004A2828;   // #28284a
const CLR_TEXT: u32 = 0x00B0A090;           // #90a0b0
const CLR_TEXT_BRIGHT: u32 = 0x00F0E8E0;    // #e0e8f0
const CLR_TEXT_DIM: u32 = 0x00706060;       // #606070
const CLR_ACCENT: u32 = 0x00FFD44A;         // #4ad4ff (bright cyan)
const CLR_ACCENT2: u32 = 0x00FF8888;        // #8888ff (bright indigo)
const CLR_ACCENT_SOFT: u32 = 0x00A08C5A;    // #5a8ca0 (muted cyan)
const CLR_ACCENT_GLOW: u32 = 0x00FFE880;    // #80e8ff
const CLR_DANGER: u32 = 0x006650FF;         // #ff5066
const CLR_DANGER_HOVER: u32 = 0x008060FF;   // #ff6080
const CLR_DANGER_SOFT: u32 = 0x00442088;    // #882044
const CLR_SUCCESS: u32 = 0x0060E848;        // #48e860
const CLR_SUCCESS_SOFT: u32 = 0x00307020;   // #207030
const CLR_WHITE: u32 = 0x00FFFFFF;          // #ffffff
const CLR_CLOSE_HOVER: u32 = 0x003018F0;    // #f01830
const CLR_HEADER_GRAD1: u32 = 0x00381414;   // #141438
const CLR_HEADER_GRAD2: u32 = 0x00140A0A;   // #0a0a14
const CLR_ICON_BG: u32 = 0x002A1515;        // #15152a
const CLR_ICON_BG_SELECTED: u32 = 0x003A2020; // #20203a

// ============ Layout constants (logical pixels @ 96 DPI) ============

const W_WIDTH: i32 = 480;
const W_HEIGHT: i32 = 500;
const SHADOW_SIZE: i32 = 4;
const WINDOW_RADIUS: i32 = 18;
const TITLE_BAR_H: i32 = 36;
const HEADER_H: i32 = 120;
const PADDING: i32 = 28;
const OPTION_START_Y: i32 = 150;
const OPTION_H: i32 = 58;
const OPTION_GAP: i32 = 10;
const OPTION_ICON_SIZE: i32 = 40;
const OPTION_ICON_X: i32 = 36;
const OPTION_TEXT_X: i32 = 92;
const BUTTON_W: i32 = 120;
const BUTTON_H: i32 = 40;
const BUTTONS_Y: i32 = 440;
const CLOSE_BTN_SIZE: i32 = 32;
const CLOSE_BTN_OFFSET: i32 = 4;
const CARD_RADIUS: i32 = 12;
const BTN_RADIUS: i32 = 20;

// ============ Custom messages ============

const WM_PROGRESS: u32 = WM_USER;
const WM_DONE: u32 = WM_USER + 1;
const TIMER_AUTO_CLOSE: usize = 1;
const TIMER_ANIMATE: usize = 2;

// ============ Phase ============

#[derive(Clone, Copy, PartialEq)]
enum Phase { Select, Confirm, Uninstalling, Done }

// ============ Option icon kinds ============

enum IconKind { Shield, Music, Trash, Eraser }

// ============ State ============

struct State {
    phase: Phase,
    selected: i32,
    hover_option: i32,
    hover_btn: i32,
    hover_close: bool,
    progress_step: i32,
    uninstalled: bool,
    anim_tick: u32,
    // GDI resources
    font_big: HFONT,
    font_title: HFONT,
    font_body: HFONT,
    font_body_bold: HFONT,
    font_small: HFONT,
    font_micro: HFONT,
    brush_bg: HBRUSH,
    brush_bg_light: HBRUSH,
    brush_card: HBRUSH,
    brush_card_hover: HBRUSH,
    brush_card_selected: HBRUSH,
    brush_accent: HBRUSH,
    brush_success: HBRUSH,
    brush_white: HBRUSH,
    brush_icon_bg: HBRUSH,
    brush_icon_bg_selected: HBRUSH,
    brush_danger: HBRUSH,
    brush_danger_soft: HBRUSH,
    pen_border: HPEN,
    pen_border_glow: HPEN,
    pen_accent: HPEN,
    pen_accent_glow: HPEN,
    pen_close: HPEN,
    pen_success: HPEN,
    pen_window: HPEN,
    pen_danger: HPEN,
}

fn make_font(pt_size: i32, weight: i32) -> HFONT {
    let dpi = get_dpi();
    let h = -pt_size * dpi / 72;
    unsafe {
        CreateFontW(
            h, 0, 0, 0, weight, 0, 0, 0, 0, 0, 0,
            5,  // CLEARTYPE_QUALITY
            0 | 32, // DEFAULT_PITCH | FF_SWISS
            w!("Segoe UI"),
        )
    }
}

impl State {
    fn new() -> Self {
        Self {
            phase: Phase::Select,
            selected: 3,
            hover_option: -1,
            hover_btn: -1,
            hover_close: false,
            progress_step: 0,
            uninstalled: false,
            anim_tick: 0,
            font_big: make_font(20, 700),
            font_title: make_font(13, 600),
            font_body: make_font(11, 400),
            font_body_bold: make_font(11, 600),
            font_small: make_font(9, 400),
            font_micro: make_font(8, 400),
            brush_bg: unsafe { CreateSolidBrush(COLORREF(CLR_BG)) },
            brush_bg_light: unsafe { CreateSolidBrush(COLORREF(CLR_BG_LIGHT)) },
            brush_card: unsafe { CreateSolidBrush(COLORREF(CLR_CARD)) },
            brush_card_hover: unsafe { CreateSolidBrush(COLORREF(CLR_CARD_HOVER)) },
            brush_card_selected: unsafe { CreateSolidBrush(COLORREF(CLR_CARD_SELECTED)) },
            brush_accent: unsafe { CreateSolidBrush(COLORREF(CLR_ACCENT)) },
            brush_success: unsafe { CreateSolidBrush(COLORREF(CLR_SUCCESS)) },
            brush_white: unsafe { CreateSolidBrush(COLORREF(CLR_WHITE)) },
            brush_icon_bg: unsafe { CreateSolidBrush(COLORREF(CLR_ICON_BG)) },
            brush_icon_bg_selected: unsafe { CreateSolidBrush(COLORREF(CLR_ICON_BG_SELECTED)) },
            brush_danger: unsafe { CreateSolidBrush(COLORREF(CLR_DANGER)) },
            brush_danger_soft: unsafe { CreateSolidBrush(COLORREF(CLR_DANGER_SOFT)) },
            pen_border: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_BORDER)) },
            pen_border_glow: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_BORDER_GLOW)) },
            pen_accent: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_ACCENT)) },
            pen_accent_glow: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_ACCENT_GLOW)) },
            pen_close: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_TEXT_DIM)) },
            pen_success: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_SUCCESS)) },
            pen_window: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_WINDOW_BORDER)) },
            pen_danger: unsafe { CreatePen(PS_SOLID, 1, COLORREF(CLR_DANGER)) },
        }
    }

    fn destroy(&self) {
        unsafe {
            let _ = DeleteObject(self.font_big);
            let _ = DeleteObject(self.font_title);
            let _ = DeleteObject(self.font_body);
            let _ = DeleteObject(self.font_body_bold);
            let _ = DeleteObject(self.font_small);
            let _ = DeleteObject(self.font_micro);
            let _ = DeleteObject(self.brush_bg);
            let _ = DeleteObject(self.brush_bg_light);
            let _ = DeleteObject(self.brush_card);
            let _ = DeleteObject(self.brush_card_hover);
            let _ = DeleteObject(self.brush_card_selected);
            let _ = DeleteObject(self.brush_accent);
            let _ = DeleteObject(self.brush_success);
            let _ = DeleteObject(self.brush_white);
            let _ = DeleteObject(self.brush_icon_bg);
            let _ = DeleteObject(self.brush_icon_bg_selected);
            let _ = DeleteObject(self.brush_danger);
            let _ = DeleteObject(self.brush_danger_soft);
            let _ = DeleteObject(self.pen_border);
            let _ = DeleteObject(self.pen_border_glow);
            let _ = DeleteObject(self.pen_accent);
            let _ = DeleteObject(self.pen_accent_glow);
            let _ = DeleteObject(self.pen_close);
            let _ = DeleteObject(self.pen_success);
            let _ = DeleteObject(self.pen_window);
            let _ = DeleteObject(self.pen_danger);
        }
    }
}

// ============ DPI ============

static DPI: AtomicI32 = AtomicI32::new(0);

fn get_dpi() -> i32 {
    let v = DPI.load(Ordering::Relaxed);
    if v != 0 { return v; }
    let v = unsafe {
        let hdc = GetDC(None);
        let d = GetDeviceCaps(hdc, LOGPIXELSY);
        let _ = ReleaseDC(None, hdc);
        d
    };
    DPI.store(v, Ordering::Relaxed);
    v
}

fn s(v: i32) -> i32 { v * get_dpi() / 96 }

// ============ Utilities ============

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().collect() }

fn get_x_lparam(lp: LPARAM) -> i32 { (lp.0 as u32 & 0xFFFF) as i16 as i32 }
fn get_y_lparam(lp: LPARAM) -> i32 { ((lp.0 as u32 >> 16) & 0xFFFF) as i16 as i32 }

fn make_rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

fn window_inner_rect() -> RECT {
    make_rect(s(SHADOW_SIZE), s(SHADOW_SIZE),
        s(W_WIDTH) - s(SHADOW_SIZE), s(W_HEIGHT) - s(SHADOW_SIZE))
}

fn option_rect(idx: i32) -> RECT {
    let y = s(OPTION_START_Y) + (s(OPTION_H) + s(OPTION_GAP)) * idx;
    make_rect(s(PADDING), y, s(W_WIDTH) - s(PADDING), y + s(OPTION_H))
}

fn left_button_rect() -> RECT {
    make_rect(s(PADDING), s(BUTTONS_Y), s(PADDING) + s(BUTTON_W), s(BUTTONS_Y) + s(BUTTON_H))
}

fn right_button_rect() -> RECT {
    make_rect(s(W_WIDTH) - s(PADDING) - s(BUTTON_W), s(BUTTONS_Y), s(W_WIDTH) - s(PADDING), s(BUTTONS_Y) + s(BUTTON_H))
}

fn close_button_rect() -> RECT {
    make_rect(
        s(W_WIDTH) - s(CLOSE_BTN_SIZE) - s(CLOSE_BTN_OFFSET) - s(SHADOW_SIZE),
        s(CLOSE_BTN_OFFSET) + s(SHADOW_SIZE),
        s(W_WIDTH) - s(CLOSE_BTN_OFFSET) - s(SHADOW_SIZE),
        s(CLOSE_BTN_SIZE) + s(CLOSE_BTN_OFFSET) + s(SHADOW_SIZE),
    )
}

fn point_in_rect(x: i32, y: i32, r: &RECT) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn hit_test_option(x: i32, y: i32) -> i32 {
    for i in 0..4 { if point_in_rect(x, y, &option_rect(i)) { return i; } }
    -1
}

fn get_data_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let p1 = PathBuf::from(&appdata).join("com.phonon.app");
    if p1.exists() { return Some(p1); }
    let p2 = PathBuf::from(&appdata).join("Phonon");
    if p2.exists() { return Some(p2); }
    Some(p1)
}

// ============ Text data ============

const OPTION_TITLES: [&str; 4] = [
    "保留所有数据",
    "仅保留歌单曲库",
    "删除所有数据（保留外置插件）",
    "删除所有数据及外置插件",
];

const OPTION_DESCS: [&str; 4] = [
    "仅卸载程序，配置与曲库完整保留",
    "保留曲库索引、评分，删除设置和插件",
    "删除设置、曲库、会话，保留外置插件",
    "彻底清除所有数据，外置插件也将删除",
];

const PROGRESS_STEPS: [&str; 4] = [
    "正在关闭 Phonon...",
    "正在删除程序文件...",
    "正在清理数据...",
    "正在清理注册表...",
];

// ============ Entry Point ============

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let hinstance = unsafe { GetModuleHandleW(None) }.expect("GetModuleHandleW");

    let wcex = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap(),
        hbrBackground: unsafe { HBRUSH(GetStockObject(NULL_BRUSH).0) },
        lpszClassName: w!("PhononUninstaller"),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassExW(&wcex) };
    assert!(atom != 0, "RegisterClassExW failed");

    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let x = (sw - s(W_WIDTH)) / 2;
    let y = (sh - s(W_HEIGHT)) / 2;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_LAYERED,
            w!("PhononUninstaller"),
            w!("Phonon 卸载程序"),
            WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
            x, y, s(W_WIDTH), s(W_HEIGHT),
            None, None, hinstance, None,
        )
    }.expect("CreateWindowExW");

    unsafe {
        // Set layered window with transparency for shadow effect
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0x00000000), 255, LWA_COLORKEY);
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let _ = UpdateWindow(hwnd);
        let _ = SetTimer(hwnd, TIMER_ANIMATE, 33, None);
    }

    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !ret.as_bool() { break; }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ============ Window Procedure ============

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let state = Box::new(State::new());
            let ptr = Box::into_raw(state) as isize;
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);
            LRESULT(0)
        }
        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = Box::from_raw(ptr as *mut State);
                state.destroy();
                if state.uninstalled { self_delete(); }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = &mut *(ptr as *mut State);
                on_paint(hwnd, state);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = &mut *(ptr as *mut State);
                on_lbuttondown(hwnd, state, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = &mut *(ptr as *mut State);
                on_mousemove(hwnd, state, lparam);
            }
            LRESULT(0)
        }
        WM_PROGRESS => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = &mut *(ptr as *mut State);
                state.progress_step = wparam.0 as i32;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_DONE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let state = &mut *(ptr as *mut State);
                state.phase = Phase::Done;
                state.uninstalled = true;
                let _ = SetTimer(hwnd, TIMER_AUTO_CLOSE, 2000, None);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_AUTO_CLOSE {
                let _ = KillTimer(hwnd, TIMER_AUTO_CLOSE);
                let _ = DestroyWindow(hwnd);
            } else if wparam.0 == TIMER_ANIMATE {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    let state = &mut *(ptr as *mut State);
                    state.anim_tick = state.anim_tick.wrapping_add(1);
                    if state.phase == Phase::Uninstalling || state.phase == Phase::Done {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ============ Paint ============

fn on_paint(hwnd: HWND, state: &mut State) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut rc: RECT = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;

        let mem_dc = CreateCompatibleDC(hdc);
        let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem_dc, mem_bmp);

        // Clear with transparent black for layered window
        let clear_br = CreateSolidBrush(COLORREF(0x00000000));
        FillRect(mem_dc, &rc, clear_br);
        let _ = DeleteObject(clear_br);

        // Draw shadow and window frame
        draw_window_with_shadow(mem_dc, state);

        match state.phase {
            Phase::Select => paint_select(mem_dc, state),
            Phase::Confirm => paint_confirm(mem_dc, state),
            Phase::Uninstalling => paint_progress(mem_dc, state),
            Phase::Done => paint_done(mem_dc, state),
        }

        let _ = BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
        let _ = SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(mem_bmp);
        let _ = DeleteDC(mem_dc);

        let _ = EndPaint(hwnd, &ps);
    }
}

// Draw window with shadow effect
fn draw_window_with_shadow(hdc: HDC, state: &State) {
    let inner = window_inner_rect();
    let radius = s(WINDOW_RADIUS);
    let shadow = s(SHADOW_SIZE);

    unsafe {
        // Draw shadow layers (multi-pass for soft shadow effect)
        for i in (0i32..3).rev() {
            let alpha = 20 + i as u32 * 15;
            let expand = shadow - i * 1;
            let shadow_rect = make_rect(
                inner.left - expand,
                inner.top - expand + i * 2,
                inner.right + expand,
                inner.bottom + expand + i * 2,
            );
            let shadow_color = make_shadow_color(alpha);
            let pen = CreatePen(PS_SOLID, 1, COLORREF(shadow_color));
            let br = CreateSolidBrush(COLORREF(shadow_color));
            let old_pen = SelectObject(hdc, pen);
            let old_br = SelectObject(hdc, br);
            let sr = radius + expand;
            let _ = RoundRect(hdc, shadow_rect.left, shadow_rect.top,
                shadow_rect.right, shadow_rect.bottom, sr, sr);
            let _ = SelectObject(hdc, old_pen);
            let _ = SelectObject(hdc, old_br);
            let _ = DeleteObject(pen);
            let _ = DeleteObject(br);
        }

        // Main window background with rounded corners
        let hrgn = CreateRoundRectRgn(inner.left, inner.top, inner.right, inner.bottom, radius, radius);
        let _ = SelectClipRgn(hdc, hrgn);

        // Base background
        let br = CreateSolidBrush(COLORREF(CLR_BG));
        let mut fill_rc = RECT::default();
        let _ = GetClipBox(hdc, &mut fill_rc);
        FillRect(hdc, &fill_rc, br);
        let _ = DeleteObject(br);

        let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
        let _ = DeleteObject(hrgn);

        // Window border
        draw_rounded_border(hdc, &inner, radius, state.pen_window);
    }
}

fn make_shadow_color(alpha: u32) -> u32 {
    // COLORREF format: 0x00BBGGRR
    // We use dark colors with varying intensity for shadow layers
    let v = (alpha * 30 / 255).min(30);
    0x00FFFFFF & (v | (v << 8) | (v << 16))
}

fn draw_gradient_v(hdc: HDC, rect: &RECT, top: u32, bottom: u32) {
    unsafe {
        let verts = [
            trivertex(rect.left, rect.top, top),
            trivertex(rect.right, rect.bottom, bottom),
        ];
        let mesh = [GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 }];
        let _ = GradientFill(hdc, &verts, mesh.as_ptr() as *const std::ffi::c_void,
            mesh.len() as u32, GRADIENT_FILL_RECT_V);
    }
}

fn draw_gradient_h(hdc: HDC, rect: &RECT, left: u32, right: u32) {
    unsafe {
        let verts = [
            trivertex(rect.left, rect.top, left),
            trivertex(rect.right, rect.bottom, right),
        ];
        let mesh = [GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 }];
        let _ = GradientFill(hdc, &verts, mesh.as_ptr() as *const std::ffi::c_void,
            mesh.len() as u32, GRADIENT_FILL_RECT_H);
    }
}

fn draw_text_left(hdc: HDC, text: &str, x: i32, y: i32, color: u32, font: HFONT) {
    unsafe {
        let old = SelectObject(hdc, font);
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, COLORREF(color));
        let w = wide(text);
        let _ = TextOutW(hdc, x, y, &w);
        let _ = SelectObject(hdc, old);
    }
}

fn draw_text_center(hdc: HDC, text: &str, cx: i32, y: i32, color: u32, font: HFONT) {
    unsafe {
        let old = SelectObject(hdc, font);
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, COLORREF(color));
        let w = wide(text);
        let mut size = SIZE::default();
        let _ = GetTextExtentPointW(hdc, &w, &mut size);
        let _ = TextOutW(hdc, cx - size.cx / 2, y, &w);
        let _ = SelectObject(hdc, old);
    }
}

fn draw_rounded_gradient(hdc: HDC, rect: &RECT, radius: i32, c1: u32, c2: u32, horizontal: bool) {
    unsafe {
        let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        let _ = SelectClipRgn(hdc, hrgn);

        if horizontal {
            draw_gradient_h(hdc, rect, c1, c2);
        } else {
            draw_gradient_v(hdc, rect, c1, c2);
        }

        let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
        let _ = DeleteObject(hrgn);
    }
}

fn draw_rounded_fill(hdc: HDC, rect: &RECT, radius: i32, brush: HBRUSH) {
    unsafe {
        let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        let _ = SelectClipRgn(hdc, hrgn);
        let mut rc = RECT::default();
        let _ = GetClipBox(hdc, &mut rc);
        FillRect(hdc, &rc, brush);
        let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
        let _ = DeleteObject(hrgn);
    }
}

fn draw_rounded_border(hdc: HDC, rect: &RECT, radius: i32, pen: HPEN) {
    unsafe {
        let old = SelectObject(hdc, pen);
        let null_brush = GetStockObject(NULL_BRUSH);
        let old_br = SelectObject(hdc, null_brush);
        let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        let _ = SelectObject(hdc, old);
        let _ = SelectObject(hdc, old_br);
    }
}

// ============ Icons (redesigned with cleaner, more modern look) ============

fn draw_option_icon(hdc: HDC, cx: i32, cy: i32, kind: IconKind, selected: bool, state: &State) {
    let size = s(OPTION_ICON_SIZE);
    let half = size / 2;
    let r = s(9); // rounded square radius
    let rect = make_rect(cx - half, cy - half, cx + half, cy + half);

    unsafe {
        // Background - gradient for selected, solid for unselected
        if selected {
            // Gradient background when selected
            draw_rounded_gradient(hdc, &rect, r, CLR_ACCENT, CLR_ACCENT2, true);
        } else {
            // Subtle background
            draw_rounded_fill(hdc, &rect, r, state.brush_icon_bg);
            draw_rounded_border(hdc, &rect, r, state.pen_border);
        }

        // Icon glyph color
        let glyph_color = if selected { CLR_WHITE } else { CLR_ACCENT };
        let pen_width = if selected { 2 } else { 2 };
        let pen = CreatePen(PS_SOLID, pen_width, COLORREF(glyph_color));
        let old = SelectObject(hdc, pen);

        match kind {
            IconKind::Shield => {
                // Cleaner shield with checkmark
                let top_y = cy - half + s(7);
                let bottom_y = cy + half - s(6);
                let left_x = cx - half + s(8);
                let right_x = cx + half - s(8);
                let mid_y = cy + s(2);

                // Shield outline
                let _ = MoveToEx(hdc, cx, top_y, None);
                let _ = LineTo(hdc, right_x, top_y + s(3));
                let _ = LineTo(hdc, right_x, mid_y);
                let _ = LineTo(hdc, cx + s(5), bottom_y - s(1));
                let _ = LineTo(hdc, cx, bottom_y);
                let _ = LineTo(hdc, cx - s(5), bottom_y - s(1));
                let _ = LineTo(hdc, left_x, mid_y);
                let _ = LineTo(hdc, left_x, top_y + s(3));
                let _ = LineTo(hdc, cx, top_y);

                // Checkmark inside shield
                let check_pen = CreatePen(PS_SOLID, 2, COLORREF(glyph_color));
                let old_cp = SelectObject(hdc, check_pen);
                let p1x = cx - s(4);
                let p1y = cy - s(1);
                let p2x = cx - s(1);
                let p2y = cy + s(3);
                let p3x = cx + s(5);
                let p3y = cy - s(3);
                let _ = MoveToEx(hdc, p1x, p1y, None);
                let _ = LineTo(hdc, p2x, p2y);
                let _ = MoveToEx(hdc, p2x, p2y, None);
                let _ = LineTo(hdc, p3x, p3y);
                let _ = SelectObject(hdc, old_cp);
                let _ = DeleteObject(check_pen);
            }
            IconKind::Music => {
                // Cleaner music note - eighth note style
                let stem_x = cx + s(3);
                let note_top = cy - half + s(6);
                let note_bottom = cy + half - s(5);

                // Stem
                let _ = MoveToEx(hdc, stem_x, note_top + s(2), None);
                let _ = LineTo(hdc, stem_x, note_bottom - s(1));

                // Flag (beamed)
                let _ = MoveToEx(hdc, stem_x, note_top + s(2), None);
                let _ = LineTo(hdc, stem_x + s(5), note_top + s(5));
                let _ = LineTo(hdc, stem_x, note_top + s(8));

                // Note head (filled ellipse)
                let head_br = CreateSolidBrush(COLORREF(glyph_color));
                let old_hbr = SelectObject(hdc, head_br);
                let pen_null = GetStockObject(NULL_PEN);
                let old_pen = SelectObject(hdc, pen_null);
                let head_rx = s(5);
                let head_ry = s(4);
                let head_cx = cx - s(2);
                let head_cy = note_bottom - s(1);
                let _ = Ellipse(hdc, head_cx - head_rx, head_cy - head_ry,
                    head_cx + head_rx + 1, head_cy + head_ry + 1);
                let _ = SelectObject(hdc, old_hbr);
                let _ = SelectObject(hdc, old_pen);
                let _ = DeleteObject(head_br);
            }
            IconKind::Trash => {
                // Cleaner trash can icon
                let top_y = cy - half + s(6);
                let bottom_y = cy + half - s(6);
                let left_x = cx - half + s(8);
                let right_x = cx + half - s(8);
                let lid_y = top_y + s(4);

                // Lid (horizontal bar - thicker)
                let lid_pen = CreatePen(PS_SOLID, 2, COLORREF(glyph_color));
                let old_lid = SelectObject(hdc, lid_pen);
                let _ = MoveToEx(hdc, left_x - s(2), lid_y, None);
                let _ = LineTo(hdc, right_x + s(2), lid_y);
                let _ = SelectObject(hdc, old_lid);
                let _ = DeleteObject(lid_pen);

                // Lid handle (small rectangle on top)
                let handle_l = cx - s(3);
                let handle_r = cx + s(3);
                let _handle_h = s(3);
                let _ = MoveToEx(hdc, handle_l, top_y, None);
                let _ = LineTo(hdc, handle_r, top_y);
                let _ = MoveToEx(hdc, handle_l, top_y, None);
                let _ = LineTo(hdc, handle_l, lid_y);
                let _ = MoveToEx(hdc, handle_r, top_y, None);
                let _ = LineTo(hdc, handle_r, lid_y);

                // Can body sides
                let _ = MoveToEx(hdc, left_x + s(1), lid_y, None);
                let _ = LineTo(hdc, left_x + s(2), bottom_y);
                let _ = MoveToEx(hdc, right_x - s(1), lid_y, None);
                let _ = LineTo(hdc, right_x - s(2), bottom_y);

                // Bottom
                let _ = MoveToEx(hdc, left_x + s(2), bottom_y, None);
                let _ = LineTo(hdc, right_x - s(2), bottom_y);

                // Inner lines (representing trash)
                let inner_pen = CreatePen(PS_SOLID, 1, COLORREF(glyph_color));
                let old_inner = SelectObject(hdc, inner_pen);
                let _ = MoveToEx(hdc, cx - s(3), lid_y + s(5), None);
                let _ = LineTo(hdc, cx - s(3), bottom_y - s(3));
                let _ = MoveToEx(hdc, cx + s(3), lid_y + s(5), None);
                let _ = LineTo(hdc, cx + s(3), bottom_y - s(3));
                let _ = SelectObject(hdc, old_inner);
                let _ = DeleteObject(inner_pen);
            }
            IconKind::Eraser => {
                // Explosion/bomb icon - more dynamic
                let center_x = cx;
                let center_y = cy + s(1);
                let radius = s(6);

                // Main circle (bomb body)
                let bomb_br = CreateSolidBrush(COLORREF(glyph_color));
                let old_bbr = SelectObject(hdc, bomb_br);
                let pen_null = GetStockObject(NULL_PEN);
                let old_pen = SelectObject(hdc, pen_null);
                let _ = Ellipse(hdc, center_x - radius, center_y - radius,
                    center_x + radius + 1, center_y + radius + 1);
                let _ = SelectObject(hdc, old_bbr);
                let _ = SelectObject(hdc, old_pen);
                let _ = DeleteObject(bomb_br);

                // Fuse (curved-ish)
                let fuse_pen = CreatePen(PS_SOLID, 2, COLORREF(glyph_color));
                let old_fuse = SelectObject(hdc, fuse_pen);
                let _ = MoveToEx(hdc, center_x + s(3), center_y - s(5), None);
                let _ = LineTo(hdc, center_x + s(5), center_y - s(8));
                let _ = LineTo(hdc, center_x + s(7), center_y - s(6));
                let _ = SelectObject(hdc, old_fuse);
                let _ = DeleteObject(fuse_pen);

                // Spark (small star-like)
                let spark_cx = center_x + s(7);
                let spark_cy = center_y - s(9);
                let spark_br = CreateSolidBrush(COLORREF(glyph_color));
                let old_sbr = SelectObject(hdc, spark_br);
                let spen_null = GetStockObject(NULL_PEN);
                let old_spen = SelectObject(hdc, spen_null);
                let sr = s(2);
                let _ = Ellipse(hdc, spark_cx - sr, spark_cy - sr,
                    spark_cx + sr + 1, spark_cy + sr + 1);
                let _ = SelectObject(hdc, old_sbr);
                let _ = SelectObject(hdc, old_spen);
                let _ = DeleteObject(spark_br);

                // X inside (white if selected, accent if not)
                let x_color = if selected { CLR_WHITE } else { CLR_ICON_BG };
                let x_pen = CreatePen(PS_SOLID, 2, COLORREF(x_color));
                let old_x = SelectObject(hdc, x_pen);
                let _ = MoveToEx(hdc, center_x - s(3), center_y - s(3), None);
                let _ = LineTo(hdc, center_x + s(3), center_y + s(3));
                let _ = MoveToEx(hdc, center_x + s(3), center_y - s(3), None);
                let _ = LineTo(hdc, center_x - s(3), center_y + s(3));
                let _ = SelectObject(hdc, old_x);
                let _ = DeleteObject(x_pen);
            }
        }

        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(pen);
    }
}

// ============ Header / Branding ============

fn draw_header(hdc: HDC, state: &State) {
    let inner = window_inner_rect();
    let header_rect = make_rect(inner.left, inner.top,
        inner.right, inner.top + s(HEADER_H));

    // Clip to window rounded shape at top
    let radius = s(WINDOW_RADIUS);
    unsafe {
        let hrgn = CreateRoundRectRgn(
            inner.left, inner.top,
            inner.right, inner.top + s(HEADER_H) + s(WINDOW_RADIUS),
            radius, radius,
        );
        let _ = SelectClipRgn(hdc, hrgn);
        draw_gradient_v(hdc, &header_rect, CLR_HEADER_GRAD1, CLR_HEADER_GRAD2);

        // Subtle radial glow at center bottom
        let cx = s(W_WIDTH) / 2;
        let glow_cy = inner.top + s(HEADER_H) - s(10);
        for i in (0i32..5).rev() {
            let rx = s(60) + i * s(12);
            let ry = s(20) + i * s(5);
            let alpha = 10 + i as u32 * 6;
            let glow_color = make_glow_color(CLR_ACCENT, alpha);
            let glow_br = CreateSolidBrush(COLORREF(glow_color));
            let old = SelectObject(hdc, glow_br);
            let pen_null = GetStockObject(NULL_PEN);
            let _ = SelectObject(hdc, pen_null);
            let _ = Ellipse(hdc, cx - rx, glow_cy - ry, cx + rx + 1, glow_cy + ry + 1);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(glow_br);
        }

        let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
        let _ = DeleteObject(hrgn);
    }

    // Sound wave decoration with animation
    unsafe {
        let cx = s(W_WIDTH) / 2;
        let base_y = inner.top + s(HEADER_H) - s(20);

        for i in 0..7 {
            let offset = (i - 3) * s(10);
            let base_h = match i {
                0 => s(4),
                1 => s(8),
                2 => s(14),
                3 => s(22),
                4 => s(14),
                5 => s(8),
                _ => s(4),
            };

            // Animated height variation
            let t = state.anim_tick as f64;
            let wave = (t * 0.06 + i as f64 * 0.6).sin() * 0.35 + 1.0;
            let height_var = (base_h as f64 * wave) as i32;
            let height_var = height_var.max(s(2)).min(base_h + s(5));

            // Glow behind center wave
            if i == 3 {
                let glow_br = CreateSolidBrush(COLORREF(make_glow_color(CLR_ACCENT, 25)));
                let old = SelectObject(hdc, glow_br);
                let pen_null = GetStockObject(NULL_PEN);
                let _ = SelectObject(hdc, pen_null);
                let gr = s(6);
                let _ = Ellipse(hdc, cx + offset - gr, base_y - height_var - gr,
                    cx + offset + gr + 1, base_y + height_var + gr + 1);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(glow_br);
            }

            let color = if i == 3 { CLR_ACCENT } else { CLR_ACCENT_SOFT };
            let pen = CreatePen(PS_SOLID, 2, COLORREF(color));
            let old = SelectObject(hdc, pen);
            let _ = MoveToEx(hdc, cx + offset, base_y - height_var, None);
            let _ = LineTo(hdc, cx + offset, base_y + height_var);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(pen);
        }

        // Accent dot at bottom center with glow
        let dot_br = CreateSolidBrush(COLORREF(CLR_ACCENT));
        let old = SelectObject(hdc, dot_br);
        let pen_null = GetStockObject(NULL_PEN);
        let _ = SelectObject(hdc, pen_null);
        let dr = s(3);
        let tallest_h = s(22);
        // Glow
        let glow_br = CreateSolidBrush(COLORREF(make_glow_color(CLR_ACCENT, 35)));
        let _ = SelectObject(hdc, glow_br);
        let gdr = s(6);
        let _ = Ellipse(hdc, cx - gdr, base_y + tallest_h + s(2) - gdr,
            cx + gdr + 1, base_y + tallest_h + s(2) + gdr + 1);
        let _ = SelectObject(hdc, dot_br);
        let _ = DeleteObject(glow_br);
        let _ = Ellipse(hdc, cx - dr, base_y + tallest_h + s(2) - dr,
            cx + dr + 1, base_y + tallest_h + s(2) + dr + 1);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(dot_br);
    }

    // Brand text with subtle glow
    let title_y = s(SHADOW_SIZE) + s(22);
    unsafe {
        // Glow layer
        let glow_color = make_glow_color(CLR_ACCENT, 20);
        let old_font = SelectObject(hdc, state.font_big);
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, COLORREF(glow_color));
        let w = wide("PHONON");
        let mut size = SIZE::default();
        let _ = GetTextExtentPointW(hdc, &w, &mut size);
        let tx = s(W_WIDTH) / 2 - size.cx / 2;
        for dx in [-1, 0, 1] {
            for dy in [-1, 0, 1] {
                if dx == 0 && dy == 0 { continue; }
                let _ = TextOutW(hdc, tx + dx, title_y + dy, &w);
            }
        }
        let _ = SelectObject(hdc, old_font);
    }
    draw_text_center(hdc, "PHONON", s(W_WIDTH) / 2, title_y, CLR_TEXT_BRIGHT, state.font_big);
    draw_text_center(hdc, "高保真音频平台", s(W_WIDTH) / 2, s(SHADOW_SIZE) + s(56), CLR_TEXT_DIM, state.font_small);
}

fn draw_title_bar(hdc: HDC, state: &State) {
    let inner = window_inner_rect();
    draw_text_left(hdc, "Phonon 卸载程序", inner.left + s(14), inner.top + s(9), CLR_TEXT_DIM, state.font_small);
    draw_close_button(hdc, state);
}

fn draw_close_button(hdc: HDC, state: &State) {
    let r = close_button_rect();
    let inner = window_inner_rect();
    let _radius = s(WINDOW_RADIUS) - s(3);
    unsafe {
        if state.hover_close {
            let br = CreateSolidBrush(COLORREF(CLR_CLOSE_HOVER));
            // Clip to top-right corner of window
            let hrgn = CreateRoundRectRgn(inner.left, inner.top, inner.right, inner.bottom,
                s(WINDOW_RADIUS), s(WINDOW_RADIUS));
            let _ = SelectClipRgn(hdc, hrgn);
            FillRect(hdc, &r, br);
            let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
            let _ = DeleteObject(hrgn);
            let _ = DeleteObject(br);
        }
        let pen = if state.hover_close {
            CreatePen(PS_SOLID, 1, COLORREF(CLR_WHITE))
        } else {
            state.pen_close
        };
        let old = SelectObject(hdc, pen);
        let m = s(8);
        let cx = r.left + (r.right - r.left) / 2;
        let cy = r.top + (r.bottom - r.top) / 2;
        let _ = MoveToEx(hdc, cx - m, cy - m, None);
        let _ = LineTo(hdc, cx + m, cy + m);
        let _ = MoveToEx(hdc, cx + m, cy - m, None);
        let _ = LineTo(hdc, cx - m, cy + m);
        let _ = SelectObject(hdc, old);
        if state.hover_close { let _ = DeleteObject(pen); }
    }
}

// ============ Button ============

enum ButtonKind { Primary, Outline, Danger }

fn trivertex(x: i32, y: i32, color: u32) -> TRIVERTEX {
    TRIVERTEX {
        x, y,
        Red: ((color & 0xFF) as u16) << 8,
        Green: (((color >> 8) & 0xFF) as u16) << 8,
        Blue: (((color >> 16) & 0xFF) as u16) << 8,
        Alpha: 0,
    }
}

fn draw_button(hdc: HDC, rect: &RECT, text: &str, kind: ButtonKind, hovered: bool, state: &State) {
    let radius = s(BTN_RADIUS);
    unsafe {
        match kind {
            ButtonKind::Primary => {
                // Outer glow on hover
                if hovered {
                    for layer in 0i32..3 {
                        let expand = 3 - layer;
                        let alpha = 30 + layer as u32 * 25;
                        let glow_color = make_glow_color(CLR_ACCENT, alpha);
                        let glow_pen = CreatePen(PS_SOLID, 1, COLORREF(glow_color));
                        let glow_br = CreateSolidBrush(COLORREF(glow_color));
                        let old_pen = SelectObject(hdc, glow_pen);
                        let old_br = SelectObject(hdc, glow_br);
                        let gr = make_rect(rect.left - expand, rect.top - expand, rect.right + expand, rect.bottom + expand);
                        let _ = RoundRect(hdc, gr.left, gr.top, gr.right, gr.bottom, radius + expand, radius + expand);
                        let _ = SelectObject(hdc, old_pen);
                        let _ = SelectObject(hdc, old_br);
                        let _ = DeleteObject(glow_pen);
                        let _ = DeleteObject(glow_br);
                    }
                }
                let (c1, c2) = if hovered {
                    (lighten_color(CLR_ACCENT, 15), lighten_color(CLR_ACCENT2, 15))
                } else {
                    (CLR_ACCENT, CLR_ACCENT2)
                };
                draw_rounded_gradient(hdc, rect, radius, c1, c2, true);
                // Top highlight
                let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, radius, radius);
                let _ = SelectClipRgn(hdc, hrgn);
                let hl_pen = CreatePen(PS_SOLID, 1, COLORREF(0x00FFFFFF & lighten_color(CLR_ACCENT, 40)));
                let old = SelectObject(hdc, hl_pen);
                let _ = MoveToEx(hdc, rect.left + s(4), rect.top + 1, None);
                let _ = LineTo(hdc, rect.right - s(4), rect.top + 1);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(hl_pen);
                let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
                let _ = DeleteObject(hrgn);
            }
            ButtonKind::Outline => {
                if hovered {
                    draw_rounded_fill(hdc, rect, radius, state.brush_card_hover);
                    // Top highlight
                    let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, radius, radius);
                    let _ = SelectClipRgn(hdc, hrgn);
                    let hl_pen = CreatePen(PS_SOLID, 1, COLORREF(CLR_BORDER_GLOW));
                    let old = SelectObject(hdc, hl_pen);
                    let _ = MoveToEx(hdc, rect.left + s(4), rect.top + 1, None);
                    let _ = LineTo(hdc, rect.right - s(4), rect.top + 1);
                    let _ = SelectObject(hdc, old);
                    let _ = DeleteObject(hl_pen);
                    let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
                    let _ = DeleteObject(hrgn);
                }
                let border_pen = if hovered { state.pen_border_glow } else { state.pen_border };
                draw_rounded_border(hdc, rect, radius, border_pen);
            }
            ButtonKind::Danger => {
                if hovered {
                    // Outer glow
                    for layer in 0i32..3 {
                        let expand = 3 - layer;
                        let alpha = 30 + layer as u32 * 25;
                        let glow_color = make_glow_color(CLR_DANGER, alpha);
                        let glow_pen = CreatePen(PS_SOLID, 1, COLORREF(glow_color));
                        let glow_br = CreateSolidBrush(COLORREF(glow_color));
                        let old_pen = SelectObject(hdc, glow_pen);
                        let old_br = SelectObject(hdc, glow_br);
                        let gr = make_rect(rect.left - expand, rect.top - expand, rect.right + expand, rect.bottom + expand);
                        let _ = RoundRect(hdc, gr.left, gr.top, gr.right, gr.bottom, radius + expand, radius + expand);
                        let _ = SelectObject(hdc, old_pen);
                        let _ = SelectObject(hdc, old_br);
                        let _ = DeleteObject(glow_pen);
                        let _ = DeleteObject(glow_br);
                    }
                    let br = CreateSolidBrush(COLORREF(CLR_DANGER_HOVER));
                    draw_rounded_fill(hdc, rect, radius, br);
                    let _ = DeleteObject(br);
                    // Top highlight
                    let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, radius, radius);
                    let _ = SelectClipRgn(hdc, hrgn);
                    let hl_pen = CreatePen(PS_SOLID, 1, COLORREF(lighten_color(CLR_DANGER_HOVER, 30)));
                    let old = SelectObject(hdc, hl_pen);
                    let _ = MoveToEx(hdc, rect.left + s(4), rect.top + 1, None);
                    let _ = LineTo(hdc, rect.right - s(4), rect.top + 1);
                    let _ = SelectObject(hdc, old);
                    let _ = DeleteObject(hl_pen);
                    let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
                    let _ = DeleteObject(hrgn);
                } else {
                    let pen = CreatePen(PS_SOLID, 1, COLORREF(CLR_DANGER));
                    draw_rounded_border(hdc, rect, radius, pen);
                    let _ = DeleteObject(pen);
                }
            }
        }
        let color = match kind {
            ButtonKind::Primary => CLR_WHITE,
            ButtonKind::Outline if hovered => CLR_ACCENT,
            ButtonKind::Outline => CLR_TEXT_BRIGHT,
            ButtonKind::Danger if hovered => CLR_WHITE,
            ButtonKind::Danger => CLR_DANGER,
        };
        let cx = (rect.left + rect.right) / 2;
        let cy = (rect.top + rect.bottom) / 2;
        let font = match kind {
            ButtonKind::Primary => state.font_body_bold,
            _ => state.font_body,
        };
        let old_font = SelectObject(hdc, font);
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, COLORREF(color));
        let w = wide(text);
        let mut size = SIZE::default();
        let _ = GetTextExtentPointW(hdc, &w, &mut size);
        let _ = TextOutW(hdc, cx - size.cx / 2, cy - size.cy / 2, &w);
        let _ = SelectObject(hdc, old_font);
    }
}

fn lighten_color(color: u32, percent: u32) -> u32 {
    let r = (color & 0xFF) as u32;
    let g = ((color >> 8) & 0xFF) as u32;
    let b = ((color >> 16) & 0xFF) as u32;
    let r = std::cmp::min(255, r + r * percent / 100);
    let g = std::cmp::min(255, g + g * percent / 100);
    let b = std::cmp::min(255, b + b * percent / 100);
    0x00FFFFFF & (r | (g << 8) | (b << 16))
}

fn make_glow_color(base: u32, alpha: u32) -> u32 {
    let bg = CLR_BG;
    let br = bg & 0xFF;
    let bg_g = (bg >> 8) & 0xFF;
    let bb = (bg >> 16) & 0xFF;
    let fr = base & 0xFF;
    let fg = (base >> 8) & 0xFF;
    let fb = (base >> 16) & 0xFF;
    let a = alpha.min(255);
    let r = (fr * a + br * (255 - a)) / 255;
    let g = (fg * a + bg_g * (255 - a)) / 255;
    let b = (fb * a + bb * (255 - a)) / 255;
    0x00FFFFFF & (r | (g << 8) | (b << 16))
}

// ============ Option Card ============

fn draw_option_card(hdc: HDC, idx: i32, state: &State) {
    let r = option_rect(idx);
    let hovered = state.hover_option == idx;
    let selected = state.selected == idx;
    let radius = s(CARD_RADIUS);

    let icon_kind = match idx {
        0 => IconKind::Shield,
        1 => IconKind::Music,
        2 => IconKind::Trash,
        _ => IconKind::Eraser,
    };

    unsafe {
        // Outer glow for selected state
        if selected {
            for layer in 0i32..3 {
                let expand = 2 - layer;
                let alpha = 40 + layer as u32 * 30;
                let glow_color = make_glow_color(CLR_ACCENT, alpha);
                let glow_pen = CreatePen(PS_SOLID, 1, COLORREF(glow_color));
                let glow_br = CreateSolidBrush(COLORREF(CLR_CARD_SELECTED));
                let old_pen = SelectObject(hdc, glow_pen);
                let old_br = SelectObject(hdc, glow_br);
                let gr = make_rect(r.left - expand, r.top - expand, r.right + expand, r.bottom + expand);
                let _ = RoundRect(hdc, gr.left, gr.top, gr.right, gr.bottom, radius + expand, radius + expand);
                let _ = SelectObject(hdc, old_pen);
                let _ = SelectObject(hdc, old_br);
                let _ = DeleteObject(glow_pen);
                let _ = DeleteObject(glow_br);
            }
        }

        // Background
        let bg_brush = if selected {
            state.brush_card_selected
        } else if hovered {
            state.brush_card_hover
        } else {
            state.brush_card
        };
        draw_rounded_fill(hdc, &r, radius, bg_brush);

        // Top highlight line for depth
        if selected || hovered {
            let highlight_color = if selected { CLR_ACCENT_GLOW } else { CLR_BORDER_GLOW };
            let hl_pen = CreatePen(PS_SOLID, 1, COLORREF(highlight_color));
            let old = SelectObject(hdc, hl_pen);
            let hrgn = CreateRoundRectRgn(r.left, r.top, r.right, r.bottom, radius, radius);
            let _ = SelectClipRgn(hdc, hrgn);
            let _ = MoveToEx(hdc, r.left + s(2), r.top + 1, None);
            let _ = LineTo(hdc, r.right - s(2), r.top + 1);
            let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
            let _ = DeleteObject(hrgn);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(hl_pen);
        }

        // Border
        if selected {
            // Gradient-like border using two passes
            draw_rounded_border(hdc, &r, radius, state.pen_accent);
            let inner_r = make_rect(r.left + 1, r.top + 1, r.right - 1, r.bottom - 1);
            draw_rounded_border(hdc, &inner_r, radius - 1, state.pen_accent_glow);
        } else if hovered {
            draw_rounded_border(hdc, &r, radius, state.pen_border_glow);
        } else {
            draw_rounded_border(hdc, &r, radius, state.pen_border);
        }

        // Left accent bar when selected - gradient pill style
        if selected {
            let bar_w = s(4);
            let bar_h = s(28);
            let bar_x = r.left + s(4);
            let bar_y = r.top + (r.bottom - r.top - bar_h) / 2;
            let bar_rect = make_rect(bar_x, bar_y, bar_x + bar_w * 2, bar_y + bar_h);
            draw_rounded_gradient(hdc, &bar_rect, bar_w, CLR_ACCENT, CLR_ACCENT2, false);
        }
    }

    // Icon
    let icon_cx = s(OPTION_ICON_X) + s(OPTION_ICON_SIZE) / 2;
    let icon_cy = r.top + s(OPTION_H) / 2;
    draw_option_icon(hdc, icon_cx, icon_cy, icon_kind, selected, state);

    // Text
    let title_color = if selected { CLR_TEXT_BRIGHT } else { CLR_TEXT };
    let title_font = if selected { state.font_body_bold } else { state.font_body };
    draw_text_left(hdc, OPTION_TITLES[idx as usize],
        s(OPTION_TEXT_X), r.top + s(14), title_color, title_font);
    draw_text_left(hdc, OPTION_DESCS[idx as usize],
        s(OPTION_TEXT_X), r.top + s(32), CLR_TEXT_DIM, state.font_small);
}

// ============ Phase: Select ============

fn paint_select(hdc: HDC, state: &State) {
    draw_header(hdc, state);
    draw_title_bar(hdc, state);

    draw_text_left(hdc, "选择卸载方式", s(PADDING), s(SHADOW_SIZE) + s(116), CLR_TEXT_DIM, state.font_micro);

    for i in 0..4 {
        draw_option_card(hdc, i, state);
    }

    let lb = left_button_rect();
    let rb = right_button_rect();
    draw_button(hdc, &lb, "取消", ButtonKind::Outline, state.hover_btn == 0, state);
    draw_button(hdc, &rb, "卸载", ButtonKind::Primary, state.hover_btn == 1, state);
}

// ============ Phase: Confirm ============

fn paint_confirm(hdc: HDC, state: &State) {
    draw_header(hdc, state);
    draw_title_bar(hdc, state);

    let cx = s(W_WIDTH) / 2;
    let icon_cy = s(SHADOW_SIZE) + s(160);
    let icon_r = s(32);

    unsafe {
        // Outer glow
        for layer in 0i32..3 {
            let expand = 4 - layer;
            let alpha = 25 + layer as u32 * 20;
            let glow_color = make_glow_color(CLR_DANGER, alpha);
            let glow_br = CreateSolidBrush(COLORREF(glow_color));
            let old = SelectObject(hdc, glow_br);
            let pen_null = GetStockObject(NULL_PEN);
            let _ = SelectObject(hdc, pen_null);
            let gr = icon_r + expand;
            let _ = Ellipse(hdc, cx - gr, icon_cy - gr, cx + gr + 1, icon_cy + gr + 1);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(glow_br);
        }

        // Warning circle with gradient
        let rect = make_rect(cx - icon_r, icon_cy - icon_r, cx + icon_r + 1, icon_cy + icon_r + 1);
        let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, icon_r, icon_r);
        let _ = SelectClipRgn(hdc, hrgn);
        draw_gradient_v(hdc, &rect, CLR_DANGER, CLR_DANGER_SOFT);
        let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
        let _ = DeleteObject(hrgn);

        // Exclamation mark
        let ex_br = CreateSolidBrush(COLORREF(CLR_WHITE));
        let old = SelectObject(hdc, ex_br);
        let pen_null = GetStockObject(NULL_PEN);
        let _ = SelectObject(hdc, pen_null);
        let bar_w = s(3);
        let bar_top = icon_cy - s(14);
        let bar_bottom = icon_cy + s(2);
        let _ = RoundRect(hdc, cx - bar_w, bar_top, cx + bar_w + 1, bar_bottom, bar_w, bar_w);
        let dot_r = s(3);
        let dot_y = icon_cy + s(13);
        let _ = Ellipse(hdc, cx - dot_r, dot_y - dot_r, cx + dot_r + 1, dot_y + dot_r + 1);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(ex_br);
    }

    draw_text_center(hdc, "确认要卸载 Phonon 吗？", cx, s(SHADOW_SIZE) + s(208), CLR_TEXT_BRIGHT, state.font_title);
    draw_text_center(hdc, OPTION_DESCS[state.selected as usize], cx, s(SHADOW_SIZE) + s(236), CLR_ACCENT, state.font_body);
    draw_text_center(hdc, "此操作不可撤销", cx, s(SHADOW_SIZE) + s(266), CLR_DANGER, state.font_small);

    let lb = left_button_rect();
    let rb = right_button_rect();
    draw_button(hdc, &lb, "取消", ButtonKind::Outline, state.hover_btn == 0, state);
    draw_button(hdc, &rb, "确认卸载", ButtonKind::Danger, state.hover_btn == 1, state);
}

// ============ Phase: Progress ============

fn paint_progress(hdc: HDC, state: &State) {
    draw_header(hdc, state);
    draw_title_bar(hdc, state);

    let cx = s(W_WIDTH) / 2;
    let step = state.progress_step.clamp(0, 3) as usize;
    let spinner_cy = s(SHADOW_SIZE) + s(180);
    let spinner_r = s(34);
    let t = state.anim_tick;

    unsafe {
        // Outer glow pulse
        let pulse_phase = (t as f64 * 0.05).sin() * 0.3 + 0.7;
        let glow_r = spinner_r + s(8);
        for layer in 0i32..3 {
            let expand = s(6) - layer * s(2);
            let alpha = ((15 + layer as u32 * 12) as f64 * pulse_phase) as u32;
            let glow_color = make_glow_color(CLR_ACCENT, alpha);
            let glow_br = CreateSolidBrush(COLORREF(glow_color));
            let old = SelectObject(hdc, glow_br);
            let pen_null = GetStockObject(NULL_PEN);
            let _ = SelectObject(hdc, pen_null);
            let gr = glow_r + expand;
            let _ = Ellipse(hdc, cx - gr, spinner_cy - gr, cx + gr + 1, spinner_cy + gr + 1);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(glow_br);
        }

        // Background ring
        let pen_bg = CreatePen(PS_SOLID, 3, COLORREF(CLR_CARD));
        let old = SelectObject(hdc, pen_bg);
        let null_brush = GetStockObject(NULL_BRUSH);
        let _ = SelectObject(hdc, null_brush);
        let _ = Ellipse(hdc, cx - spinner_r, spinner_cy - spinner_r,
            cx + spinner_r + 1, spinner_cy + spinner_r + 1);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(pen_bg);

        // Rotating gradient arc
        let segments = 60;
        let start_angle = (t * 6) % 360;
        let sweep = 160.0;

        use std::f64::consts::PI;
        for i in 0..segments {
            let frac = i as f64 / segments as f64;
            let angle = (start_angle as f64 + frac * sweep) * PI / 180.0;
            let next_angle = (start_angle as f64 + (i + 1) as f64 / segments as f64 * sweep) * PI / 180.0;

            let fade = 1.0 - frac * 0.6;
            let r_col = ((CLR_ACCENT & 0xFF) as f64 * fade) as u32;
            let g_col = (((CLR_ACCENT >> 8) & 0xFF) as f64 * fade) as u32;
            let b_col = (((CLR_ACCENT >> 16) & 0xFF) as f64 * fade) as u32;
            let seg_color = 0x00FFFFFF & (r_col | (g_col << 8) | (b_col << 16));

            let seg_pen = CreatePen(PS_SOLID, 3, COLORREF(seg_color));
            let _ = SelectObject(hdc, seg_pen);

            let x1 = cx + (spinner_r as f64 * angle.cos()) as i32;
            let y1 = spinner_cy + (spinner_r as f64 * angle.sin()) as i32;
            let x2 = cx + (spinner_r as f64 * next_angle.cos()) as i32;
            let y2 = spinner_cy + (spinner_r as f64 * next_angle.sin()) as i32;

            let _ = MoveToEx(hdc, x1, y1, None);
            let _ = LineTo(hdc, x2, y2);
            let _ = DeleteObject(seg_pen);
        }
    }

    draw_text_center(hdc, PROGRESS_STEPS[step], cx, s(SHADOW_SIZE) + s(240), CLR_TEXT_BRIGHT, state.font_body);

    // Step dots
    let dots_y = s(SHADOW_SIZE) + s(275);
    let dot_spacing = s(20);
    let dot_r = s(3);
    let total_w = dot_spacing * 3;
    let start_x = cx - total_w / 2;

    unsafe {
        for i in 0..4 {
            let dx = start_x + i * dot_spacing;
            let is_done = i <= state.progress_step;
            let is_current = i == state.progress_step;

            if is_current {
                // Glow for current step
                let glow_r = s(7);
                let glow_br = CreateSolidBrush(COLORREF(make_glow_color(CLR_ACCENT, 40)));
                let old = SelectObject(hdc, glow_br);
                let pen_null = GetStockObject(NULL_PEN);
                let _ = SelectObject(hdc, pen_null);
                let _ = Ellipse(hdc, dx - glow_r, dots_y - glow_r, dx + glow_r + 1, dots_y + glow_r + 1);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(glow_br);
            }

            let color = if is_done { CLR_ACCENT } else { CLR_BORDER };
            let br = CreateSolidBrush(COLORREF(color));
            let old = SelectObject(hdc, br);
            let pen_null = GetStockObject(NULL_PEN);
            let _ = SelectObject(hdc, pen_null);

            if is_current {
                let pulse = ((t as f64 * 0.1).sin() * 0.3 + 1.0) as i32;
                let pr = dot_r * pulse;
                let _ = Ellipse(hdc, dx - pr, dots_y - pr, dx + pr + 1, dots_y + pr + 1);
            } else {
                let _ = Ellipse(hdc, dx - dot_r, dots_y - dot_r, dx + dot_r + 1, dots_y + dot_r + 1);
            }

            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(br);
        }
    }

    // Progress bar
    let bar_y = s(SHADOW_SIZE) + s(312);
    let bar_w = s(300);
    let bar_h = s(4);
    let bar_x = cx - bar_w / 2;

    unsafe {
        let track_br = CreateSolidBrush(COLORREF(CLR_CARD));
        draw_rounded_fill(hdc, &make_rect(bar_x, bar_y, bar_x + bar_w, bar_y + bar_h), s(2), track_br);
        let _ = DeleteObject(track_br);

        // Bouncing block with gradient
        let cycle = (t * 2) % 200;
        let block_w = s(70);
        let travel = bar_w - block_w;
        let pos = if cycle < 100 {
            (cycle as i32 * travel) / 100
        } else {
            ((200 - cycle) as i32 * travel) / 100
        };

        let fill_rect = make_rect(bar_x + pos, bar_y, bar_x + pos + block_w, bar_y + bar_h);
        draw_rounded_gradient(hdc, &fill_rect, s(2), CLR_ACCENT, CLR_ACCENT2, true);
    }
}

// ============ Phase: Done ============

fn paint_done(hdc: HDC, state: &State) {
    draw_header(hdc, state);
    draw_title_bar(hdc, state);

    let cx = s(W_WIDTH) / 2;
    let cy = s(SHADOW_SIZE) + s(190);
    let t = state.anim_tick;

    let max_r = s(42);
    let grow_progress = std::cmp::min(t, 28) as f64 / 28.0;
    let eased = 1.0 - (1.0 - grow_progress).powi(3);
    let r = (max_r as f64 * eased) as i32;

    unsafe {
        // Outer glow that grows with the circle
        if t > 5 {
            let glow_progress = std::cmp::min(t - 5, 30) as f64 / 30.0;
            let glow_eased = 1.0 - (1.0 - glow_progress).powi(2);
            for layer in 0i32..4 {
                let expand = s(8) + layer * s(4);
                let alpha = ((20 - layer as u32 * 4) as f64 * glow_eased) as u32;
                let glow_color = make_glow_color(CLR_SUCCESS, alpha);
                let glow_br = CreateSolidBrush(COLORREF(glow_color));
                let old = SelectObject(hdc, glow_br);
                let pen_null = GetStockObject(NULL_PEN);
                let _ = SelectObject(hdc, pen_null);
                let gr = r + expand;
                if gr > 0 {
                    let _ = Ellipse(hdc, cx - gr, cy - gr, cx + gr + 1, cy + gr + 1);
                }
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(glow_br);
            }
        }

        // Success circle with gradient
        let rect = make_rect(cx - r, cy - r, cx + r + 1, cy + r + 1);
        if r > 0 {
            let hrgn = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, r, r);
            let _ = SelectClipRgn(hdc, hrgn);
            draw_gradient_v(hdc, &rect, CLR_SUCCESS, CLR_SUCCESS_SOFT);
            let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
            let _ = DeleteObject(hrgn);

            // Top highlight
            let hl_pen = CreatePen(PS_SOLID, 1, COLORREF(lighten_color(CLR_SUCCESS, 30)));
            let old = SelectObject(hdc, hl_pen);
            let hrgn2 = CreateRoundRectRgn(rect.left, rect.top, rect.right, rect.bottom, r, r);
            let _ = SelectClipRgn(hdc, hrgn2);
            let _ = MoveToEx(hdc, cx - r + s(4), cy - r + 1, None);
            let _ = LineTo(hdc, cx + r - s(4), cy - r + 1);
            let _ = SelectClipRgn(hdc, HRGN(std::ptr::null_mut()));
            let _ = DeleteObject(hrgn2);
            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(hl_pen);
        }

        // Checkmark
        if t >= 20 {
            let check_progress = std::cmp::min(t - 20, 24) as f64 / 24.0;
            let eased_check = 1.0 - (1.0 - check_progress).powi(2);

            let check_pen = CreatePen(PS_SOLID, 3, COLORREF(CLR_WHITE));
            let old = SelectObject(hdc, check_pen);

            let p1x = cx - s(14);
            let p1y = cy - s(1);
            let p2x = cx - s(3);
            let p2y = cy + s(11);
            let p3x = cx + s(15);
            let p3y = cy - s(9);

            if eased_check < 0.4 {
                let frac = eased_check / 0.4;
                let ex = p1x + ((p2x - p1x) as f64 * frac) as i32;
                let ey = p1y + ((p2y - p1y) as f64 * frac) as i32;
                let _ = MoveToEx(hdc, p1x, p1y, None);
                let _ = LineTo(hdc, ex, ey);
            } else {
                let _ = MoveToEx(hdc, p1x, p1y, None);
                let _ = LineTo(hdc, p2x, p2y);
                let frac = (eased_check - 0.4) / 0.6;
                let ex = p2x + ((p3x - p2x) as f64 * frac) as i32;
                let ey = p2y + ((p3y - p2y) as f64 * frac) as i32;
                let _ = MoveToEx(hdc, p2x, p2y, None);
                let _ = LineTo(hdc, ex, ey);
            }

            let _ = SelectObject(hdc, old);
            let _ = DeleteObject(check_pen);
        }

        // Particle burst
        if t >= 45 {
            let particle_count = 12;
            let age = (t - 45) as f64;
            let max_age = 50.0;
            let alpha = 1.0 - (age / max_age).min(1.0);

            for i in 0..particle_count {
                let angle = (i as f64 / particle_count as f64) * 2.0 * std::f64::consts::PI
                    + age * 0.015;
                let dist = s(55) + (age * 1.5) as i32;
                let px = cx + (dist as f64 * angle.cos()) as i32;
                let py = cy + (dist as f64 * angle.sin()) as i32;

                let col = lighten_color(CLR_SUCCESS, (alpha * 40.0) as u32);
                let br = CreateSolidBrush(COLORREF(col));
                let old = SelectObject(hdc, br);
                let pen_null = GetStockObject(NULL_PEN);
                let _ = SelectObject(hdc, pen_null);
                let pr = s(2) + ((alpha * 2.0) as i32);
                let _ = Ellipse(hdc, px - pr, py - pr, px + pr + 1, py + pr + 1);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(br);
            }
        }
    }

    // Fade-in text
    if t >= 15 {
        let text_fade = std::cmp::min(t - 15, 22) as f64 / 22.0;
        let text_alpha = (text_fade * 255.0) as u32;

        let mix = |fg: u32, alpha: u32| -> u32 {
            let fr = fg & 0xFF;
            let fg2 = (fg >> 8) & 0xFF;
            let fb = (fg >> 16) & 0xFF;
            let br = CLR_BG & 0xFF;
            let bg2 = (CLR_BG >> 8) & 0xFF;
            let bb = (CLR_BG >> 16) & 0xFF;
            let r = (fr * alpha + br * (255 - alpha)) / 255;
            let g = (fg2 * alpha + bg2 * (255 - alpha)) / 255;
            let b = (fb * alpha + bb * (255 - alpha)) / 255;
            0x00FFFFFF & (r | (g << 8) | (b << 16))
        };

        draw_text_center(hdc, "卸载完成", cx, s(SHADOW_SIZE) + s(258),
            mix(CLR_SUCCESS, text_alpha), state.font_big);

        if t >= 32 {
            let sub_fade = std::cmp::min(t - 32, 22) as f64 / 22.0;
            let sub_alpha = (sub_fade * 255.0) as u32;
            draw_text_center(hdc, "Phonon 已成功从您的电脑中移除", cx, s(SHADOW_SIZE) + s(298),
                mix(CLR_TEXT, sub_alpha), state.font_body);
        }

        if t >= 52 {
            let hint_fade = std::cmp::min(t - 52, 22) as f64 / 22.0;
            let hint_alpha = (hint_fade * 255.0) as u32;
            draw_text_center(hdc, "窗口即将自动关闭...", cx, s(SHADOW_SIZE) + s(330),
                mix(CLR_TEXT_DIM, hint_alpha), state.font_small);
        }
    }
}

// ============ Interaction ============

fn on_lbuttondown(hwnd: HWND, state: &mut State, lparam: LPARAM) {
    let x = get_x_lparam(lparam);
    let y = get_y_lparam(lparam);

    let inner = window_inner_rect();
    let title_bottom = inner.top + s(TITLE_BAR_H);

    if y < title_bottom {
        if point_in_rect(x, y, &close_button_rect()) {
            unsafe { let _ = DestroyWindow(hwnd); }
            return;
        }
        unsafe {
            let _ = ReleaseCapture();
            let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, WPARAM(2), LPARAM(0));
        }
        return;
    }

    match state.phase {
        Phase::Select => {
            let opt = hit_test_option(x, y);
            if opt >= 0 {
                state.selected = opt;
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
                return;
            }
            if point_in_rect(x, y, &left_button_rect()) {
                unsafe { let _ = DestroyWindow(hwnd); }
                return;
            }
            if point_in_rect(x, y, &right_button_rect()) {
                state.phase = Phase::Confirm;
                state.hover_btn = -1;
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
                return;
            }
        }
        Phase::Confirm => {
            if point_in_rect(x, y, &left_button_rect()) {
                state.phase = Phase::Select;
                state.hover_btn = -1;
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
                return;
            }
            if point_in_rect(x, y, &right_button_rect()) {
                state.phase = Phase::Uninstalling;
                state.hover_btn = -1;
                state.anim_tick = 0;
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
                perform_uninstall(hwnd, state.selected);
                return;
            }
        }
        Phase::Done => { unsafe { let _ = DestroyWindow(hwnd); } }
        Phase::Uninstalling => {}
    }
}

fn on_mousemove(hwnd: HWND, state: &mut State, lparam: LPARAM) {
    let x = get_x_lparam(lparam);
    let y = get_y_lparam(lparam);
    let mut changed = false;

    let inner = window_inner_rect();
    let title_bottom = inner.top + s(TITLE_BAR_H);

    let hover_close = y < title_bottom && point_in_rect(x, y, &close_button_rect());
    if hover_close != state.hover_close { state.hover_close = hover_close; changed = true; }

    match state.phase {
        Phase::Select => {
            let opt = hit_test_option(x, y);
            let lb = point_in_rect(x, y, &left_button_rect());
            let rb = point_in_rect(x, y, &right_button_rect());
            let new_opt = if opt >= 0 { opt } else { -1 };
            let new_btn = if lb { 0 } else if rb { 1 } else { -1 };
            if new_opt != state.hover_option || new_btn != state.hover_btn {
                state.hover_option = new_opt;
                state.hover_btn = new_btn;
                changed = true;
            }
        }
        Phase::Confirm => {
            let lb = point_in_rect(x, y, &left_button_rect());
            let rb = point_in_rect(x, y, &right_button_rect());
            let new_btn = if lb { 0 } else if rb { 1 } else { -1 };
            if new_btn != state.hover_btn { state.hover_btn = new_btn; changed = true; }
        }
        _ => {}
    }

    if changed { unsafe { let _ = InvalidateRect(hwnd, None, false); } }
}

// ============ Uninstall Logic ============

use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn perform_uninstall(hwnd: HWND, option: i32) {
    let hwnd_ptr = hwnd.0 as usize;
    thread::spawn(move || {
        post_progress(hwnd_ptr, 0);
        kill_process();

        post_progress(hwnd_ptr, 1);
        let exe_path = std::env::current_exe().unwrap_or_default();
        if let Some(install_dir) = exe_path.parent() {
            delete_program_files(install_dir, &exe_path);
        }

        post_progress(hwnd_ptr, 2);
        delete_app_data(option);

        post_progress(hwnd_ptr, 3);
        clean_registry();

        unsafe {
            let _ = PostMessageW(HWND(hwnd_ptr as *mut std::ffi::c_void), WM_DONE, WPARAM(0), LPARAM(0));
        }
    });
}

fn post_progress(hwnd_ptr: usize, step: i32) {
    unsafe {
        let _ = PostMessageW(
            HWND(hwnd_ptr as *mut std::ffi::c_void),
            WM_PROGRESS,
            WPARAM(step as usize),
            LPARAM(0),
        );
    }
}

fn kill_process() {
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", "Phonon.exe", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    thread::sleep(std::time::Duration::from_millis(500));
}

fn delete_program_files(install_dir: &Path, exe_path: &Path) {
    let exe_name = exe_path.file_name().unwrap_or_default();
    if let Ok(entries) = std::fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name() == Some(exe_name) { continue; }
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn delete_app_data(option: i32) {
    let Some(data_dir) = get_data_dir() else { return };
    match option {
        0 => {}
        1 => {
            let keep = ["library.sqlite", "replaygains.sqlite"];
            delete_dir_contents_except(&data_dir, &keep);
        }
        2 => {
            let keep = ["plugins"];
            delete_dir_contents_except(&data_dir, &keep);
        }
        3 => { let _ = std::fs::remove_dir_all(&data_dir); }
        _ => {}
    }
}

fn delete_dir_contents_except(dir: &Path, keep: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if keep.contains(&name) { continue; }
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn clean_registry() {
    let keys = [
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\Phonon",
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.phonon.app",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Phonon",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.phonon.app",
        r"HKLM\Software\Phonon",
        r"HKCU\Software\Phonon",
        r"HKLM\Software\com.phonon.app",
        r"HKCU\Software\com.phonon.app",
        r"HKLM\Software\Classes\phonon",
        r"HKCU\Software\Classes\phonon",
    ];
    for key in &keys {
        let _ = std::process::Command::new("reg")
            .args(["delete", key, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

fn self_delete() {
    let Ok(exe_path) = std::env::current_exe() else { return };
    let Some(install_dir) = exe_path.parent() else { return };

    let exe_str = exe_path.to_string_lossy();
    let dir_str = install_dir.to_string_lossy();
    let bat_content = format!(
        "@echo off\r\n\
         :retry\r\n\
         timeout /t 1 /nobreak >nul\r\n\
         del /f /q \"{exe}\" 2>nul\r\n\
         if exist \"{exe}\" goto retry\r\n\
         rd /s /q \"{dir}\" 2>nul\r\n\
         del /f /q \"%~f0\"\r\n",
        exe = exe_str,
        dir = dir_str,
    );

    let bat_path = std::env::temp_dir().join("phonon_cleanup.bat");
    if std::fs::write(&bat_path, &bat_content).is_err() { return; }

    let bat_str = bat_path.to_string_lossy().to_string();
    let _ = std::process::Command::new("cmd")
        .args(["/c", &bat_str])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
