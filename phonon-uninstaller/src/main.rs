#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};
use std::thread;

use phonon_winui::d2d::{tf, Canvas, FontStyle};
use phonon_winui::s;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetStockObject, InvalidateRect, UpdateWindow, HBRUSH, NULL_BRUSH,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SendMessageW, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_ARROW, MSG, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOWDEFAULT, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEMOVE,
    WM_NCLBUTTONDOWN, WM_PAINT, WM_TIMER, WM_USER, WNDCLASSEXW, WS_CLIPSIBLINGS, WS_EX_APPWINDOW,
    WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};

// ============ Colors (COLORREF = 0x00BBGGRR) ============

const CLR_BG: u32 = 0x00140A0A;
const CLR_CARD: u32 = 0x001D1111;
const CLR_CARD_HOVER: u32 = 0x002D1A1A;
const CLR_CARD_SELECTED: u32 = 0x00281616;
const CLR_BORDER: u32 = 0x003A2020;
const CLR_BORDER_GLOW: u32 = 0x00803C3C;
const CLR_WINDOW_BORDER: u32 = 0x004A2828;
const CLR_TEXT: u32 = 0x00B0A090;
const CLR_TEXT_BRIGHT: u32 = 0x00F0E8E0;
const CLR_TEXT_DIM: u32 = 0x00706060;
const CLR_ACCENT: u32 = 0x00FFD44A;
const CLR_ACCENT2: u32 = 0x00FF8888;
const CLR_ACCENT_SOFT: u32 = 0x00A08C5A;
const CLR_ACCENT_GLOW: u32 = 0x00FFE880;
const CLR_DANGER: u32 = 0x006650FF;
const CLR_DANGER_HOVER: u32 = 0x008060FF;
const CLR_DANGER_SOFT: u32 = 0x00442088;
const CLR_SUCCESS: u32 = 0x0060E848;
const CLR_SUCCESS_SOFT: u32 = 0x00307020;
const CLR_WHITE: u32 = 0x00FFFFFF;
const CLR_CLOSE_HOVER: u32 = 0x003018F0;
const CLR_HEADER_GRAD1: u32 = 0x00381414;
const CLR_HEADER_GRAD2: u32 = 0x00140A0A;
const CLR_ICON_BG: u32 = 0x002A1515;

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
enum Phase {
    Select,
    Confirm,
    Uninstalling,
    Done,
}

// ============ Option icon kinds ============

enum IconKind {
    Shield,
    Music,
    Trash,
    Eraser,
}

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
    canvas: Option<Canvas>,
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
            canvas: None,
        }
    }
}

// ============ Utilities ============

fn get_x_lparam(lp: LPARAM) -> i32 {
    (lp.0 as u32 & 0xFFFF) as i16 as i32
}
fn get_y_lparam(lp: LPARAM) -> i32 {
    ((lp.0 as u32 >> 16) & 0xFFFF) as i16 as i32
}

fn make_rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

fn window_inner_rect() -> RECT {
    make_rect(
        s(SHADOW_SIZE),
        s(SHADOW_SIZE),
        s(W_WIDTH) - s(SHADOW_SIZE),
        s(W_HEIGHT) - s(SHADOW_SIZE),
    )
}

fn option_rect(idx: i32) -> RECT {
    let y = s(OPTION_START_Y) + (s(OPTION_H) + s(OPTION_GAP)) * idx;
    make_rect(s(PADDING), y, s(W_WIDTH) - s(PADDING), y + s(OPTION_H))
}

fn left_button_rect() -> RECT {
    make_rect(
        s(PADDING),
        s(BUTTONS_Y),
        s(PADDING) + s(BUTTON_W),
        s(BUTTONS_Y) + s(BUTTON_H),
    )
}

fn right_button_rect() -> RECT {
    make_rect(
        s(W_WIDTH) - s(PADDING) - s(BUTTON_W),
        s(BUTTONS_Y),
        s(W_WIDTH) - s(PADDING),
        s(BUTTONS_Y) + s(BUTTON_H),
    )
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
    for i in 0..4 {
        if point_in_rect(x, y, &option_rect(i)) {
            return i;
        }
    }
    -1
}

fn get_data_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let p1 = PathBuf::from(&appdata).join("com.phonon.app");
    if p1.exists() {
        return Some(p1);
    }
    let p2 = PathBuf::from(&appdata).join("Phonon");
    if p2.exists() {
        return Some(p2);
    }
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
            WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            w!("PhononUninstaller"),
            w!("Phonon 卸载程序"),
            WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
            x,
            y,
            s(W_WIDTH),
            s(W_HEIGHT),
            None,
            None,
            hinstance,
            None,
        )
    }
    .expect("CreateWindowExW");

    unsafe {
        // Per-pixel alpha comes from DirectComposition — no colorkey or
        // layered attributes needed (WS_EX_NOREDIRECTIONBITMAP is set).
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let _ = UpdateWindow(hwnd);
        let _ = SetTimer(hwnd, TIMER_ANIMATE, 33, None);
    }

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
}

