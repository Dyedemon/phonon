//! Welcome page — first screen shown when installer starts.

use phonon_winui::window::{Navigator, Page};
use phonon_winui::widgets::*;
use phonon_winui::theme::Theme;
use phonon_winui::dpi::s;
use phonon_winui::gdi::*;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::installer::detect_existing_install;
use crate::pages::{license::LicensePage, AppContext};

pub struct WelcomePage {
    ctx: AppContext,
    btn_next: Button,
    btn_cancel: Button,
    lbl_title: Label,
    lbl_subtitle: Label,
    lbl_info: Label,
    existing_install: Option<String>,
}

impl WelcomePage {
    pub fn new(ctx: AppContext) -> Self {
        let existing = detect_existing_install();

        let btn_next = Button::new("下一步 >", ButtonStyle::Primary);
        let btn_cancel = Button::new("取消", ButtonStyle::Secondary);

        let mut lbl_title = Label::new("欢迎使用 Phonon 安装向导");
        lbl_title.font = LabelFont::Big;
        lbl_title.color = LabelColor::Bright;

        let mut lbl_subtitle = Label::new("纯净、高保真、可扩展的音频平台");
        lbl_subtitle.font = LabelFont::Body;
        lbl_subtitle.color = LabelColor::Dim;

        let info_text = if existing.is_some() {
            "检测到已安装的 Phonon，点击下一步更新或重新安装。"
        } else {
            "点击下一步继续安装，或取消退出安装向导。"
        };
        let mut lbl_info = Label::new(info_text);
        lbl_info.font = LabelFont::Body;
        lbl_info.color = LabelColor::Normal;

        Self {
            ctx,
            btn_next,
            btn_cancel,
            lbl_title,
            lbl_subtitle,
            lbl_info,
            existing_install: existing,
        }
    }
}

impl Page for WelcomePage {
    fn on_enter(&mut self, hwnd: HWND) {
        // Position widgets based on window size
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        let content = content_rect(rc.right - rc.left, rc.bottom - rc.top);

        // Title
        let mut title_rc = content.clone();
        title_rc.bottom = title_rc.top + s(32);
        self.lbl_title.set_pos(title_rc);

        // Subtitle
        let mut sub_rc = title_rc.clone();
        sub_rc.top = title_rc.bottom + s(4);
        sub_rc.bottom = sub_rc.top + s(20);
        self.lbl_subtitle.set_pos(sub_rc);

        // Info text
        let mut info_rc = sub_rc.clone();
        info_rc.top = sub_rc.bottom + s(32);
        info_rc.bottom = info_rc.top + s(60);
        self.lbl_info.set_pos(info_rc);

        // Buttons (bottom row)
        let (left_btn, right_btn) = bottom_buttons(rc.right - rc.left, rc.bottom - rc.top);
        self.btn_cancel.set_pos(left_btn);
        self.btn_next.set_pos(right_btn);
    }

    fn paint(&mut self, hdc: HDC, content_rc: &RECT, theme: &Theme) {
        self.lbl_title.paint(hdc, theme);
        self.lbl_subtitle.paint(hdc, theme);
        self.lbl_info.paint(hdc, theme);

        // Decorative accent line under title
        unsafe {
            let old_pen = windows::Win32::Graphics::Gdi::SelectObject(hdc, theme.pen_accent);
            let y = content_rc.top + s(44);
            let _ = windows::Win32::Graphics::Gdi::MoveToEx(
                hdc, content_rc.left, y, None,
            );
            let _ = windows::Win32::Graphics::Gdi::LineTo(
                hdc, content_rc.left + s(60), y,
            );
            let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_pen);
        }

        // If existing install detected, show info card
        if let Some(path) = &self.existing_install {
            let card_y = content_rc.top + s(140);
            let card_rc = make_rect(
                content_rc.left,
                card_y,
                content_rc.right,
                card_y + s(60),
            );
            fill_rounded_rect(hdc, &card_rc, s(12), theme.brush_card);
            unsafe {
                let null_brush = windows::Win32::Graphics::Gdi::GetStockObject(
                    windows::Win32::Graphics::Gdi::NULL_BRUSH,
                );
                let old_brush = windows::Win32::Graphics::Gdi::SelectObject(hdc, null_brush);
                let old_pen = windows::Win32::Graphics::Gdi::SelectObject(hdc, theme.pen_border);
                let _ = windows::Win32::Graphics::Gdi::RoundRect(
                    hdc, card_rc.left, card_rc.top, card_rc.right, card_rc.bottom,
                    s(12), s(12),
                );
                let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_brush);
                let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, old_pen);
            }

            let mut text_rc = card_rc.clone();
            text_rc.left += s(16);
            text_rc.right -= s(16);
            text_rc.top += s(8);
            text_rc.bottom = text_rc.top + s(20);
            let mut label = Label::new("检测到已安装版本");
            label.font = LabelFont::BodyBold;
            label.color = LabelColor::Bright;
            label.set_pos(text_rc.clone());
            label.paint(hdc, theme);

            let mut path_rc = text_rc.clone();
            path_rc.top = text_rc.bottom + s(2);
            path_rc.bottom = path_rc.top + s(18);
            let mut path_label = Label::new(path.clone());
            path_label.font = LabelFont::Small;
            path_label.color = LabelColor::Dim;
            path_label.set_pos(path_rc);
            path_label.paint(hdc, theme);
        }

        // Version info (bottom-left, above buttons)
        let ver_text = concat!("版本 ", env!("CARGO_PKG_VERSION"));
        let mut ver_rc = content_rc.clone();
        ver_rc.top = ver_rc.bottom - s(60);
        ver_rc.bottom = ver_rc.top + s(16);
        let mut label = Label::new(ver_text);
        label.font = LabelFont::Micro;
        label.color = LabelColor::Dim;
        label.set_pos(ver_rc);
        label.paint(hdc, theme);

        self.btn_cancel.paint(hdc, theme);
        self.btn_next.paint(hdc, theme);
    }

    fn on_mouse_move(&mut self, hwnd: HWND, x: i32, y: i32) {
        let mut needs_redraw = false;

        let hover_next = self.btn_next.hit_test(x, y);
        if hover_next != self.btn_next.hover {
            self.btn_next.hover = hover_next;
            needs_redraw = true;
        }

        let hover_cancel = self.btn_cancel.hit_test(x, y);
        if hover_cancel != self.btn_cancel.hover {
            self.btn_cancel.hover = hover_cancel;
            needs_redraw = true;
        }

        if needs_redraw {
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_lbutton_down(&mut self, hwnd: HWND, x: i32, y: i32) {
        if self.btn_next.hit_test(x, y) {
            self.btn_next.pressed = true;
            phonon_winui::invalidate_window(hwnd);
        }
        if self.btn_cancel.hit_test(x, y) {
            self.btn_cancel.pressed = true;
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_lbutton_up(&mut self, hwnd: HWND, x: i32, y: i32) {
        let was_next = self.btn_next.pressed;
        let was_cancel = self.btn_cancel.pressed;

        self.btn_next.pressed = false;
        self.btn_cancel.pressed = false;

        if was_next && self.btn_next.hit_test(x, y) {
            let page = LicensePage::new(self.ctx.clone());
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
