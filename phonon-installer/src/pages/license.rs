//! License agreement page.

use phonon_winui::window::{Navigator, Page};
use phonon_winui::widgets::*;
use phonon_winui::theme::Theme;
use phonon_winui::dpi::s;
use phonon_winui::gdi::*;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    DrawTextW, GetDC, IntersectClipRect, SelectObject, SetTextColor, SetBkMode,
    HDC, DT_CALCRECT, DT_LEFT, DT_WORDBREAK,
    TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::pages::{install_dir::InstallDirPage, welcome::WelcomePage, AppContext};

const LICENSE_TEXT: &str = "MIT License

Copyright (c) 2026 Phonon Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.";

pub struct LicensePage {
    ctx: AppContext,
    btn_back: Button,
    btn_next: Button,
    btn_cancel: Button,
    lbl_title: Label,
    checkbox_accept: Checkbox,
    license_text: String,
    text_area: RECT,
    scroll_y: i32,
    text_height: i32,
}

impl LicensePage {
    pub fn new(ctx: AppContext) -> Self {
        let btn_back = Button::new("< 上一步", ButtonStyle::Secondary);
        let mut btn_next = Button::new("下一步 >", ButtonStyle::Primary);
        btn_next.enabled = false;
        let btn_cancel = Button::new("取消", ButtonStyle::Secondary);

        let mut lbl_title = Label::new("许可协议");
        lbl_title.font = LabelFont::Title;
        lbl_title.color = LabelColor::Bright;

        let checkbox_accept = Checkbox::new("我已阅读并同意上述许可协议");

        Self {
            ctx,
            btn_back,
            btn_next,
            btn_cancel,
            lbl_title,
            checkbox_accept,
            license_text: LICENSE_TEXT.to_string(),
            text_area: RECT::default(),
            scroll_y: 0,
            text_height: 0,
        }
    }

    fn max_scroll(&self) -> i32 {
        (self.text_height - (self.text_area.bottom - self.text_area.top)).max(0)
    }

    fn measure_text_height(&self, hdc: HDC, theme: &Theme) -> i32 {
        let w = self.text_area.right - self.text_area.left - s(24);
        if w <= 0 {
            return 0;
        }
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: 0,
        };
        let mut wtext: Vec<u16> = self.license_text.encode_utf16().collect();
        unsafe {
            let old_font = SelectObject(hdc, theme.font_small);
            let h = DrawTextW(
                hdc,
                &mut wtext,
                &mut rc as *mut RECT,
                DT_LEFT | DT_WORDBREAK | DT_CALCRECT,
            );
            let _ = SelectObject(hdc, old_font);
            h
        }
    }

    fn draw_scrollbar(&self, hdc: HDC, theme: &Theme) {
        let max = self.max_scroll();
        if max <= 0 {
            return;
        }

        let bar_w = s(4);
        let bar_x = self.text_area.right - bar_w - s(4);
        let bar_top = self.text_area.top + s(4);
        let bar_bot = self.text_area.bottom - s(4);
        let bar_h = bar_bot - bar_top;

        // Track
        unsafe {
            let track_brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(
                COLORREF(theme.colors.border),
            );
            let _ = windows::Win32::Graphics::Gdi::FillRect(
                hdc,
                &RECT {
                    left: bar_x,
                    top: bar_top,
                    right: bar_x + bar_w,
                    bottom: bar_bot,
                },
                track_brush,
            );
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(track_brush);
        }

        // Thumb
        let thumb_h = ((bar_h as f32) * (self.text_area.bottom - self.text_area.top) as f32
            / self.text_height as f32) as i32;
        let thumb_h = thumb_h.max(s(20));
        let thumb_y = bar_top + (bar_h - thumb_h) * self.scroll_y / max;

        unsafe {
            let thumb_brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(
                COLORREF(theme.colors.accent_soft),
            );
            let _ = windows::Win32::Graphics::Gdi::FillRect(
                hdc,
                &RECT {
                    left: bar_x,
                    top: thumb_y,
                    right: bar_x + bar_w,
                    bottom: thumb_y + thumb_h,
                },
                thumb_brush,
            );
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(thumb_brush);
        }
    }
}

impl Page for LicensePage {
    fn on_enter(&mut self, hwnd: HWND) {
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        let content = content_rect(rc.right - rc.left, rc.bottom - rc.top);

        // Title
        let mut title_rc = content.clone();
        title_rc.bottom = title_rc.top + s(24);
        self.lbl_title.set_pos(title_rc);

        // Checkbox (above buttons)
        let (left_btn, right_btn) = bottom_buttons(rc.right - rc.left, rc.bottom - rc.top);
        let mut cb_rc = content.clone();
        cb_rc.top = left_btn.top - s(40);
        cb_rc.bottom = cb_rc.top + s(24);
        self.checkbox_accept.set_pos(cb_rc);

        // License text area
        let mut text_rc = content.clone();
        text_rc.top += s(36);
        text_rc.bottom = self.checkbox_accept.rect.top - s(12);
        self.text_area = text_rc;

        // Buttons
        let back_rc = left_btn.clone();
        self.btn_back.set_pos(back_rc);

        let mut cancel_rc = left_btn.clone();
        cancel_rc.left += s(130);
        cancel_rc.right += s(130);
        self.btn_cancel.set_pos(cancel_rc);

        self.btn_next.set_pos(right_btn);

        // Measure text height
        unsafe {
            let hdc = GetDC(hwnd);
            // Need theme to measure — use a temporary Theme for measurement
            let theme = Theme::new();
            self.text_height = self.measure_text_height(hdc, &theme);
            theme.destroy();
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
        }
        self.scroll_y = 0;
    }