// ============ Window Procedure ============

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
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
                if state.uninstalled {
                    self_delete();
                }
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
        let mut rc: RECT = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w <= 0 || h <= 0 {
            return;
        }

        // Presentation goes through the composition swapchain — BeginPaint
        // is only needed to validate the update region.
        let mut ps = PAINTSTRUCT::default();
        let _hdc = BeginPaint(hwnd, &mut ps);

        // (Re)create the render target on first paint and after device loss.
        if state.canvas.as_ref().is_none_or(|c| c.needs_recreate()) {
            match Canvas::new(hwnd, w, h) {
                Ok(c) => state.canvas = Some(c),
                Err(e) => {
                    log::error!("D2D render target creation failed: {}", e);
                    let _ = EndPaint(hwnd, &ps);
                    return;
                }
            }
        }
        // Copy the small paint inputs before taking the canvas mutably —
        // avoids a double borrow of `state`.
        let phase = state.phase;
        let selected = state.selected;
        let hover_btn = state.hover_btn;
        let hover_close = state.hover_close;
        let progress_step = state.progress_step;
        let anim_tick = state.anim_tick;
        let hover_option = state.hover_option;
        let canvas = state.canvas.as_mut().unwrap();

        canvas.begin();

        // Draw window frame (rounded body + border)
        draw_window_body(canvas);

        match phase {
            Phase::Select => paint_select(
                canvas,
                anim_tick,
                selected,
                hover_option,
                hover_close,
                hover_btn,
            ),
            Phase::Confirm => paint_confirm(canvas, anim_tick, selected, hover_close, hover_btn),
            Phase::Uninstalling => paint_progress(canvas, progress_step, anim_tick, hover_close),
            Phase::Done => paint_done(canvas, anim_tick, hover_close),
        }

        canvas.end();
        let _ = EndPaint(hwnd, &ps);
    }
}

fn draw_window_body(canvas: &mut Canvas) {
    let inner = window_inner_rect();
    let radius = s(WINDOW_RADIUS);

    // True soft drop shadow (alpha rects, blended by DWM over the desktop)
    for i in 0..4 {
        let e = s(6) - i * s(2);
        let a = 0.12 - i as f32 * 0.03;
        let gr = make_rect(
            inner.left - e,
            inner.top - e,
            inner.right + e,
            inner.bottom + e,
        );
        canvas.fill_round_alpha(&gr, radius + e, 0x00000000, a.max(0.03));
    }

    // Main window background with rounded corners
    canvas.fill_round(&inner, radius, CLR_BG);

    // Window border
    canvas.stroke_round(&inner, radius, CLR_WINDOW_BORDER, 1.0);
}

fn draw_text_left(canvas: &mut Canvas, text: &str, x: i32, y: i32, color: u32, style: FontStyle) {
    let max_w = s(W_WIDTH) - x;
    let (_, h) = canvas.measure_text(text, max_w, style, false);
    let rect = make_rect(x, y, x + max_w, y + h.max(s(14)));
    canvas.text(
        text,
        &rect,
        style,
        color,
        tf::TOP | tf::LEFT | tf::SINGLELINE,
    );
}

fn draw_text_center(
    canvas: &mut Canvas,
    text: &str,
    cx: i32,
    y: i32,
    color: u32,
    style: FontStyle,
) {
    let (w, h) = canvas.measure_text(text, s(W_WIDTH), style, false);
    let rect = make_rect(cx - w / 2, y, cx - w / 2 + w, y + h.max(s(14)));
    canvas.text(
        text,
        &rect,
        style,
        color,
        tf::TOP | tf::CENTER | tf::SINGLELINE,
    );
}

// ============ Icons (redesigned with cleaner, more modern look) ============

