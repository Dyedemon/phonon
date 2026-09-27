//! Installation progress page.
//!
//! Runs the actual installation on a background thread and updates
//! the UI with progress via PostMessage.

use phonon_winui::d2d::Canvas;
use phonon_winui::dpi::s;
use phonon_winui::gdi::*;
use phonon_winui::theme::Theme;
use phonon_winui::widgets::*;
use phonon_winui::window::{Navigator, Page, WM_CUSTOM_START};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::installer::{self, InstallConfig, InstallProgress};
use crate::pages::{finish::FinishPage, AppContext};

/// Custom messages from install thread to UI.
const MSG_PROGRESS: u32 = WM_CUSTOM_START; // wParam = permille (0-1000), lParam = step text ptr
const MSG_DONE: u32 = WM_CUSTOM_START + 1; // wParam = 0 success, 1 error
const MSG_ERROR: u32 = WM_CUSTOM_START + 2; // lParam = error message ptr

pub struct ProgressPage {
    ctx: AppContext,
    progress_bar: ProgressBar,
    lbl_title: Label,
    lbl_step: Label,
    lbl_percent: Label,
    current_step: String,
    percent: f32,
    done: bool,
    error: Option<String>,
}

impl ProgressPage {
    pub fn new(ctx: AppContext) -> Self {
        let progress_bar = ProgressBar::new();

        let mut lbl_title = Label::new("正在安装 Phonon");
        lbl_title.font = LabelFont::Title;
        lbl_title.color = LabelColor::Bright;

        let mut lbl_step = Label::new("正在准备安装...");
        lbl_step.font = LabelFont::Body;
        lbl_step.color = LabelColor::Normal;

        let mut lbl_percent = Label::new("0%");
        lbl_percent.font = LabelFont::BodyBold;
        lbl_percent.color = LabelColor::Bright;
        lbl_percent.align = LabelAlign::Right;

        Self {
            ctx,
            progress_bar,
            lbl_title,
            lbl_step,
            lbl_percent,
            current_step: "正在准备安装...".to_string(),
            percent: 0.0,
            done: false,
            error: None,
        }
    }
}

impl Page for ProgressPage {
    fn on_enter(&mut self, hwnd: HWND) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let content = content_rect(rc.right - rc.left, rc.bottom - rc.top);

        // Title
        let mut title_rc = content.clone();
        title_rc.bottom = title_rc.top + s(24);
        self.lbl_title.set_pos(title_rc);

        // Progress bar (middle of the page)
        let bar_y = content.top + s(120);
        let bar_h = s(24);
        let mut bar_rc = content.clone();
        bar_rc.top = bar_y;
        bar_rc.bottom = bar_y + bar_h;
        self.progress_bar.set_pos(bar_rc);

        // Step label (above progress bar)
        let mut step_rc = content.clone();
        step_rc.top = bar_y - s(32);
        step_rc.bottom = bar_y - s(8);
        self.lbl_step.set_pos(step_rc);

        // Percent label (right of step)
        let mut pct_rc = step_rc.clone();
        pct_rc.left = pct_rc.right - s(60);
        self.lbl_percent.set_pos(pct_rc);

