//! Phonon WinUI — Shared native UI toolkit for Windows (GDI-based).
//!
//! Provides a lightweight, dependency-free widget set used by the installer
//! and uninstaller.  Designed to be upgradable to Direct2D later without
//! changing the public widget API.

pub mod dpi;
pub mod gdi;
pub mod theme;
pub mod widgets;
pub mod window;

pub use dpi::{get_dpi, s};
pub use theme::Theme;
pub use window::{invalidate_window, NativeWindow, Navigator, Page, WindowConfig, WM_CUSTOM_START};
pub use widgets::*;
