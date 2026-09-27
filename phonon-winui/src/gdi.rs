//! Low-level GDI drawing helpers.
//!
//! These are building blocks used by widgets.  They wrap raw GDI calls
//! with safer ergonomics (round rects, text with DT_ flags, double-buffering).

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::*;

use crate::theme::Theme;

/// Create a RECT from left/top/right/bottom.
#[inline]
pub fn make_rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

/// Point-in-rect test.
#[inline]
pub fn point_in_rect(x: i32, y: i32, r: &RECT) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Extract X coordinate from LPARAM (client-area mouse message).
#[inline]
pub fn lparam_x(lp: isize) -> i32 { (lp as u32 & 0xFFFF) as i16 as i32 }

/// Extract Y coordinate from LPARAM.
#[inline]
pub fn lparam_y(lp: isize) -> i32 { ((lp as u32) >> 16 & 0xFFFF) as i16 as i32 }

/// Convert a Rust string to a null-terminated UTF-16 Vec.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ============ Double-buffered paint helper ============

/// Run `draw_fn` on an off-screen DC, then blit to `hdc`.
///
/// Prevents flicker and tearing when the window has many controls.
pub fn with_double_buffer<F>(hdc: HDC, rc: &RECT, draw_fn: F)
where
    F: FnOnce(HDC),
{
    unsafe {
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        let mem_dc = CreateCompatibleDC(hdc);
        let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem_dc, mem_bmp);

        draw_fn(mem_dc);

        let _ = BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
        let _ = SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(mem_bmp);
        let _ = DeleteDC(mem_dc);
    }
}

// ============ Rounded rectangle ============

/// Draw and fill a rounded rectangle with the given brush and pen.
pub fn draw_rounded_rect(hdc: HDC, rc: &RECT, radius: i32, brush: HBRUSH, pen: HPEN) {
    unsafe {
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        let _ = RoundRect(
            hdc,
            rc.left, rc.top, rc.right, rc.bottom,
            radius, radius,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
    }
}

/// Fill a rounded rectangle (no border).
pub fn fill_rounded_rect(hdc: HDC, rc: &RECT, radius: i32, brush: HBRUSH) {
    unsafe {
        let null_pen = GetStockObject(NULL_PEN);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, null_pen);
        let _ = RoundRect(
            hdc,
            rc.left, rc.top, rc.right, rc.bottom,
            radius, radius,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
    }
}

// ============ Text drawing ============

/// Draw text with the given font, color, and DT_ flags.
pub fn draw_text(
    hdc: HDC,
    text: &str,
    rc: &RECT,
    font: HFONT,
    color: u32,
    flags: DRAW_TEXT_FORMAT,
) -> i32 {
    if text.is_empty() { return 0; }
    let mut wtext: Vec<u16> = text.encode_utf16().collect();
    let mut rc_mut = *rc;
    unsafe {
        let old_font = SelectObject(hdc, font);
        let old_color = SetTextColor(hdc, COLORREF(color));
        let _ = SetBkMode(hdc, TRANSPARENT);
        let result = DrawTextW(
            hdc,
            &mut wtext,
            &mut rc_mut as *mut RECT,
            flags,
        );
        let _ = SelectObject(hdc, old_font);
        let _ = SetTextColor(hdc, old_color);
        result
    }
}

// ============ Window frame with shadow ============

/// Draw the outer window shape: shadow + rounded frame + title bar.
pub fn draw_window_frame(hdc: HDC, theme: &Theme, title: &str, width: i32, height: i32) {
    use crate::theme::LAYOUT;
    use crate::dpi::s;

    let radius = s(LAYOUT.window_radius);
    let title_h = s(LAYOUT.title_bar_h);

    let inner = make_rect(0, 0, width, height);

    unsafe {
        // Main window body
        fill_rounded_rect(hdc, &inner, radius, theme.brush_bg);

        // Single border
        let null_brush = GetStockObject(NULL_BRUSH);
        let old_brush = SelectObject(hdc, null_brush);
        let old_pen = SelectObject(hdc, theme.pen_window);
        let _ = RoundRect(hdc, inner.left, inner.top, inner.right, inner.bottom, radius, radius);
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);

        // Title bar area (top portion)
        let title_rc = make_rect(inner.left, inner.top, inner.right, inner.top + title_h);
        fill_rounded_rect(hdc, &title_rc, radius, theme.brush_bg_light);
        // Clip bottom corners by filling a solid rect below the rounded top
        let title_solid = make_rect(inner.left, inner.top + radius / 2, inner.right, inner.top + title_h);
        let _ = FillRect(hdc, &title_solid, theme.brush_bg_light);

        // Title text
        let mut title_text_rc = title_rc;
        title_text_rc.left += s(16);
        draw_text(
            hdc, title, &title_text_rc,
            theme.font_body_bold, theme.colors.text_bright,
            DT_VCENTER | DT_SINGLELINE | DT_LEFT,
        );

        // Close button (X)
        let close_size = s(LAYOUT.close_btn_size);
        let close_off = s(LAYOUT.close_btn_offset);
        let close_x = inner.right - close_size - close_off;
        let close_y = inner.top + close_off;
        let close_rc = make_rect(close_x, close_y, close_x + close_size, close_y + close_size);

        let old_pen = SelectObject(hdc, theme.pen_close);
        let cx = close_x + close_size / 2;
        let cy = close_y + close_size / 2;
        let half = close_size / 3;
        let _ = MoveToEx(hdc, cx - half, cy - half, None);
        let _ = LineTo(hdc, cx + half, cy + half);
        let _ = MoveToEx(hdc, cx + half, cy - half, None);
        let _ = LineTo(hdc, cx - half, cy + half);
        let _ = SelectObject(hdc, old_pen);
        // suppress unused warning
        let _ = close_rc;
    }
}

/// Get the inner content rect (inside title bar + padding).
pub fn content_rect(width: i32, height: i32) -> RECT {
    use crate::theme::LAYOUT;
    use crate::dpi::s;

    let title_h = s(LAYOUT.title_bar_h);
    let pad = LAYOUT.pad();
    make_rect(
        pad,
        title_h + pad / 2,
        width - pad,
        height - pad,
    )
}

/// Returns the close button rect (for hit testing).
pub fn close_button_rect(width: i32) -> RECT {
    use crate::theme::LAYOUT;
    use crate::dpi::s;

    let close_size = s(LAYOUT.close_btn_size);
    let close_off = s(LAYOUT.close_btn_offset);
    let close_x = width - close_size - close_off;
    let close_y = close_off;
    make_rect(close_x, close_y, close_x + close_size, close_y + close_size)
}

/// Returns the title bar drag rect.
pub fn title_bar_rect(width: i32) -> RECT {
    use crate::theme::LAYOUT;
    use crate::dpi::s;

    let title_h = s(LAYOUT.title_bar_h);
    make_rect(0, 0, width, title_h)
}

/// Returns the bottom button row rects (left + right button positions).
pub fn bottom_buttons(width: i32, height: i32) -> (RECT, RECT) {
    use crate::theme::LAYOUT;

    let pad = LAYOUT.pad();
    let bw = LAYOUT.btn_w();
    let bh = LAYOUT.btn_h();
    let by = height - pad - bh;
    let left = make_rect(pad, by, pad + bw, by + bh);
    let right = make_rect(width - pad - bw, by, width - pad, by + bh);
    (left, right)
}
