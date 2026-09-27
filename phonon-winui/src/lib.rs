//! Phonon WinUI — shared native UI toolkit for Windows (Direct2D + DirectWrite).
//!
//! Provides a lightweight, dependency-free widget set used by the installer
//! and uninstaller.  Rendering goes through [`d2d::Canvas`], a thin wrapper
//! over `ID2D1HwndRenderTarget` + DirectWrite; the layered-window color key
//! keeps working because the HwndRenderTarget presents via GDI blit.

pub mod d2d;
pub mod dpi;
pub mod gdi;
pub mod theme;
pub mod widgets;
pub mod window;

pub use dpi::{get_dpi, s};
pub use theme::Theme;
pub use widgets::*;
pub use window::{invalidate_window, NativeWindow, Navigator, Page, WindowConfig, WM_CUSTOM_START};
