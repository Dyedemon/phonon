//! Geometry and mouse helpers.
//!
//! Drawing itself lives in [`crate::d2d::Canvas`] (Direct2D + DirectWrite);
//! these are the coordinate utilities and window-chrome painter shared by
//! widgets and pages.

use windows::Win32::Foundation::RECT;

use crate::d2d::Canvas;
use crate::dpi::s;
use crate::theme::{COLORS, LAYOUT};

/// Create a RECT from left/top/right/bottom.
#[inline]
pub fn make_rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

/// Point-in-rect test.
#[inline]
pub fn point_in_rect(x: i32, y: i32, r: &RECT) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Extract X coordinate from LPARAM (client-area mouse message).
#[inline]
pub fn lparam_x(lp: isize) -> i32 {
    (lp as u32 & 0xFFFF) as i16 as i32
}

/// Extract Y coordinate from LPARAM.
#[inline]
pub fn lparam_y(lp: isize) -> i32 {
    ((lp as u32) >> 16 & 0xFFFF) as i16 as i32
}

/// Convert a Rust string to a null-terminated UTF-16 Vec.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Draw the outer window shape: rounded frame + gradient title bar + close X.
pub fn draw_window_frame(canvas: &mut Canvas, title: &str, width: i32, height: i32) {
    let radius = s(LAYOUT.window_radius);
    let title_h = s(LAYOUT.title_bar_h);

    let inner = make_rect(0, 0, width, height);

    // Main window body — subtle vertical gradient for depth.
    canvas.fill_round(&inner, radius, COLORS.bg);
    canvas.push_clip(&inner);
    canvas.fill_gradient_v(
        &make_rect(0, 0, width, title_h + radius),
        COLORS.header_grad1,
        COLORS.header_grad2,
    );
    canvas.pop_clip();

    // Single border
    canvas.stroke_round(&inner, radius, COLORS.window_border, 1.0);

    // Title text
    let title_rc = make_rect(s(16), 0, width - s(60), title_h);
    canvas.text(
        title,
        &title_rc,
        crate::d2d::FontStyle::BodyBold,
        COLORS.text_bright,
        crate::d2d::tf::LEFT | crate::d2d::tf::VCENTER | crate::d2d::tf::SINGLELINE,
    );

    // Close button (X)
    let close_size = s(LAYOUT.close_btn_size);
    let close_off = s(LAYOUT.close_btn_offset);
    let cx = inner.right - close_size / 2 - close_off;
    let cy = inner.top + close_size / 2 + close_off;
    let half = close_size / 3;
    canvas.line(
        cx - half,
        cy - half,
        cx + half,
        cy + half,
        COLORS.text_dim,
        1.4,
    );
    canvas.line(
        cx + half,
        cy - half,
        cx - half,
        cy + half,
        COLORS.text_dim,
        1.4,
    );
}

/// Get the inner content rect (inside title bar + padding).
pub fn content_rect(width: i32, height: i32) -> RECT {
    let title_h = s(LAYOUT.title_bar_h);
    let pad = LAYOUT.pad();
    make_rect(pad, title_h + pad / 2, width - pad, height - pad)
}

/// Returns the close button rect (for hit testing).
pub fn close_button_rect(width: i32) -> RECT {
    let close_size = s(LAYOUT.close_btn_size);
    let close_off = s(LAYOUT.close_btn_offset);
    let close_x = width - close_size - close_off;
    let close_y = close_off;
    make_rect(close_x, close_y, close_x + close_size, close_y + close_size)
}

/// Returns the title bar drag rect.
pub fn title_bar_rect(width: i32) -> RECT {
    let title_h = s(LAYOUT.title_bar_h);
    make_rect(0, 0, width, title_h)
}

/// Returns the bottom button row rects (left + right button positions).
pub fn bottom_buttons(width: i32, height: i32) -> (RECT, RECT) {
    let pad = LAYOUT.pad();
    let bw = LAYOUT.btn_w();
    let bh = LAYOUT.btn_h();
    let by = height - pad - bh;
    let left = make_rect(pad, by, pad + bw, by + bh);
    let right = make_rect(width - pad - bw, by, width - pad, by + bh);
    (left, right)
}
