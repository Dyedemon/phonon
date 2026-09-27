//! Install directory selection page (with Per-User/Per-Machine selection).

use phonon_winui::d2d::{tf, Canvas, FontStyle};
use phonon_winui::dpi::s;
use phonon_winui::gdi::*;
use phonon_winui::theme::{Theme, COLORS};
use phonon_winui::widgets::*;
use phonon_winui::window::{Navigator, Page};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::installer;
use crate::pages::{license::LicensePage, progress::ProgressPage, AppContext};

pub struct InstallDirPage {
    ctx: AppContext,
    btn_back: Button,
    btn_next: Button,
    btn_cancel: Button,
    btn_browse: Button,
    lbl_title: Label,
    lbl_dir_label: Label,
    lbl_space_info: Label,
    dir_text: String,

    rc_per_user: RadioCard,
    rc_per_machine: RadioCard,
    per_machine: bool,
}

impl InstallDirPage {
    pub fn new(ctx: AppContext) -> Self {
        let dir_text = ctx.config.lock().unwrap().install_dir.clone();
        let per_machine = ctx.config.lock().unwrap().per_machine;

        let btn_back = Button::new("< 上一步", ButtonStyle::Secondary);
        let btn_next = Button::new("安装 >", ButtonStyle::Primary);
        let btn_cancel = Button::new("取消", ButtonStyle::Secondary);
        let btn_browse = Button::new("浏览...", ButtonStyle::Secondary);

        let mut lbl_title = Label::new("选择安装位置");
        lbl_title.font = LabelFont::Title;
        lbl_title.color = LabelColor::Bright;

        let mut lbl_dir_label = Label::new("安装到以下文件夹：");
        lbl_dir_label.font = LabelFont::Body;
        lbl_dir_label.color = LabelColor::Normal;

        let mut lbl_space_info =
            Label::new(&format!("所需空间：约 {} MB", installer::REQUIRED_SPACE_MB));
        lbl_space_info.font = LabelFont::Small;
        lbl_space_info.color = LabelColor::Dim;

        let mut rc_per_user = RadioCard::new("仅为当前用户安装", "安装到用户目录，无需管理员权限");
        rc_per_user.selected = !per_machine;

        let mut rc_per_machine = RadioCard::new("为所有用户安装", "安装到系统目录，需要管理员权限");
        rc_per_machine.selected = per_machine;

        Self {
            ctx,
            btn_back,
            btn_next,
            btn_cancel,
            btn_browse,
            lbl_title,
            lbl_dir_label,
            lbl_space_info,
            dir_text,
            rc_per_user,
            rc_per_machine,
            per_machine,
        }
    }
}

impl Page for InstallDirPage {
    fn on_enter(&mut self, hwnd: HWND) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let content = content_rect(rc.right - rc.left, rc.bottom - rc.top);
        let (left_btn, right_btn) = bottom_buttons(rc.right - rc.left, rc.bottom - rc.top);

        // Title
        let mut title_rc = content.clone();
        title_rc.bottom = title_rc.top + s(24);
        self.lbl_title.set_pos(title_rc);

        // Install mode cards (side by side)
        let card_y = content.top + s(36);
        let card_h = s(70);
        let gap = s(8);
        let half_w = (content.right - content.left - gap) / 2;

        let mut left_card = content.clone();
        left_card.top = card_y;
        left_card.bottom = card_y + card_h;
        left_card.right = content.left + half_w;
        self.rc_per_user.set_pos(left_card);

        let mut right_card = content.clone();
        right_card.top = card_y;
        right_card.bottom = card_y + card_h;
        right_card.left = content.left + half_w + gap;
        self.rc_per_machine.set_pos(right_card);

        // Dir label
        let mut label_rc = content.clone();
        label_rc.top = card_y + card_h + s(20);
        label_rc.bottom = label_rc.top + s(20);
        self.lbl_dir_label.set_pos(label_rc);

        // Browse button
        let browse_w = s(80);
        let mut browse_rc = content.clone();
        browse_rc.top = label_rc.bottom + s(8);
        browse_rc.bottom = browse_rc.top + s(32);
        browse_rc.left = browse_rc.right - browse_w;
        self.btn_browse.set_pos(browse_rc);

        // Space info
        let mut space_rc = content.clone();
        space_rc.top = browse_rc.bottom + s(12);
        space_rc.bottom = space_rc.top + s(18);
        self.lbl_space_info.set_pos(space_rc);

        // Buttons
        let back_rc = left_btn.clone();
        self.btn_back.set_pos(back_rc);

        let mut cancel_rc = left_btn.clone();
        cancel_rc.left += s(130);
        cancel_rc.right += s(130);
        self.btn_cancel.set_pos(cancel_rc);

