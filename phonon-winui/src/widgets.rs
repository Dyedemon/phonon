//! Reusable UI widgets.
//!
//! All widgets share a common pattern: they own their position/size and
//! drawing state (hover, pressed, etc.), and expose `paint`, `hit_test`,
//! and event handler methods.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;

use crate::gdi::{draw_rounded_rect, draw_text, fill_rounded_rect, point_in_rect};
use crate::theme::Theme;
use crate::dpi::s;

/// Visual style of a button.
#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Danger,
    Success,
}

/// A push-button widget.
pub struct Button {
    pub rect: RECT,
    pub text: String,
    pub style: ButtonStyle,
    pub hover: bool,
    pub pressed: bool,
    pub enabled: bool,
    pub radius: i32,
}

impl Button {
    pub fn new(text: impl Into<String>, style: ButtonStyle) -> Self {
        Self {
            rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            text: text.into(),
            style,
            hover: false,
            pressed: false,
            enabled: true,
            radius: 20,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.enabled && point_in_rect(x, y, &self.rect)
    }

    pub fn paint(&self, hdc: HDC, theme: &Theme) {
        use crate::theme::COLORS;

        let (brush, pen, text_color): (HBRUSH, HPEN, u32) = match self.style {
            ButtonStyle::Primary => {
                if !self.enabled {
                    (theme.brush_accent_soft, theme.pen_border, COLORS.text_dim)
                } else if self.pressed {
                    (theme.brush_accent_glow, theme.pen_accent_glow, 0x00010101)
                } else if self.hover {
                    (theme.brush_accent, theme.pen_accent_glow, 0x00010101)
                } else {
                    (theme.brush_accent, theme.pen_accent, 0x00010101)
                }
            }
            ButtonStyle::Secondary => {
                if !self.enabled {
                    (theme.brush_card, theme.pen_border, COLORS.text_dim)
                } else if self.pressed {
                    (theme.brush_card_selected, theme.pen_border_glow, COLORS.text_bright)
                } else if self.hover {
                    (theme.brush_card_hover, theme.pen_border, COLORS.text_bright)
                } else {
                    (theme.brush_card, theme.pen_border, COLORS.text)
                }
            }
            ButtonStyle::Danger => {
                if !self.enabled {
                    (theme.brush_danger_soft, theme.pen_border, COLORS.text_dim)
                } else if self.pressed {
                    (theme.brush_danger, theme.pen_danger, COLORS.white)
                } else if self.hover {
                    (theme.brush_danger, theme.pen_danger, COLORS.white)
                } else {
                    (theme.brush_danger_soft, theme.pen_danger, COLORS.white)
                }
            }
            ButtonStyle::Success => {
                if !self.enabled {
                    (theme.brush_success_soft, theme.pen_border, COLORS.text_dim)
                } else if self.pressed {
                    (theme.brush_success, theme.pen_success, COLORS.white)
                } else if self.hover {
                    (theme.brush_success, theme.pen_success, COLORS.white)
                } else {
                    (theme.brush_success_soft, theme.pen_success, COLORS.white)
                }
            }
        };

        let r = s(self.radius);
        draw_rounded_rect(hdc, &self.rect, r, brush, pen);

        draw_text(
            hdc, &self.text, &self.rect,
            theme.font_body_bold, text_color,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

/// A checkbox widget.
pub struct Checkbox {
    pub rect: RECT,
    pub text: String,
    pub checked: bool,
    pub hover: bool,
    pub enabled: bool,
}

impl Checkbox {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            text: text.into(),
            checked: false,
            hover: false,
            enabled: true,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.enabled && point_in_rect(x, y, &self.rect)
    }

    pub fn paint(&self, hdc: HDC, theme: &Theme) {
        use crate::theme::COLORS;

        let box_size = s(16);
        let box_y = self.rect.top + ((self.rect.bottom - self.rect.top) - box_size) / 2;
        let box_rc = RECT {
            left: self.rect.left,
            top: box_y,
            right: self.rect.left + box_size,
            bottom: box_y + box_size,
        };

        let (brush, pen) = if self.checked {
            (theme.brush_accent, theme.pen_accent)
        } else if self.hover {
            (theme.brush_card_hover, theme.pen_border_glow)
        } else {
            (theme.brush_card, theme.pen_border)
        };

        draw_rounded_rect(hdc, &box_rc, s(4), brush, pen);

        // Check mark (V shape)
        if self.checked {
            unsafe {
                let old_pen = SelectObject(hdc, theme.pen_close);
                let cx = box_rc.left + box_size / 2;
                let cy = box_rc.top + box_size / 2;
                let half = box_size / 3;
                let _ = MoveToEx(hdc, cx - half, cy, None);
                let _ = LineTo(hdc, cx - half / 2, cy + half / 2);
                let _ = LineTo(hdc, cx + half, cy - half / 2);
                let _ = SelectObject(hdc, old_pen);
            }
        }

        // Text
        let mut text_rc = self.rect;
        text_rc.left += box_size + s(8);
        let text_color = if self.enabled { COLORS.text } else { COLORS.text_dim };
        draw_text(
            hdc, &self.text, &text_rc,
            theme.font_body, text_color,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

/// A horizontal progress bar.
pub struct ProgressBar {
    pub rect: RECT,
    pub percent: f32,
}

impl ProgressBar {
    pub fn new() -> Self {
        Self {
            rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            percent: 0.0,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn set_percent(&mut self, p: f32) {
        self.percent = p.clamp(0.0, 1.0);
    }

    pub fn paint(&self, hdc: HDC, theme: &Theme) {
        let radius = s(6);

        // Background track
        fill_rounded_rect(hdc, &self.rect, radius, theme.brush_card);

        // Filled portion — clip to rounded rect so both ends stay rounded
        let w = self.rect.right - self.rect.left;
        let fill_w = (w as f32 * self.percent) as i32;
        if fill_w > 0 {
            unsafe {
                let hrgn = CreateRoundRectRgn(
                    self.rect.left, self.rect.top,
                    self.rect.right + 1, self.rect.bottom + 1,
                    radius * 2, radius * 2,
                );
                if !hrgn.is_invalid() {
                    let _ = SelectClipRgn(hdc, hrgn);

                    let mut fill_rc = self.rect;
                    fill_rc.right = fill_rc.left + fill_w;
                    let _ = FillRect(hdc, &fill_rc, theme.brush_accent);

                    let _ = SelectClipRgn(hdc, HRGN::default());
                    let _ = DeleteObject(hrgn);
                }
            }
        }

        // Border
        unsafe {
            let null_brush = GetStockObject(NULL_BRUSH);
            let old_brush = SelectObject(hdc, null_brush);
            let old_pen = SelectObject(hdc, theme.pen_border);
            let _ = RoundRect(
                hdc,
                self.rect.left, self.rect.top,
                self.rect.right, self.rect.bottom,
                radius, radius,
            );
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
        }
    }
}

/// A selectable card (radio-button style).
pub struct RadioCard {
    pub rect: RECT,
    pub title: String,
    pub description: String,
    pub selected: bool,
    pub hover: bool,
}

impl RadioCard {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            title: title.into(),
            description: description.into(),
            selected: false,
            hover: false,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        point_in_rect(x, y, &self.rect)
    }

    pub fn paint(&self, hdc: HDC, theme: &Theme) {
        use crate::theme::COLORS;
        use crate::dpi::s;

        let r = s(12);
        let (brush, pen) = if self.selected {
            (theme.brush_card_selected, theme.pen_accent)
        } else if self.hover {
            (theme.brush_card_hover, theme.pen_border_glow)
        } else {
            (theme.brush_card, theme.pen_border)
        };

        draw_rounded_rect(hdc, &self.rect, r, brush, pen);

        // Icon circle (left side)
        let icon_size = s(40);
        let icon_x = self.rect.left + s(16);
        let icon_y = self.rect.top + (self.rect.bottom - self.rect.top - icon_size) / 2;
        let icon_rc = RECT {
            left: icon_x,
            top: icon_y,
            right: icon_x + icon_size,
            bottom: icon_y + icon_size,
        };
        let icon_brush = if self.selected {
            theme.brush_icon_bg_selected
        } else {
            theme.brush_icon_bg
        };
        fill_rounded_rect(hdc, &icon_rc, s(20), icon_brush);

        // Title
        let mut title_rc = self.rect;
        title_rc.left = icon_x + icon_size + s(12);
        title_rc.top += s(10);
        title_rc.right -= s(16);
        draw_text(
            hdc, &self.title, &title_rc,
            theme.font_body_bold, COLORS.text_bright,
            DT_LEFT | DT_TOP | DT_SINGLELINE,
        );

        // Description
        let mut desc_rc = title_rc;
        desc_rc.top += s(20);
        desc_rc.bottom = self.rect.bottom - s(10);
        draw_text(
            hdc, &self.description, &desc_rc,
            theme.font_small, COLORS.text_dim,
            DT_LEFT | DT_TOP | DT_WORDBREAK,
        );
    }
}

/// A simple static text label.
pub struct Label {
    pub rect: RECT,
    pub text: String,
    pub font: LabelFont,
    pub color: LabelColor,
    pub align: LabelAlign,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LabelFont {
    Big,
    Title,
    Body,
    BodyBold,
    Small,
    Micro,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LabelColor {
    Bright,
    Normal,
    Dim,
    Accent,
    Success,
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LabelAlign {
    Left,
    Center,
    Right,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            text: text.into(),
            font: LabelFont::Body,
            color: LabelColor::Normal,
            align: LabelAlign::Left,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn paint(&self, hdc: HDC, theme: &Theme) {
        use crate::theme::COLORS;

        let font = match self.font {
            LabelFont::Big => theme.font_big,
            LabelFont::Title => theme.font_title,
            LabelFont::Body => theme.font_body,
            LabelFont::BodyBold => theme.font_body_bold,
            LabelFont::Small => theme.font_small,
            LabelFont::Micro => theme.font_micro,
        };

        let color = match self.color {
            LabelColor::Bright => COLORS.text_bright,
            LabelColor::Normal => COLORS.text,
            LabelColor::Dim => COLORS.text_dim,
            LabelColor::Accent => COLORS.accent,
            LabelColor::Success => COLORS.success,
            LabelColor::Danger => COLORS.danger,
        };

        let align_flags = match self.align {
            LabelAlign::Left => DT_LEFT,
            LabelAlign::Center => DT_CENTER,
            LabelAlign::Right => DT_RIGHT,
        };

        draw_text(hdc, &self.text, &self.rect, font, color, align_flags | DT_SINGLELINE);
    }
}
