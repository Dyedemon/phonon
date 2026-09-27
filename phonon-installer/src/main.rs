#![windows_subsystem = "windows"]

//! Phonon Installer — Native Windows setup program.
//!
//! Uses the phonon-winui toolkit for a borderless, dark-themed UI that
//! matches the main Phonon application.  Installation logic handles file
//! extraction, registry entries, shortcuts, and uninstall registration.

mod installer;
mod logger;
mod pages;

use std::sync::{Arc, Mutex};

use phonon_winui::window::{NativeWindow, WindowConfig};

use installer::InstallConfig;
use pages::{welcome::WelcomePage, AppContext};

fn main() {
    logger::init();

    let args = parse_args();

    if args.silent {
        let exit_code = run_silent_install(&args);
        std::process::exit(exit_code);
    }

    let default_dir = args.install_dir.clone().unwrap_or_else(default_install_dir);

    let config = Arc::new(Mutex::new(InstallConfig {
 install_dir: default_dir,
 per_machine: false,
 create_desktop_shortcut: true,
 create_start_menu: false,
 add_to_path: false,
 associate_files: false,
 launch_after_install: true,
 }));

    let ctx = AppContext::new(config);
    let initial_page = Box::new(WelcomePage::new(ctx));

    let window_config = WindowConfig {
 class_name: "PhononInstaller",
 title: "Phonon 安装向导",
 width: 600,
 height: 560,
 };

    let exit_code = NativeWindow::run(window_config, initial_page);
    std::process::exit(exit_code);
}

struct CliArgs {
    silent: bool,
    install_dir: Option<String>,
}

fn parse_args() -> CliArgs {
    let mut silent = false;
    let mut install_dir = None;

    for arg in std::env::args().skip(1) {
        if arg.eq_ignore_ascii_case("/S") || arg.eq_ignore_ascii_case("--silent") {
            silent = true;
        } else if let Some(rest) = arg.strip_prefix("/D=")
            .or_else(|| arg.strip_prefix("--dir="))
        {
            install_dir = Some(rest.trim_matches('"').to_string());
        }
    }

    CliArgs { silent, install_dir }
}

fn run_silent_install(args: &CliArgs) -> i32 {
    let install_dir = args.install_dir.clone().unwrap_or_else(default_install_dir);

    let config = InstallConfig {
 install_dir,
 per_machine: false,
 create_desktop_shortcut: true,
 create_start_menu: false,
 add_to_path: false,
 associate_files: false,
 launch_after_install: false,
 };

    log::info!("静默安装开始: {}", config.install_dir);

    match installer::run_install(&config, |p| {
        match p {
            installer::InstallProgress::Step(s) => log::info!("  步骤: {}", s),
            installer::InstallProgress::Overall(v) => log::debug!("  进度: {:.0}%", v * 100.0),
            installer::InstallProgress::Done => log::info!("安装完成"),
            installer::InstallProgress::Failed(c) => log::error!("安装失败，错误码: {}", c),
        }
    }) {
        Ok(()) => {
            log::info!("静默安装成功");
            0
        }
        Err(e) => {
            log::error!("静默安装失败: {}", e);
            1
        }
    }
}

fn default_install_dir() -> String {
 if let Ok(lappdata) = std::env::var("LOCALAPPDATA") {
 format!("{}\\Phonon", lappdata)
 } else if let Ok(pf) = std::env::var("ProgramFiles") {
 format!("{}\\Phonon", pf)
 } else {
 "C:\\Program Files\\Phonon".to_string()
 }
}
