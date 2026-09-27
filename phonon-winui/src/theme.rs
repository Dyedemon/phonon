//! Theme: colors, fonts, brushes, pens, and layout constants.

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DeleteObject,
    HBRUSH, HFONT, HPEN, PS_SOLID,
};
use windows::core::w;

use crate::dpi::{get_dpi, s};

/// Color palette (COLORREF = 0x00BBGGRR).
#[derive(Clone, Copy)]
pub struct Colors {
    pub bg: u32,
    pub bg_light: u32,
    pub card: u32,
    pub card_hover: u32,
    pub card_selected: u32,
    pub border: u32,
    pub border_glow: u32,
    pub window_border: u32,
    pub text: u32,
    pub text_bright: u32,
    pub text_dim: u32,
    pub accent: u32,
    pub accent2: u32,
    pub accent_soft: u32,
    pub accent_glow: u32,
    pub danger: u32,
    pub danger_hover: u32,
    pub danger_soft: u32,
    pub success: u32,
    pub success_soft: u32,
    pub white: u32,
    pub close_hover: u32,
    pub header_grad1: u32,
    pub header_grad2: u32,
    pub icon_bg: u32,
    pub icon_bg_selected: u32,
}

pub const COLORS: Colors = Colors {
    bg:             0x00140A0A,
    bg_light:       0x001E1010,
    card:           0x001D1111,
    card_hover:     0x002D1A1A,
    card_selected:  0x00281616,
    border:         0x003A2020,
    border_glow:    0x00803C3C,
    window_border:  0x004A2828,
    text:           0x00B0A090,
    text_bright:    0x00F0E8E0,
    text_dim:       0x00706060,
    accent:         0x00FFD44A,
    accent2:        0x00FF8888,
    accent_soft:    0x00A08C5A,
    accent_glow:    0x00FFE880,
    danger:         0x006650FF,
    danger_hover:   0x008060FF,
    danger_soft:    0x00442088,
    success:        0x0060E848,
    success_soft:   0x00307020,
    white:          0x00FFFFFF,
    close_hover:    0x003018F0,
    header_grad1:   0x00381414,
    header_grad2:   0x00140A0A,
    icon_bg:        0x002A1515,
    icon_bg_selected: 0x003A2020,
};

/// Pre-created GDI resources.
pub struct Theme {
    pub colors: Colors,
    pub font_big: HFONT,
    pub font_title: HFONT,
    pub font_body: HFONT,
    pub font_body_bold: HFONT,
    pub font_small: HFONT,
    pub font_micro: HFONT,

    pub brush_bg: HBRUSH,
    pub brush_bg_light: HBRUSH,
    pub brush_card: HBRUSH,
    pub brush_card_hover: HBRUSH,
    pub brush_card_selected: HBRUSH,
    pub brush_accent: HBRUSH,
    pub brush_accent_soft: HBRUSH,
    pub brush_accent_glow: HBRUSH,
    pub brush_success: HBRUSH,
    pub brush_success_soft: HBRUSH,
    pub brush_white: HBRUSH,
    pub brush_icon_bg: HBRUSH,
    pub brush_icon_bg_selected: HBRUSH,
    pub brush_danger: HBRUSH,
    pub brush_danger_soft: HBRUSH,

    pub pen_border: HPEN,
    pub pen_border_glow: HPEN,
    pub pen_accent: HPEN,
    pub pen_accent_glow: HPEN,
    pub pen_close: HPEN,
    pub pen_success: HPEN,
    pub pen_window: HPEN,
    pub pen_danger: HPEN,
}

fn make_font(pt_size: i32, weight: i32) -> HFONT {
    let dpi = get_dpi();
    let h = -pt_size * dpi / 72;
    unsafe {
        // CreateFontW: all params after cweight are u32/bool
        CreateFontW(
            h,              // nHeight
            0,              // nWidth
            0,              // nEscapement
            0,              // nOrientation
            weight,         // fnWeight (FW_NORMAL=400, FW_BOLD=700)
            0,              // fdwItalic
            0,              // fdwUnderline
            0,              // fdwStrikeOut
            0,              // fdwCharSet (DEFAULT_CHARSET=1)
            0,              // fdwOutputPrecision
            0,              // fdwClipPrecision
            5,              // fdwQuality (CLEARTYPE_QUALITY=5)
            0 | 32,         // fdwPitchAndFamily (DEFAULT_PITCH=0 | FF_SWISS=32)
            w!("Segoe UI"),
        )
    }
}

