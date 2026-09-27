//! Installation finish/completion page.

use phonon_winui::d2d::Canvas;
use phonon_winui::dpi::s;
use phonon_winui::gdi::*;
use phonon_winui::theme::{Theme, COLORS};
use phonon_winui::widgets::*;
use phonon_winui::window::{Navigator, Page};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::installer;
use crate::pages::AppContext;

pub struct FinishPage {
    ctx: AppContext,
    btn_finish: Button,
    lbl_title: Label,
    lbl_message: Label,
    cb_launch: Checkbox,
    cb_desktop: Checkbox,
    cb_start_menu: Checkbox,
    cb_add_to_path: Checkbox,
    cb_associate: Checkbox,
}

impl FinishPage {
    pub fn new(ctx: AppContext) -> Self {
        let (desktop, start_menu, add_to_path, _associate, launch) = {
            let c = ctx.config.lock().unwrap();
            (
                c.create_desktop_shortcut,
                c.create_start_menu,
                c.add_to_path,
                c.associate_files,
                c.launch_after_install,
            )
        };

        let mut btn_finish = Button::new("完成", ButtonStyle::Primary);
        btn_finish.radius = 20;

        let mut lbl_title = Label::new("安装完成");
        lbl_title.font = LabelFont::Big;
        lbl_title.color = LabelColor::Bright;

        let mut lbl_message = Label::new("Phonon 已成功安装到您的计算机。");
        lbl_message.font = LabelFont::Body;
        lbl_message.color = LabelColor::Normal;

        let mut cb_launch = Checkbox::new("立即运行 Phonon");
        cb_launch.checked = launch;

        let mut cb_desktop = Checkbox::new("创建桌面快捷方式");
        cb_desktop.checked = desktop;

        let mut cb_start_menu = Checkbox::new("创建开始菜单快捷方式");
        cb_start_menu.checked = start_menu;

        let mut cb_add_to_path = Checkbox::new("添加到系统 PATH");
        cb_add_to_path.checked = add_to_path;

        let mut cb_associate = Checkbox::new("在“打开方式”中提供 Phonon（flac/mp3/wav 等）");
        // Default OFF: registration only fills the "Open with" list and only
        // claims the default when an extension has no default player at all,
        // so the user's current player is never overridden silently.
        cb_associate.checked = false;

        Self {
            ctx,
            btn_finish,
            lbl_title,
            lbl_message,
            cb_launch,
            cb_desktop,
            cb_start_menu,
            cb_add_to_path,
            cb_associate,
        }
    }
}

impl Page for FinishPage {
    fn on_enter(&mut self, hwnd: HWND) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let content = content_rect(rc.right - rc.left, rc.bottom - rc.top);
        let (_left_btn, right_btn) = bottom_buttons(rc.right - rc.left, rc.bottom - rc.top);

        // Title
        let mut title_rc = content.clone();
        title_rc.top += s(50);
        title_rc.bottom = title_rc.top + s(36);
        self.lbl_title.set_pos(title_rc);

        // Message
        let mut msg_rc = title_rc.clone();
        msg_rc.top = title_rc.bottom + s(12);
        msg_rc.bottom = msg_rc.top + s(24);
        self.lbl_message.set_pos(msg_rc);

        // Checkboxes (above the finish button)
        let cb_start = right_btn.top - s(5 * 28 + 16);
        let cb_h = s(28);
        let cb_gap = s(4);
        let checkboxes = [
            &mut self.cb_desktop,
            &mut self.cb_start_menu,
            &mut self.cb_add_to_path,
            &mut self.cb_associate,
            &mut self.cb_launch,
        ];
        for (i, cb) in checkboxes.into_iter().enumerate() {
            let y = cb_start + (cb_h + cb_gap) * i as i32;
            let mut cb_rc = content.clone();
            cb_rc.top = y;
            cb_rc.bottom = y + cb_h;
            cb.set_pos(cb_rc);
        }