        // Start the install thread
        let hwnd_raw = hwnd.0 as isize;
        let config = self.ctx.config.lock().unwrap().clone();
        std::thread::spawn(move || {
            let hwnd = HWND(hwnd_raw as *mut _);
            install_thread(hwnd, config);
        });
    }

    fn paint(&mut self, canvas: &mut Canvas, _content_rc: &RECT, theme: &Theme) {
        self.lbl_title.paint(canvas, theme);
        self.lbl_step.paint(canvas, theme);
        self.lbl_percent.paint(canvas, theme);
        self.progress_bar.paint(canvas, theme);
    }

    fn on_user_message(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        match msg {
            MSG_PROGRESS => {
                // wparam = permille (0-1000), or usize::MAX to skip progress update
                let permille = wparam.0 as i32;
                if permille != usize::MAX as i32 {
                    self.percent = permille as f32 / 1000.0;
                    self.progress_bar.set_percent(self.percent);
                    let pct = (self.percent * 100.0) as i32;
                    self.lbl_percent.text = format!("{}%", pct);
                }

                // Update step text if provided
                if lparam.0 != 0 {
                    unsafe {
                        self.current_step = take_wide_string(lparam.0 as *const u16);
                        self.lbl_step.text = self.current_step.clone();
                    }
                }

                phonon_winui::invalidate_window(hwnd);
            }

            MSG_DONE => {
                self.done = true;
                self.percent = 1.0;
                self.progress_bar.set_percent(1.0);
                self.lbl_step.text = "安装完成".to_string();
                self.lbl_percent.text = "100%".to_string();

                // Navigate to finish page after a short delay
                let ctx = self.ctx.clone();
                let hwnd_raw = hwnd.0 as isize;
                std::thread::spawn(move || {
                    let hwnd = HWND(hwnd_raw as *mut _);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    Navigator::navigate(hwnd, Box::new(FinishPage::new(ctx)));
                });

                phonon_winui::invalidate_window(hwnd);
            }

            MSG_ERROR => {
                self.done = true;
                if lparam.0 != 0 {
                    unsafe {
                        self.error = Some(take_wide_string(lparam.0 as *const u16));
                    }
                }
                self.lbl_step.text = "安装失败".to_string();
                phonon_winui::invalidate_window(hwnd);
            }

            _ => {}
        }
    }
}

/// Reclaim a heap-allocated null-terminated UTF-16 string sent via PostMessage.
///
/// The sender allocates via `Box::into_raw(Box<[u16]>)`; this function
/// reconstructs and drops the `Box<[u16]>` to free the memory.
unsafe fn take_wide_string(ptr: *const u16) -> String {
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    let s = String::from_utf16_lossy(slice);
    let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u16, len + 1));
    s
}

/// Background thread that runs the installation.
fn install_thread(hwnd: HWND, config: InstallConfig) {
    let hwnd_raw = hwnd.0 as isize;

    // The GUI flow previously had zero logging — an install that failed and
    // rolled back left only an empty install dir and a one-line log header.
    // Log every step so %TEMP%\phonon-installer.log can diagnose failures.
    log::info!(
        "GUI 安装开始: {} (per_machine: {})",
        config.install_dir,
        config.per_machine
    );
    let result = installer::run_install(&config, move |progress| {
        let hwnd = HWND(hwnd_raw as *mut _);
        match progress {
            InstallProgress::Overall(p) => {
                let permille = (p * 1000.0) as usize;
                unsafe {
                    let _ = PostMessageW(hwnd, MSG_PROGRESS, WPARAM(permille), LPARAM(0));
                }
            }
            InstallProgress::Step(step) => {
                log::info!("  步骤: {}", step);
                let ptr = alloc_wide_string(step);
                unsafe {
                    let _ =
                        PostMessageW(hwnd, MSG_PROGRESS, WPARAM(usize::MAX), LPARAM(ptr as isize));
                }
            }
            InstallProgress::Done => {
                log::info!("安装完成");
                unsafe {
                    let _ = PostMessageW(hwnd, MSG_DONE, WPARAM(0), LPARAM(0));
                }
            }
            InstallProgress::Failed(code) => {
                log::error!("安装失败，错误码: {}", code);
                let msg = format!("安装失败 (错误代码: {})", code);
                let ptr = alloc_wide_string(&msg);
                unsafe {
                    let _ = PostMessageW(hwnd, MSG_ERROR, WPARAM(0), LPARAM(ptr as isize));
                }
            }
        }
    });

    if let Err(e) = result {
        log::error!("安装异常: {e:?}");
        let msg = format!("安装失败: {}", e);
        let ptr = alloc_wide_string(&msg);
        unsafe {
            let _ = PostMessageW(hwnd, MSG_ERROR, WPARAM(0), LPARAM(ptr as isize));
        }
    }
}

/// Allocate a null-terminated UTF-16 string on the heap as `Box<[u16]>`,
/// returning a raw pointer for use with `PostMessageW`.
///
/// The receiver must call `take_wide_string` to reclaim it.
fn alloc_wide_string(s: &str) -> *const u16 {
    let wtext: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let boxed: Box<[u16]> = wtext.into_boxed_slice();
    Box::into_raw(boxed) as *const u16
}