fn draw_option_icon(canvas: &mut Canvas, cx: i32, cy: i32, kind: &IconKind, selected: bool) {
    let size = s(OPTION_ICON_SIZE);
    let half = size / 2;
    let r = s(9); // rounded square radius
    let rect = make_rect(cx - half, cy - half, cx + half, cy + half);

    // Background - gradient for selected, solid for unselected
    if selected {
        canvas.fill_round_gradient(&rect, r, CLR_ACCENT, CLR_ACCENT2, true);
    } else {
        canvas.fill_round(&rect, r, CLR_ICON_BG);
        canvas.stroke_round(&rect, r, CLR_BORDER, 1.0);
    }

    // Icon glyph color
    let glyph_color = if selected { CLR_WHITE } else { CLR_ACCENT };
    let gw = 2.0;

    match kind {
        IconKind::Shield => {
            // Cleaner shield with checkmark
            let top_y = cy - half + s(7);
            let bottom_y = cy + half - s(6);
            let left_x = cx - half + s(8);
            let right_x = cx + half - s(8);
            let mid_y = cy + s(2);

            // Shield outline
            canvas.line(cx, top_y, right_x, top_y + s(3), glyph_color, gw);
            canvas.line(right_x, top_y + s(3), right_x, mid_y, glyph_color, gw);
            canvas.line(right_x, mid_y, cx + s(5), bottom_y - s(1), glyph_color, gw);
            canvas.line(cx + s(5), bottom_y - s(1), cx, bottom_y, glyph_color, gw);
            canvas.line(cx, bottom_y, cx - s(5), bottom_y - s(1), glyph_color, gw);
            canvas.line(cx - s(5), bottom_y - s(1), left_x, mid_y, glyph_color, gw);
            canvas.line(left_x, mid_y, left_x, top_y + s(3), glyph_color, gw);
            canvas.line(left_x, top_y + s(3), cx, top_y, glyph_color, gw);

            // Checkmark inside shield
            canvas.line(cx - s(4), cy - s(1), cx - s(1), cy + s(3), glyph_color, 2.0);
            canvas.line(cx - s(1), cy + s(3), cx + s(5), cy - s(3), glyph_color, 2.0);
        }
        IconKind::Music => {
            // Cleaner music note - eighth note style
            let stem_x = cx + s(3);
            let note_top = cy - half + s(6);
            let note_bottom = cy + half - s(5);

            // Stem
            canvas.line(
                stem_x,
                note_top + s(2),
                stem_x,
                note_bottom - s(1),
                glyph_color,
                gw,
            );

            // Flag (beamed)
            canvas.line(
                stem_x,
                note_top + s(2),
                stem_x + s(5),
                note_top + s(5),
                glyph_color,
                gw,
            );
            canvas.line(
                stem_x + s(5),
                note_top + s(5),
                stem_x,
                note_top + s(8),
                glyph_color,
                gw,
            );

            // Note head (filled ellipse)
            let head_rx = s(5);
            let head_ry = s(4);
            let head_cx = cx - s(2);
            let head_cy = note_bottom - s(1);
            canvas.fill_ellipse(head_cx, head_cy, head_rx, head_ry, glyph_color);
        }
        IconKind::Trash => {
            // Cleaner trash can icon
            let top_y = cy - half + s(6);
            let bottom_y = cy + half - s(6);
            let left_x = cx - half + s(8);
            let right_x = cx + half - s(8);
            let lid_y = top_y + s(4);

            // Lid (horizontal bar - thicker)
            canvas.line(
                left_x - s(2),
                lid_y,
                right_x + s(2),
                lid_y,
                glyph_color,
                2.0,
            );

            // Lid handle (small rectangle on top)
            canvas.line(cx - s(3), top_y, cx + s(3), top_y, glyph_color, gw);
            canvas.line(cx - s(3), top_y, cx - s(3), lid_y, glyph_color, gw);
            canvas.line(cx + s(3), top_y, cx + s(3), lid_y, glyph_color, gw);

            // Can body sides
            canvas.line(
                left_x + s(1),
                lid_y,
                left_x + s(2),
                bottom_y,
                glyph_color,
                gw,
            );
            canvas.line(
                right_x - s(1),
                lid_y,
                right_x - s(2),
                bottom_y,
                glyph_color,
                gw,
            );

            // Bottom
            canvas.line(
                left_x + s(2),
                bottom_y,
                right_x - s(2),
                bottom_y,
                glyph_color,
                gw,
            );

            // Inner lines (representing trash)
            canvas.line(
                cx - s(3),
                lid_y + s(5),
                cx - s(3),
                bottom_y - s(3),
                glyph_color,
                1.0,
            );
            canvas.line(
                cx + s(3),
                lid_y + s(5),
                cx + s(3),
                bottom_y - s(3),
                glyph_color,
                1.0,
            );
        }
        IconKind::Eraser => {
            // Explosion/bomb icon - more dynamic
            let center_x = cx;
            let center_y = cy + s(1);
            let radius = s(6);

            // Main circle (bomb body)
            canvas.fill_ellipse(center_x, center_y, radius, radius, glyph_color);

            // Fuse (curved-ish)
            canvas.line(
                center_x + s(3),
                center_y - s(5),
                center_x + s(5),
                center_y - s(8),
                glyph_color,
                2.0,
            );
            canvas.line(
                center_x + s(5),
                center_y - s(8),
                center_x + s(7),
                center_y - s(6),
                glyph_color,
                2.0,
            );

            // Spark (small star-like)
            let spark_cx = center_x + s(7);
            let spark_cy = center_y - s(9);
            let sr = s(2);
            canvas.fill_ellipse(spark_cx, spark_cy, sr, sr, glyph_color);

            // X inside (white if selected, icon bg if not)
            let x_color = if selected { CLR_WHITE } else { CLR_ICON_BG };
            canvas.line(
                center_x - s(3),
                center_y - s(3),
                center_x + s(3),
                center_y + s(3),
                x_color,
                2.0,
            );
            canvas.line(
                center_x + s(3),
                center_y - s(3),
                center_x - s(3),
                center_y + s(3),
                x_color,
                2.0,
            );
        }
    }
}

// ============ Header / Branding ============