        // Finish button
        self.btn_finish.set_pos(right_btn);
    }

    fn paint(&mut self, canvas: &mut Canvas, content_rc: &RECT, _theme: &Theme) {
        // Success circle with checkmark
        let cx = content_rc.left + (content_rc.right - content_rc.left) / 2;
        let circle_top = content_rc.top + s(20);
        let cy = circle_top + s(30);
        let radius = s(28);

        // Success ring: outer success-colored circle + inner soft fill
        canvas.fill_ellipse(cx, cy, radius, radius, COLORS.success);
        canvas.fill_ellipse(cx, cy, radius - s(2), radius - s(2), COLORS.success_soft);

        // Checkmark inside the circle
        let half = radius / 2;
        canvas.line(
            cx - half,
            cy,
            cx - half / 3,
            cy + half / 2,
            COLORS.success,
            3.0,
        );
        canvas.line(
            cx - half / 3,
            cy + half / 2,
            cx + half,
            cy - half / 2,
            COLORS.success,
            3.0,
        );

        self.lbl_title.paint(canvas, _theme);
        self.lbl_message.paint(canvas, _theme);
        self.cb_desktop.paint(canvas, _theme);
        self.cb_start_menu.paint(canvas, _theme);
        self.cb_add_to_path.paint(canvas, _theme);
        self.cb_associate.paint(canvas, _theme);
        self.cb_launch.paint(canvas, _theme);
        self.btn_finish.paint(canvas, _theme);
    }

    fn on_mouse_move(&mut self, hwnd: HWND, x: i32, y: i32) {
        let mut needs_redraw = false;

        let hover_finish = self.btn_finish.hit_test(x, y);
        if hover_finish != self.btn_finish.hover {
            self.btn_finish.hover = hover_finish;
            needs_redraw = true;
        }

        for cb in [
            &mut self.cb_launch,
            &mut self.cb_desktop,
            &mut self.cb_start_menu,
            &mut self.cb_add_to_path,
            &mut self.cb_associate,
        ]
        .iter_mut()
        {
            let hover = cb.hit_test(x, y);
            if hover != cb.hover {
                cb.hover = hover;
                needs_redraw = true;
            }
        }

        if needs_redraw {
            phonon_winui::invalidate_window(hwnd);
        }
    }

    fn on_lbutton_down(&mut self, hwnd: HWND, x: i32, y: i32) {
        if self.btn_finish.hit_test(x, y) {
            self.btn_finish.pressed = true;
        }
        phonon_winui::invalidate_window(hwnd);
    }

    fn on_lbutton_up(&mut self, hwnd: HWND, x: i32, y: i32) {
        let was_finish = self.btn_finish.pressed;
        self.btn_finish.pressed = false;

        // Toggle checkboxes
        for cb in [
            &mut self.cb_launch,
            &mut self.cb_desktop,
            &mut self.cb_start_menu,
            &mut self.cb_add_to_path,
            &mut self.cb_associate,
        ]
        .iter_mut()
        {
            if cb.hit_test(x, y) {
                cb.checked = !cb.checked;
            }
        }

        if was_finish && self.btn_finish.hit_test(x, y) {
            // Save config
            {
                let mut cfg = self.ctx.config.lock().unwrap();
                cfg.create_desktop_shortcut = self.cb_desktop.checked;
                cfg.create_start_menu = self.cb_start_menu.checked;
                cfg.add_to_path = self.cb_add_to_path.checked;
                cfg.associate_files = self.cb_associate.checked;
                cfg.launch_after_install = self.cb_launch.checked;
            }

            // Apply post-install options (shortcuts, PATH)
            let config = self.ctx.config.lock().unwrap().clone();
            let _ = installer::apply_post_install_options(&config);

            // Launch Phonon if checkbox is checked
            if self.cb_launch.checked {
                let exe_path = format!("{}\\phonon.exe", config.install_dir);
                let _ = std::process::Command::new(&exe_path)
                    .current_dir(&config.install_dir)
                    .spawn();
            }

            Navigator::close(hwnd);
            return;
        }

        phonon_winui::invalidate_window(hwnd);
    }
}