impl Theme {
    pub fn new() -> Self {
        let c = COLORS;
        unsafe {
            Self {
                colors: c,
                font_big: make_font(20, 700),
                font_title: make_font(13, 600),
                font_body: make_font(11, 400),
                font_body_bold: make_font(11, 600),
                font_small: make_font(9, 400),
                font_micro: make_font(8, 400),

                brush_bg: CreateSolidBrush(COLORREF(c.bg)),
                brush_bg_light: CreateSolidBrush(COLORREF(c.bg_light)),
                brush_card: CreateSolidBrush(COLORREF(c.card)),
                brush_card_hover: CreateSolidBrush(COLORREF(c.card_hover)),
                brush_card_selected: CreateSolidBrush(COLORREF(c.card_selected)),
                brush_accent: CreateSolidBrush(COLORREF(c.accent)),
                brush_accent_soft: CreateSolidBrush(COLORREF(c.accent_soft)),
                brush_accent_glow: CreateSolidBrush(COLORREF(c.accent_glow)),
                brush_success: CreateSolidBrush(COLORREF(c.success)),
                brush_success_soft: CreateSolidBrush(COLORREF(c.success_soft)),
                brush_white: CreateSolidBrush(COLORREF(c.white)),
                brush_icon_bg: CreateSolidBrush(COLORREF(c.icon_bg)),
                brush_icon_bg_selected: CreateSolidBrush(COLORREF(c.icon_bg_selected)),
                brush_danger: CreateSolidBrush(COLORREF(c.danger)),
                brush_danger_soft: CreateSolidBrush(COLORREF(c.danger_soft)),

                pen_border: CreatePen(PS_SOLID, 1, COLORREF(c.border)),
                pen_border_glow: CreatePen(PS_SOLID, 1, COLORREF(c.border_glow)),
                pen_accent: CreatePen(PS_SOLID, 1, COLORREF(c.accent)),
                pen_accent_glow: CreatePen(PS_SOLID, 1, COLORREF(c.accent_glow)),
                pen_close: CreatePen(PS_SOLID, 1, COLORREF(c.text_dim)),
                pen_success: CreatePen(PS_SOLID, 1, COLORREF(c.success)),
                pen_window: CreatePen(PS_SOLID, 1, COLORREF(c.window_border)),
                pen_danger: CreatePen(PS_SOLID, 1, COLORREF(c.danger)),
            }
        }
    }

    pub fn destroy(&self) {
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
            let _ = DeleteObject(self.brush_accent_soft);
            let _ = DeleteObject(self.brush_accent_glow);
            let _ = DeleteObject(self.brush_success);
            let _ = DeleteObject(self.brush_success_soft);
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

impl Default for Theme {
    fn default() -> Self {
        Self::new()
    }
}

/// Standard layout constants (in logical pixels @ 96 DPI).
pub struct Layout {
    pub window_width: i32,
    pub window_height: i32,
    pub shadow_size: i32,
    pub window_radius: i32,
    pub title_bar_h: i32,
    pub padding: i32,
    pub button_w: i32,
    pub button_h: i32,
    pub button_radius: i32,
    pub card_radius: i32,
    pub close_btn_size: i32,
    pub close_btn_offset: i32,
}

pub const LAYOUT: Layout = Layout {
 window_width: 600,
 window_height: 560,
 shadow_size: 4,
 window_radius: 18,
 title_bar_h: 36,
 padding: 28,
 button_w: 120,
 button_h: 40,
 button_radius: 20,
 card_radius: 12,
 close_btn_size: 32,
 close_btn_offset: 4,
};

impl Layout {
    #[inline]
    pub fn sw(&self) -> i32 { s(self.window_width) }
    #[inline]
    pub fn sh(&self) -> i32 { s(self.window_height) }
    #[inline]
    pub fn pad(&self) -> i32 { s(self.padding) }
    #[inline]
    pub fn btn_w(&self) -> i32 { s(self.button_w) }
    #[inline]
    pub fn btn_h(&self) -> i32 { s(self.button_h) }
    #[inline]
    pub fn shadow(&self) -> i32 { s(self.shadow_size) }
}