fn draw_header(canvas: &mut Canvas, anim_tick: u32) {
    let inner = window_inner_rect();
    let header_rect = make_rect(inner.left, inner.top, inner.right, inner.top + s(HEADER_H));

    // Clip to window rounded shape at top
    canvas.push_clip(&make_rect(
        inner.left,
        inner.top,
        inner.right,
        inner.top + s(HEADER_H) + s(WINDOW_RADIUS),
    ));
    canvas.fill_gradient_v(&header_rect, CLR_HEADER_GRAD1, CLR_HEADER_GRAD2);

    // Subtle radial glow at center bottom
    let cx = s(W_WIDTH) / 2;
    let glow_cy = inner.top + s(HEADER_H) - s(10);
    for i in (0i32..5).rev() {
        let rx = s(60) + i * s(12);
        let ry = s(20) + i * s(5);
        let alpha = 10 + i as u32 * 6;
        let glow_color = make_glow_color(CLR_ACCENT, alpha);
        canvas.fill_ellipse(cx, glow_cy, rx, ry, glow_color);
    }

    canvas.pop_clip();

    // Sound wave decoration with animation
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
        let t = anim_tick as f64;
        let wave = (t * 0.06 + i as f64 * 0.6).sin() * 0.35 + 1.0;
        let height_var = (base_h as f64 * wave) as i32;
        let height_var = height_var.max(s(2)).min(base_h + s(5));

        // Glow behind center wave
        if i == 3 {
            let glow_color = make_glow_color(CLR_ACCENT, 25);
            let gr = s(6);
            canvas.fill_ellipse(
                cx + offset,
                base_y - height_var + (base_y + height_var - (base_y - height_var)) / 2,
                gr,
                (base_y + height_var - (base_y - height_var)) / 2 + gr,
                glow_color,
            );
        }

        let color = if i == 3 { CLR_ACCENT } else { CLR_ACCENT_SOFT };
        canvas.line(
            cx + offset,
            base_y - height_var,
            cx + offset,
            base_y + height_var,
            color,
            2.0,
        );
    }

    // Accent dot at bottom center with glow
    let tallest_h = s(22);
    let dot_y = base_y + tallest_h + s(2);
    let glow_color = make_glow_color(CLR_ACCENT, 35);
    canvas.fill_ellipse(cx, dot_y, s(6), s(6), glow_color);
    canvas.fill_ellipse(cx, dot_y, s(3), s(3), CLR_ACCENT);

    // Brand text with subtle glow
    let title_y = s(SHADOW_SIZE) + s(22);
    let glow_color = make_glow_color(CLR_ACCENT, 20);
    let brand_w = s(W_WIDTH);
    let (bw, bh) = canvas.measure_text("PHONON", brand_w, FontStyle::Big, false);
    let tx = s(W_WIDTH) / 2 - bw / 2;
    let brand_rect = make_rect(tx, title_y, tx + bw, title_y + bh.max(s(20)));
    for dx in [-1, 0, 1] {
        for dy in [-1, 0, 1] {
            if dx == 0 && dy == 0 {
                continue;
            }
            let glow_rect = make_rect(
                brand_rect.left + dx,
                brand_rect.top + dy,
                brand_rect.right + dx,
                brand_rect.bottom + dy,
            );
            canvas.text(
                "PHONON",
                &glow_rect,
                FontStyle::Big,
                glow_color,
                tf::TOP | tf::LEFT | tf::SINGLELINE,
            );
        }
    }
    canvas.text(
        "PHONON",
        &brand_rect,
        FontStyle::Big,
        CLR_TEXT_BRIGHT,
        tf::TOP | tf::CENTER | tf::SINGLELINE,
    );
    draw_text_center(
        canvas,
        "高保真音频平台",
        s(W_WIDTH) / 2,
        s(SHADOW_SIZE) + s(56),
        CLR_TEXT_DIM,
        FontStyle::Small,
    );
}

fn draw_title_bar(canvas: &mut Canvas, hover_close: bool) {
    let inner = window_inner_rect();
    draw_text_left(
        canvas,
        "Phonon 卸载程序",
        inner.left + s(14),
        inner.top + s(9),
        CLR_TEXT_DIM,
        FontStyle::Small,
    );
    draw_close_button(canvas, hover_close);
}

fn draw_close_button(canvas: &mut Canvas, hover_close: bool) {
    let r = close_button_rect();

    if hover_close {
        // Clip to top-right corner of window
        let inner = window_inner_rect();
        canvas.push_clip(&inner);
        canvas.fill_rect(&r, CLR_CLOSE_HOVER);
        canvas.pop_clip();
    }
    let color = if hover_close { CLR_WHITE } else { CLR_TEXT_DIM };
    let m = s(8);
    let cx = r.left + (r.right - r.left) / 2;
    let cy = r.top + (r.bottom - r.top) / 2;
    canvas.line(cx - m, cy - m, cx + m, cy + m, color, 1.0);
    canvas.line(cx + m, cy - m, cx - m, cy + m, color, 1.0);
}

// ============ Button ============

enum ButtonKind {
    Primary,
    Outline,
    Danger,
}