    fn paint(&mut self, hdc: HDC, _content_rc: &RECT, theme: &Theme) {
        self.lbl_title.paint(hdc, theme);

        // Background box for license text
        fill_rounded_rect(hdc, &self.text_area, s(8), theme.brush_bg_light);
        unsafe {
            let null_brush = windows::Win32::Graphics::Gdi::GetStockObject(
                windows::Win32::Graphics::Gdi::NULL_BRUSH,
            );
            let old_brush = windows::Win32::Graphics::Gdi::SelectObject(hdc, null_brush);
            let old_pen = windows::Win32::Graphics::Gdi::SelectObject(hdc, theme.pen_border);
            let _ = windows::Win32::Graphics::Gdi::RoundRect(
                hdc, self.text_area.left, self.text_area.top,
                self.text_area.right, self.text_area.bottom,
                s(8), s(8),
            );
            let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_brush);
            let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_pen);
        }

        // Clip to inner text area and draw scrolled text
        let inner_left = self.text_area.left + s(12);
        let inner_right = self.text_area.right - s(16);
        let inner_top = self.text_area.top + s(10);
        let inner_bot = self.text_area.bottom - s(10);

        unsafe {
            let _ = IntersectClipRect(hdc, inner_left, inner_top, inner_right, inner_bot);

            let mut text_rc = RECT {
                left: inner_left,
                top: inner_top - self.scroll_y,
                right: inner_right,
                bottom: inner_top + self.text_height,
            };

            let old_font = SelectObject(hdc, theme.font_small);
            let old_color = SetTextColor(hdc, COLORREF(0x00B0A090));
            let _ = SetBkMode(hdc, TRANSPARENT);
            let mut wtext: Vec<u16> = self.license_text.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut wtext,
                &mut text_rc as *mut RECT,
                DT_LEFT | DT_WORDBREAK,
            );
            let _ = SelectObject(hdc, old_font);
            let _ = SetTextColor(hdc, old_color);

            // Reset clip region
            let _ = windows::Win32::Graphics::Gdi::SelectClipRgn(hdc, windows::Win32::Graphics::Gdi::HRGN::default());
        }

        // Scrollbar
        self.draw_scrollbar(hdc, theme);

        self.checkbox_accept.paint(hdc, theme);
        self.btn_back.paint(hdc, theme);
        self.btn_cancel.paint(hdc, theme);
        self.btn_next.paint(hdc, theme);
    }

    fn on_mouse_move(&mut self, hwnd: HWND, x: i32, y: i32) {
        let mut needs_redraw = false;

        for btn in [&mut self.btn_back, &mut self.btn_next, &mut self.btn_cancel].iter_mut() {
            let hover = btn.hit_test(x, y);
            if hover != btn.hover {
                btn.hover = hover;
                needs_redraw = true;
            }
        }

        let hover_cb = self.checkbox_accept.hit_test(x, y);
        if hover_cb != self.checkbox_accept.hover {
            self.checkbox_accept.hover = hover_cb;
            needs_redraw = true;
        }

        if needs_redraw {
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_mouse_wheel(&mut self, hwnd: HWND, _x: i32, _y: i32, delta: i32) {
        let max = self.max_scroll();
        if max <= 0 {
            return;
        }
        let step = s(30);
        let new_scroll = if delta > 0 {
            self.scroll_y - step
        } else {
            self.scroll_y + step
        };
        let clamped = new_scroll.clamp(0, max);
        if clamped != self.scroll_y {
            self.scroll_y = clamped;
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_lbutton_down(&mut self, hwnd: HWND, x: i32, y: i32) {
        for btn in [&mut self.btn_back, &mut self.btn_next, &mut self.btn_cancel].iter_mut() {
            if btn.hit_test(x, y) {
                btn.pressed = true;
            }
        }
        phonon_winui::invalidate_window(hwnd);
    }

    fn on_lbutton_up(&mut self, hwnd: HWND, x: i32, y: i32) {
        let was_back = self.btn_back.pressed;
        let was_next = self.btn_next.pressed;
        let was_cancel = self.btn_cancel.pressed;

        self.btn_back.pressed = false;
        self.btn_next.pressed = false;
        self.btn_cancel.pressed = false;

        // Checkbox toggle
        if self.checkbox_accept.hit_test(x, y) {
            self.checkbox_accept.checked = !self.checkbox_accept.checked;
            self.btn_next.enabled = self.checkbox_accept.checked;
        }

        if was_back && self.btn_back.hit_test(x, y) {
            let page = WelcomePage::new(self.ctx.clone());
            Navigator::navigate(hwnd, Box::new(page));
            return;
        }

        if was_next && self.btn_next.hit_test(x, y) && self.btn_next.enabled {
            let page = InstallDirPage::new(self.ctx.clone());
            Navigator::navigate(hwnd, Box::new(page));
            return;
        }

        if was_cancel && self.btn_cancel.hit_test(x, y) {
            Navigator::close(hwnd);
            return;
        }

        phonon_winui::invalidate_window(hwnd);
    }
}
