//! Reusable UI widgets (Direct2D rendering).
//!
//! All widgets share a common pattern: they own their position/size and
//! drawing state (hover, pressed, etc.), and expose `paint`, `hit_test`,
//! and event handler methods.

use windows::Win32::Foundation::RECT;

use crate::d2d::{tf, Canvas, FontStyle};
use crate::dpi::s;
use crate::gdi::{make_rect, point_in_rect};
use crate::theme::{Theme, COLORS};

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
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
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

    pub fn paint(&self, canvas: &mut Canvas, _theme: &Theme) {
        let c = &COLORS;

        // (fill, stroke, text color)
        let (fill, stroke, text_color): (u32, u32, u32) = match self.style {
            ButtonStyle::Primary => {
                if !self.enabled {
                    (c.accent_soft, c.border, c.text_dim)
                } else if self.pressed {
                    (c.accent_glow, c.accent_glow, 0x00010101)
                } else if self.hover {
                    (c.accent_glow, c.accent_glow, 0x00010101)
                } else {
                    (c.accent, c.accent, 0x00010101)
                }
            }
            ButtonStyle::Secondary => {
                if !self.enabled {
                    (c.card, c.border, c.text_dim)
                } else if self.pressed {
                    (c.card_selected, c.border_glow, c.text_bright)
                } else if self.hover {
                    (c.card_hover, c.border, c.text_bright)
                } else {
                    (c.card, c.border, c.text)
                }
            }
            ButtonStyle::Danger => {
                if !self.enabled {
                    (c.danger_soft, c.border, c.text_dim)
                } else if self.pressed || self.hover {
                    (c.danger, c.danger, c.white)
                } else {
                    (c.danger_soft, c.danger, c.white)
                }
            }
            ButtonStyle::Success => {
                if !self.enabled {
                    (c.success_soft, c.border, c.text_dim)
                } else if self.pressed || self.hover {
                    (c.success, c.success, c.white)
                } else {
                    (c.success_soft, c.success, c.white)
                }
            }
        };

        let r = s(self.radius);
        canvas.fill_round(&self.rect, r, fill);
        canvas.stroke_round(&self.rect, r, stroke, 1.0);

        canvas.text(
            &self.text,
            &self.rect,
            FontStyle::BodyBold,
            text_color,
            tf::CENTER | tf::VCENTER | tf::SINGLELINE,
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
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
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

    pub fn paint(&self, canvas: &mut Canvas, _theme: &Theme) {
        let c = &COLORS;

        let box_size = s(16);
        let box_y = self.rect.top + ((self.rect.bottom - self.rect.top) - box_size) / 2;
        let box_rc = make_rect(
            self.rect.left,
            box_y,
            self.rect.left + box_size,
            box_y + box_size,
        );

        let (fill, stroke) = if self.checked {
            (c.accent, c.accent)
        } else if self.hover {
            (c.card_hover, c.border_glow)
        } else {
            (c.card, c.border)
        };

        canvas.fill_round(&box_rc, s(4), fill);
        canvas.stroke_round(&box_rc, s(4), stroke, 1.0);

        // Check mark (V shape)
        if self.checked {
            let cx = box_rc.left + box_size / 2;
            let cy = box_rc.top + box_size / 2;
            let half = box_size / 3;
            canvas.line(cx - half, cy, cx - half / 2, cy + half / 2, 0x00010101, 2.0);
            canvas.line(
                cx - half / 2,
                cy + half / 2,
                cx + half,
                cy - half / 2,
                0x00010101,
                2.0,
            );
        }

        // Text
        let mut text_rc = self.rect;
        text_rc.left += box_size + s(8);
        let text_color = if self.enabled { c.text } else { c.text_dim };
        canvas.text(
            &self.text,
            &text_rc,
            FontStyle::Body,
            text_color,
            tf::LEFT | tf::VCENTER | tf::SINGLELINE,
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
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            percent: 0.0,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn set_percent(&mut self, p: f32) {
        self.percent = p.clamp(0.0, 1.0);
    }

    pub fn paint(&self, canvas: &mut Canvas, _theme: &Theme) {
        let c = &COLORS;
        let radius = s(6);

        // Background track
        canvas.fill_round(&self.rect, radius, c.card);
        canvas.stroke_round(&self.rect, radius, c.border, 1.0);

        // Filled portion — clipped to the rounded track so both ends stay
        // rounded (D2D clip replaces the old GDI region dance).
        let w = self.rect.right - self.rect.left;
        let fill_w = (w as f32 * self.percent) as i32;
        if fill_w > 0 {
            let fill_rc = make_rect(
                self.rect.left,
                self.rect.top,
                self.rect.left + fill_w,
                self.rect.bottom,
            );
            canvas.push_clip(&self.rect);
            canvas.fill_gradient_v(&fill_rc, c.accent_glow, c.accent);
            canvas.pop_clip();
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
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
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

    pub fn paint(&self, canvas: &mut Canvas, _theme: &Theme) {
        let c = &COLORS;

        let r = s(12);
        let (fill, stroke) = if self.selected {
            (c.card_selected, c.accent)
        } else if self.hover {
            (c.card_hover, c.border_glow)
        } else {
            (c.card, c.border)
        };

        canvas.fill_round(&self.rect, r, fill);
        canvas.stroke_round(&self.rect, r, stroke, if self.selected { 1.4 } else { 1.0 });

        // Icon circle (left side)
        let icon_size = s(40);
        let icon_x = self.rect.left + s(16);
        let icon_y = self.rect.top + (self.rect.bottom - self.rect.top - icon_size) / 2;
        let icon_fill = if self.selected {
            c.icon_bg_selected
        } else {
            c.icon_bg
        };
        canvas.fill_ellipse(
            icon_x + icon_size / 2,
            icon_y + icon_size / 2,
            icon_size / 2,
            icon_size / 2,
            icon_fill,
        );

        // Title
        let mut title_rc = self.rect;
        title_rc.left = icon_x + icon_size + s(12);
        title_rc.top += s(10);
        title_rc.right -= s(16);
        canvas.text(
            &self.title,
            &title_rc,
            FontStyle::BodyBold,
            c.text_bright,
            tf::LEFT | tf::TOP | tf::SINGLELINE,
        );

        // Description
        let mut desc_rc = title_rc;
        desc_rc.top += s(20);
        desc_rc.bottom = self.rect.bottom - s(10);
        canvas.text(
            &self.description,
            &desc_rc,
            FontStyle::Small,
            c.text_dim,
            tf::LEFT | tf::TOP | tf::WORDBREAK,
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
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            text: text.into(),
            font: LabelFont::Body,
            color: LabelColor::Normal,
            align: LabelAlign::Left,
        }
    }

    pub fn set_pos(&mut self, rc: RECT) {
        self.rect = rc;
    }

    pub fn paint(&self, canvas: &mut Canvas, _theme: &Theme) {
        let c = &COLORS;

        let font = match self.font {
            LabelFont::Big => FontStyle::Big,
            LabelFont::Title => FontStyle::Title,
            LabelFont::Body => FontStyle::Body,
            LabelFont::BodyBold => FontStyle::BodyBold,
            LabelFont::Small => FontStyle::Small,
            LabelFont::Micro => FontStyle::Micro,
        };

        let color = match self.color {
            LabelColor::Bright => c.text_bright,
            LabelColor::Normal => c.text,
            LabelColor::Dim => c.text_dim,
            LabelColor::Accent => c.accent,
            LabelColor::Success => c.success,
            LabelColor::Danger => c.danger,
        };

        let align = match self.align {
            LabelAlign::Left => tf::LEFT,
            LabelAlign::Center => tf::CENTER,
            LabelAlign::Right => tf::RIGHT,
        };

        canvas.text(&self.text, &self.rect, font, color, align | tf::SINGLELINE);
    }
}