fn draw_button(canvas: &mut Canvas, rect: &RECT, text: &str, kind: &ButtonKind, hovered: bool) {
    let radius = s(BTN_RADIUS);

    match kind {
        ButtonKind::Primary => {
            // Outer glow on hover
            if hovered {
                for layer in 0i32..3 {
                    let expand = 3 - layer;
                    let alpha = 30 + layer as u32 * 25;
                    let glow_color = make_glow_color(CLR_ACCENT, alpha);
                    let gr = make_rect(
                        rect.left - expand,
                        rect.top - expand,
                        rect.right + expand,
                        rect.bottom + expand,
                    );
                    canvas.fill_round(&gr, radius + expand, glow_color);
                }
            }
            let (c1, c2) = if hovered {
                (
                    lighten_color(CLR_ACCENT, 15),
                    lighten_color(CLR_ACCENT2, 15),
                )
            } else {
                (CLR_ACCENT, CLR_ACCENT2)
            };
            canvas.fill_round_gradient(rect, radius, c1, c2, true);
            // Top highlight
            let hl_color = 0x00FFFFFF & lighten_color(CLR_ACCENT, 40);
            canvas.push_clip(rect);
            canvas.line(
                rect.left + s(4),
                rect.top + 1,
                rect.right - s(4),
                rect.top + 1,
                hl_color,
                1.0,
            );
            canvas.pop_clip();
        }
        ButtonKind::Outline => {
            if hovered {
                canvas.fill_round(rect, radius, CLR_CARD_HOVER);
                canvas.push_clip(rect);
                canvas.line(
                    rect.left + s(4),
                    rect.top + 1,
                    rect.right - s(4),
                    rect.top + 1,
                    CLR_BORDER_GLOW,
                    1.0,
                );
                canvas.pop_clip();
            }
            let border_color = if hovered { CLR_BORDER_GLOW } else { CLR_BORDER };
            canvas.stroke_round(rect, radius, border_color, 1.0);
        }
        ButtonKind::Danger => {
            if hovered {
                // Outer glow
                for layer in 0i32..3 {
                    let expand = 3 - layer;
                    let alpha = 30 + layer as u32 * 25;
                    let glow_color = make_glow_color(CLR_DANGER, alpha);
                    let gr = make_rect(
                        rect.left - expand,
                        rect.top - expand,
                        rect.right + expand,
                        rect.bottom + expand,
                    );
                    canvas.fill_round(&gr, radius + expand, glow_color);
                }
                canvas.fill_round(rect, radius, CLR_DANGER_HOVER);
                canvas.push_clip(rect);
                canvas.line(
                    rect.left + s(4),
                    rect.top + 1,
                    rect.right - s(4),
                    rect.top + 1,
                    lighten_color(CLR_DANGER_HOVER, 30),
                    1.0,
                );
                canvas.pop_clip();
            } else {
                canvas.stroke_round(rect, radius, CLR_DANGER, 1.0);
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
    canvas.text(
        text,
        rect,
        if matches!(kind, ButtonKind::Primary) {
            FontStyle::BodyBold
        } else {
            FontStyle::Body
        },
        color,
        tf::CENTER | tf::VCENTER | tf::SINGLELINE,
    );
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

fn draw_option_card(canvas: &mut Canvas, idx: i32, selected: bool, hovered: bool) {
    let r = option_rect(idx);
    // selected/hovered passed in from paint_select
    let radius = s(CARD_RADIUS);

    let icon_kind = match idx {
        0 => IconKind::Shield,
        1 => IconKind::Music,
        2 => IconKind::Trash,
        _ => IconKind::Eraser,
    };

    // Outer glow for selected state
    if selected {
        for layer in 0i32..3 {
            let expand = 2 - layer;
            let alpha = 40 + layer as u32 * 30;
            let glow_color = make_glow_color(CLR_ACCENT, alpha);
            let gr = make_rect(
                r.left - expand,
                r.top - expand,
                r.right + expand,
                r.bottom + expand,
            );
            canvas.fill_round(&gr, radius + expand, glow_color);
        }
    }

    // Background
    let bg_color = if selected {
        CLR_CARD_SELECTED
    } else if hovered {
        CLR_CARD_HOVER
    } else {
        CLR_CARD
    };
    canvas.fill_round(&r, radius, bg_color);

    // Top highlight line for depth
    if selected || hovered {
        let highlight_color = if selected {
            CLR_ACCENT_GLOW
        } else {
            CLR_BORDER_GLOW
        };
        canvas.push_clip(&r);
        canvas.line(
            r.left + s(2),
            r.top + 1,
            r.right - s(2),
            r.top + 1,
            highlight_color,
            1.0,
        );
        canvas.pop_clip();
    }

    // Border
    if selected {
        // Gradient-like border using two passes
        canvas.stroke_round(&r, radius, CLR_ACCENT, 1.0);
        let inner_r = make_rect(r.left + 1, r.top + 1, r.right - 1, r.bottom - 1);
        canvas.stroke_round(&inner_r, radius - 1, CLR_ACCENT_GLOW, 1.0);
    } else if hovered {
        canvas.stroke_round(&r, radius, CLR_BORDER_GLOW, 1.0);
    } else {
        canvas.stroke_round(&r, radius, CLR_BORDER, 1.0);
    }

    // Left accent bar when selected - gradient pill style
    if selected {
        let bar_w = s(4);
        let bar_h = s(28);
        let bar_x = r.left + s(4);
        let bar_y = r.top + (r.bottom - r.top - bar_h) / 2;
        let bar_rect = make_rect(bar_x, bar_y, bar_x + bar_w * 2, bar_y + bar_h);
        canvas.fill_round_gradient(&bar_rect, bar_w, CLR_ACCENT, CLR_ACCENT2, false);
    }

    // Icon
    let icon_cx = s(OPTION_ICON_X) + s(OPTION_ICON_SIZE) / 2;
    let icon_cy = r.top + s(OPTION_H) / 2;
    draw_option_icon(canvas, icon_cx, icon_cy, &icon_kind, selected);

    // Text
    let title_color = if selected { CLR_TEXT_BRIGHT } else { CLR_TEXT };
    let title_style = if selected {
        FontStyle::BodyBold
    } else {
        FontStyle::Body
    };
    draw_text_left(
        canvas,
        OPTION_TITLES[idx as usize],
        s(OPTION_TEXT_X),
        r.top + s(14),
        title_color,
        title_style,
    );
    draw_text_left(
        canvas,
        OPTION_DESCS[idx as usize],
        s(OPTION_TEXT_X),
        r.top + s(32),
        CLR_TEXT_DIM,
        FontStyle::Small,
    );
}

// ============ Phase: Select ============

fn paint_select(
    canvas: &mut Canvas,
    anim_tick: u32,
    selected: i32,
    hover_option: i32,
    hover_close: bool,
    hover_btn: i32,
) {
    draw_header(canvas, anim_tick);
    draw_title_bar(canvas, hover_close);

    draw_text_left(
        canvas,
        "选择卸载方式",
        s(PADDING),
        s(SHADOW_SIZE) + s(116),
        CLR_TEXT_DIM,
        FontStyle::Micro,
    );

    for i in 0..4 {
        draw_option_card(canvas, i, selected == i, hover_option == i);
    }

    let lb = left_button_rect();
    let rb = right_button_rect();
    draw_button(canvas, &lb, "取消", &ButtonKind::Outline, hover_btn == 0);
    draw_button(canvas, &rb, "卸载", &ButtonKind::Primary, hover_btn == 1);
}

// ============ Phase: Confirm ============

fn paint_confirm(
    canvas: &mut Canvas,
    anim_tick: u32,
    selected: i32,
    hover_close: bool,
    hover_btn: i32,
) {
    draw_header(canvas, anim_tick);
    draw_title_bar(canvas, hover_close);

    let cx = s(W_WIDTH) / 2;
    let icon_cy = s(SHADOW_SIZE) + s(160);
    let icon_r = s(32);

    // Outer glow
    for layer in 0i32..3 {
        let expand = 4 - layer;
        let alpha = 25 + layer as u32 * 20;
        let glow_color = make_glow_color(CLR_DANGER, alpha);
        let gr = icon_r + expand;
        canvas.fill_ellipse(cx, icon_cy, gr, gr, glow_color);
    }

    // Warning circle with gradient
    let rect = make_rect(
        cx - icon_r,
        icon_cy - icon_r,
        cx + icon_r + 1,
        icon_cy + icon_r + 1,
    );
    canvas.fill_round_gradient(&rect, icon_r, CLR_DANGER, CLR_DANGER_SOFT, false);

    // Exclamation mark
    let bar_w = s(3);
    let bar_top = icon_cy - s(14);
    let bar_bottom = icon_cy + s(2);
    canvas.fill_round(
        &make_rect(cx - bar_w, bar_top, cx + bar_w + 1, bar_bottom),
        bar_w,
        CLR_WHITE,
    );
    let dot_r = s(3);
    let dot_y = icon_cy + s(13);
    canvas.fill_ellipse(cx, dot_y, dot_r, dot_r, CLR_WHITE);

    draw_text_center(
        canvas,
        "确认要卸载 Phonon 吗？",
        cx,
        s(SHADOW_SIZE) + s(208),
        CLR_TEXT_BRIGHT,
        FontStyle::Title,
    );
    draw_text_center(
        canvas,
        OPTION_DESCS[selected as usize],
        cx,
        s(SHADOW_SIZE) + s(236),
        CLR_ACCENT,
        FontStyle::Body,
    );
    draw_text_center(
        canvas,
        "此操作不可撤销",
        cx,
        s(SHADOW_SIZE) + s(266),
        CLR_DANGER,
        FontStyle::Small,
    );

    let lb = left_button_rect();
    let rb = right_button_rect();
    draw_button(canvas, &lb, "取消", &ButtonKind::Outline, hover_btn == 0);
    draw_button(canvas, &rb, "确认卸载", &ButtonKind::Danger, hover_btn == 1);
}

// ============ Phase: Progress ============

fn paint_progress(canvas: &mut Canvas, progress_step: i32, anim_tick: u32, hover_close: bool) {
    draw_header(canvas, anim_tick);
    draw_title_bar(canvas, hover_close);

    let cx = s(W_WIDTH) / 2;
    let step = progress_step.clamp(0, 3) as usize;
    let spinner_cy = s(SHADOW_SIZE) + s(180);
    let spinner_r = s(34);
    let t = anim_tick;

    // Outer glow pulse
    let pulse_phase = (t as f64 * 0.05).sin() * 0.3 + 0.7;
    let glow_r = spinner_r + s(8);
    for layer in 0i32..3 {
        let expand = s(6) - layer * s(2);
        let alpha = ((15 + layer as u32 * 12) as f64 * pulse_phase) as u32;
        let glow_color = make_glow_color(CLR_ACCENT, alpha);
        let gr = glow_r + expand;
        canvas.fill_ellipse(cx, spinner_cy, gr, gr, glow_color);
    }

    // Background ring
    canvas.stroke_ellipse(cx, spinner_cy, spinner_r, spinner_r, CLR_CARD, 3.0);

    // Rotating gradient arc
    let segments = 60;
    let start_angle = (t * 6) % 360;
    let sweep = 160.0;

    use std::f64::consts::PI;
    for i in 0..segments {
        let frac = i as f64 / segments as f64;
        let angle = (start_angle as f64 + frac * sweep) * PI / 180.0;
        let next_angle =
            (start_angle as f64 + (i + 1) as f64 / segments as f64 * sweep) * PI / 180.0;

        let fade = 1.0 - frac * 0.6;
        let r_col = ((CLR_ACCENT & 0xFF) as f64 * fade) as u32;
        let g_col = (((CLR_ACCENT >> 8) & 0xFF) as f64 * fade) as u32;
        let b_col = (((CLR_ACCENT >> 16) & 0xFF) as f64 * fade) as u32;
        let seg_color = 0x00FFFFFF & (r_col | (g_col << 8) | (b_col << 16));

        let x1 = cx + (spinner_r as f64 * angle.cos()) as i32;
        let y1 = spinner_cy + (spinner_r as f64 * angle.sin()) as i32;
        let x2 = cx + (spinner_r as f64 * next_angle.cos()) as i32;
        let y2 = spinner_cy + (spinner_r as f64 * next_angle.sin()) as i32;

        canvas.line(x1, y1, x2, y2, seg_color, 3.0);
    }

    draw_text_center(
        canvas,
        PROGRESS_STEPS[step],
        cx,
        s(SHADOW_SIZE) + s(240),
        CLR_TEXT_BRIGHT,
        FontStyle::Body,
    );

    // Step dots
    let dots_y = s(SHADOW_SIZE) + s(275);
    let dot_spacing = s(20);
    let dot_r = s(3);
    let total_w = dot_spacing * 3;
    let start_x = cx - total_w / 2;

    for i in 0..4 {
        let dx = start_x + i * dot_spacing;
        let is_done = i <= progress_step;
        let is_current = i == progress_step;

        if is_current {
            // Glow for current step
            let glow_r = s(7);
            let glow_color = make_glow_color(CLR_ACCENT, 40);
            canvas.fill_ellipse(dx, dots_y, glow_r, glow_r, glow_color);
        }

        let color = if is_done { CLR_ACCENT } else { CLR_BORDER };
        if is_current {
            let pulse = ((t as f64 * 0.1).sin() * 0.3 + 1.0) as i32;
            let pr = dot_r * pulse;
            canvas.fill_ellipse(dx, dots_y, pr, pr, color);
        } else {
            canvas.fill_ellipse(dx, dots_y, dot_r, dot_r, color);
        }
    }

    // Progress bar
    let bar_y = s(SHADOW_SIZE) + s(312);
    let bar_w = s(300);
    let bar_h = s(4);
    let bar_x = cx - bar_w / 2;

    canvas.fill_round(
        &make_rect(bar_x, bar_y, bar_x + bar_w, bar_y + bar_h),
        s(2),
        CLR_CARD,
    );

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
    canvas.fill_round_gradient(&fill_rect, s(2), CLR_ACCENT, CLR_ACCENT2, true);
}

// ============ Phase: Done ============

fn paint_done(canvas: &mut Canvas, anim_tick: u32, hover_close: bool) {
    draw_header(canvas, anim_tick);
    draw_title_bar(canvas, hover_close);

    let cx = s(W_WIDTH) / 2;
    let cy = s(SHADOW_SIZE) + s(190);
    let t = anim_tick;

    let max_r = s(42);
    let grow_progress = std::cmp::min(t, 28) as f64 / 28.0;
    let eased = 1.0 - (1.0 - grow_progress).powi(3);
    let r = (max_r as f64 * eased) as i32;

    // Outer glow that grows with the circle
    if t > 5 {
        let glow_progress = std::cmp::min(t - 5, 30) as f64 / 30.0;
        let glow_eased = 1.0 - (1.0 - glow_progress).powi(2);
        for layer in 0i32..4 {
            let expand = s(8) + layer * s(4);
            let alpha = ((20 - layer as u32 * 4) as f64 * glow_eased) as u32;
            let glow_color = make_glow_color(CLR_SUCCESS, alpha);
            let gr = r + expand;
            if gr > 0 {
                canvas.fill_ellipse(cx, cy, gr, gr, glow_color);
            }
        }
    }

    // Success circle with gradient
    if r > 0 {
        let rect = make_rect(cx - r, cy - r, cx + r + 1, cy + r + 1);
        canvas.fill_round_gradient(&rect, r, CLR_SUCCESS, CLR_SUCCESS_SOFT, false);

        // Top highlight
        canvas.push_clip(&rect);
        canvas.line(
            cx - r + s(4),
            cy - r + 1,
            cx + r - s(4),
            cy - r + 1,
            lighten_color(CLR_SUCCESS, 30),
            1.0,
        );
        canvas.pop_clip();
    }

    // Checkmark
    if t >= 20 {
        let check_progress = std::cmp::min(t - 20, 24) as f64 / 24.0;
        let eased_check = 1.0 - (1.0 - check_progress).powi(2);

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
            canvas.line(p1x, p1y, ex, ey, CLR_WHITE, 3.0);
        } else {
            canvas.line(p1x, p1y, p2x, p2y, CLR_WHITE, 3.0);
            let frac = (eased_check - 0.4) / 0.6;
            let ex = p2x + ((p3x - p2x) as f64 * frac) as i32;
            let ey = p2y + ((p3y - p2y) as f64 * frac) as i32;
            canvas.line(p2x, p2y, ex, ey, CLR_WHITE, 3.0);
        }
    }

    // Particle burst
    if t >= 45 {
        let particle_count = 12;
        let age = (t - 45) as f64;
        let max_age = 50.0;
        let alpha = 1.0 - (age / max_age).min(1.0);

        for i in 0..particle_count {
            let angle =
                (i as f64 / particle_count as f64) * 2.0 * std::f64::consts::PI + age * 0.015;
            let dist = s(55) + (age * 1.5) as i32;
            let px = cx + (dist as f64 * angle.cos()) as i32;
            let py = cy + (dist as f64 * angle.sin()) as i32;

            let col = lighten_color(CLR_SUCCESS, (alpha * 40.0) as u32);
            let pr = s(2) + ((alpha * 2.0) as i32);
            canvas.fill_ellipse(px, py, pr, pr, col);
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

        draw_text_center(
            canvas,
            "卸载完成",
            cx,
            s(SHADOW_SIZE) + s(258),
            mix(CLR_SUCCESS, text_alpha),
            FontStyle::Big,
        );

        if t >= 32 {
            let sub_fade = std::cmp::min(t - 32, 22) as f64 / 22.0;
            let sub_alpha = (sub_fade * 255.0) as u32;
            draw_text_center(
                canvas,
                "Phonon 已成功从您的电脑中移除",
                cx,
                s(SHADOW_SIZE) + s(298),
                mix(CLR_TEXT, sub_alpha),
                FontStyle::Body,
            );
        }

        if t >= 52 {
            let hint_fade = std::cmp::min(t - 52, 22) as f64 / 22.0;
            let hint_alpha = (hint_fade * 255.0) as u32;
            draw_text_center(
                canvas,
                "窗口即将自动关闭...",
                cx,
                s(SHADOW_SIZE) + s(330),
                mix(CLR_TEXT_DIM, hint_alpha),
                FontStyle::Small,
            );
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
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
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
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
                return;
            }
            if point_in_rect(x, y, &left_button_rect()) {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return;
            }
            if point_in_rect(x, y, &right_button_rect()) {
                state.phase = Phase::Confirm;
                state.hover_btn = -1;
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
                return;
            }
        }
        Phase::Confirm => {
            if point_in_rect(x, y, &left_button_rect()) {
                state.phase = Phase::Select;
                state.hover_btn = -1;
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
                return;
            }
            if point_in_rect(x, y, &right_button_rect()) {
                state.phase = Phase::Uninstalling;
                state.hover_btn = -1;
                state.anim_tick = 0;
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
                perform_uninstall(hwnd, state.selected);
                return;
            }
        }
        Phase::Done => unsafe {
            let _ = DestroyWindow(hwnd);
        },
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
    if hover_close != state.hover_close {
        state.hover_close = hover_close;
        changed = true;
    }

    match state.phase {
        Phase::Select => {
            let opt = hit_test_option(x, y);
            let lb = point_in_rect(x, y, &left_button_rect());
            let rb = point_in_rect(x, y, &right_button_rect());
            let new_opt = if opt >= 0 { opt } else { -1 };
            let new_btn = if lb {
                0
            } else if rb {
                1
            } else {
                -1
            };
            if new_opt != state.hover_option || new_btn != state.hover_btn {
                state.hover_option = new_opt;
                state.hover_btn = new_btn;
                changed = true;
            }
        }
        Phase::Confirm => {
            let lb = point_in_rect(x, y, &left_button_rect());
            let rb = point_in_rect(x, y, &right_button_rect());
            let new_btn = if lb {
                0
            } else if rb {
                1
            } else {
                -1
            };
            if new_btn != state.hover_btn {
                state.hover_btn = new_btn;
                changed = true;
            }
        }
        _ => {}
    }

    if changed {
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
    }
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
            let _ = PostMessageW(
                HWND(hwnd_ptr as *mut std::ffi::c_void),
                WM_DONE,
                WPARAM(0),
                LPARAM(0),
            );
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
            if path.file_name() == Some(exe_name) {
                continue;
            }
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn delete_app_data(option: i32) {
    let Some(data_dir) = get_data_dir() else {
        return;
    };
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
        3 => {
            let _ = std::fs::remove_dir_all(&data_dir);
        }
        _ => {}
    }
}

fn delete_dir_contents_except(dir: &Path, keep: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if keep.contains(&name) {
            continue;
        }
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
    let Ok(exe_path) = std::env::current_exe() else {
        return;
    };
    let Some(install_dir) = exe_path.parent() else {
        return;
    };

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
    if std::fs::write(&bat_path, &bat_content).is_err() {
        return;
    }

    let bat_str = bat_path.to_string_lossy().to_string();
    let _ = std::process::Command::new("cmd")
        .args(["/c", &bat_str])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
