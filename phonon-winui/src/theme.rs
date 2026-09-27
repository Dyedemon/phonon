//! Theme: color palette and layout constants.
//!
//! With the Direct2D backend there are no cached GDI handles — colors are
//! plain COLORREF values applied to the canvas brush on demand, and text
//! styles are [`FontStyle`] variants rendered via DirectWrite.

use crate::dpi::s;

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
    bg: 0x00140A0A,
    bg_light: 0x001E1010,
    card: 0x001D1111,
    card_hover: 0x002D1A1A,
    card_selected: 0x00281616,
    border: 0x003A2020,
    border_glow: 0x00803C3C,
    window_border: 0x004A2828,
    text: 0x00B0A090,
    text_bright: 0x00F0E8E0,
    text_dim: 0x00706060,
    accent: 0x00FFD44A,
    accent2: 0x00FF8888,
    accent_soft: 0x00A08C5A,
    accent_glow: 0x00FFE880,
    danger: 0x006650FF,
    danger_hover: 0x008060FF,
    danger_soft: 0x00442088,
    success: 0x0060E848,
    success_soft: 0x00307020,
    white: 0x00FFFFFF,
    close_hover: 0x003018F0,
    header_grad1: 0x00381414,
    header_grad2: 0x00140A0A,
    icon_bg: 0x002A1515,
    icon_bg_selected: 0x003A2020,
};

/// The theme passed to pages: palette + layout. Rendering resources live in
/// the Direct2D `Canvas`.
#[derive(Default)]
pub struct Theme;

impl Theme {
    pub fn new() -> Self {
        Self
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

/// Drop-shadow width (logical px) — how far the soft shadow extends beyond
/// the window body on each side.
pub const SHADOW_SIZE: i32 = 4;

impl Layout {
    #[inline]
    pub fn sw(&self) -> i32 {
        s(self.window_width)
    }
    #[inline]
    pub fn sh(&self) -> i32 {
        s(self.window_height)
    }
    #[inline]
    pub fn pad(&self) -> i32 {
        s(self.padding)
    }
    #[inline]
    pub fn btn_w(&self) -> i32 {
        s(self.button_w)
    }
    #[inline]
    pub fn btn_h(&self) -> i32 {
        s(self.button_h)
    }
    #[inline]
    pub fn shadow(&self) -> i32 {
        s(self.shadow_size)
    }
}