        self.btn_next.set_pos(right_btn);
    }

    fn paint(&mut self, canvas: &mut Canvas, content_rc: &RECT, _theme: &Theme) {
        self.lbl_title.paint(canvas, _theme);
        self.rc_per_user.paint(canvas, _theme);
        self.rc_per_machine.paint(canvas, _theme);
        self.lbl_dir_label.paint(canvas, _theme);
        self.lbl_space_info.paint(canvas, _theme);

        // Directory path text box (readonly appearance)
        let browse_w = s(80);
        let label_bottom = self.lbl_dir_label.rect.bottom;
        let mut dir_rc = content_rc.clone();
        dir_rc.top = label_bottom + s(8);
        dir_rc.bottom = dir_rc.top + s(32);
        dir_rc.right = content_rc.right - browse_w - s(8);

        canvas.fill_round(&dir_rc, s(6), COLORS.bg_light);
        canvas.stroke_round(&dir_rc, s(6), COLORS.border, 1.0);

        // Dir text inside the box
        let mut text_rc = dir_rc.clone();
        text_rc.left += s(10);
        text_rc.right -= s(10);
        canvas.text(
            &self.dir_text,
            &text_rc,
            FontStyle::Body,
            COLORS.text_bright,
            tf::LEFT | tf::VCENTER | tf::SINGLELINE | tf::END_ELLIPSIS,
        );

        self.btn_browse.paint(canvas, _theme);
        self.btn_back.paint(canvas, _theme);
        self.btn_cancel.paint(canvas, _theme);
        self.btn_next.paint(canvas, _theme);
    }

    fn on_mouse_move(&mut self, hwnd: HWND, x: i32, y: i32) {
        let mut needs_redraw = false;

        for btn in [
            &mut self.btn_back,
            &mut self.btn_next,
            &mut self.btn_cancel,
            &mut self.btn_browse,
        ]
        .iter_mut()
        {
            let hover = btn.hit_test(x, y);
            if hover != btn.hover {
                btn.hover = hover;
                needs_redraw = true;
            }
        }

        let hover_user = self.rc_per_user.hit_test(x, y);
        if hover_user != self.rc_per_user.hover {
            self.rc_per_user.hover = hover_user;
            needs_redraw = true;
        }
        let hover_machine = self.rc_per_machine.hit_test(x, y);
        if hover_machine != self.rc_per_machine.hover {
            self.rc_per_machine.hover = hover_machine;
            needs_redraw = true;
        }

        if needs_redraw {
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_lbutton_down(&mut self, hwnd: HWND, x: i32, y: i32) {
        for btn in [
            &mut self.btn_back,
            &mut self.btn_next,
            &mut self.btn_cancel,
            &mut self.btn_browse,
        ]
        .iter_mut()
        {
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
        let was_browse = self.btn_browse.pressed;

        self.btn_back.pressed = false;
        self.btn_next.pressed = false;
        self.btn_cancel.pressed = false;
        self.btn_browse.pressed = false;

        // Radio card selection (mutually exclusive)
        if self.rc_per_user.hit_test(x, y) {
            self.per_machine = false;
            self.rc_per_user.selected = true;
            self.rc_per_machine.selected = false;
        }
        if self.rc_per_machine.hit_test(x, y) {
            self.per_machine = true;
            self.rc_per_machine.selected = true;
            self.rc_per_user.selected = false;
        }

        if was_back && self.btn_back.hit_test(x, y) {
            Navigator::navigate(hwnd, Box::new(LicensePage::new(self.ctx.clone())));
            return;
        }

        if was_next && self.btn_next.hit_test(x, y) {
            let mut cfg = self.ctx.config.lock().unwrap();
            cfg.install_dir = self.dir_text.clone();
            cfg.per_machine = self.per_machine;
            drop(cfg);

            Navigator::navigate(hwnd, Box::new(ProgressPage::new(self.ctx.clone())));
            return;
        }

        if was_cancel && self.btn_cancel.hit_test(x, y) {
            Navigator::close(hwnd);
            return;
        }

        if was_browse && self.btn_browse.hit_test(x, y) {
            if let Some(path) = browse_for_folder(hwnd, "选择 Phonon 安装目录") {
                self.dir_text = path;
            }
        }

        phonon_winui::invalidate_window(hwnd);
    }
}

fn browse_for_folder(hwnd: HWND, title: &str) -> Option<String> {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
    };

    let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let mut display_buf = [0u16; 260];

    let bi = BROWSEINFOW {
        hwndOwner: hwnd,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: windows::core::PWSTR(display_buf.as_mut_ptr()),
        lpszTitle: windows::core::PCWSTR(title_w.as_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | 0x0040,
        lpfn: None,
        lParam: LPARAM(0),
        iImage: 0,
    };

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let need_uninit = hr.is_ok();

        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            if need_uninit {
                CoUninitialize();
            }
            return None;
        }

        let mut path_buf = [0u16; 260];
        let got_path = SHGetPathFromIDListW(pidl, &mut path_buf);
        CoTaskMemFree(Some(pidl as *const std::ffi::c_void));

        if need_uninit {
            CoUninitialize();
        }

        if got_path.as_bool() {
            let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
            Some(String::from_utf16_lossy(&path_buf[..len]))
        } else {
            None
        }
    }
}
