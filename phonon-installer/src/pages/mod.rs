//! Installer UI pages.
//!
//! Each page implements the `Page` trait from phonon-winui and
//! represents one step in the install wizard.
//!
//! Flow: Welcome → License → InstallDir → Progress → Finish

use std::sync::{Arc, Mutex};

use crate::installer::InstallConfig;

pub mod finish;
pub mod install_dir;
pub mod license;
pub mod progress;
pub mod welcome;

/// Shared application state passed between pages.
#[derive(Clone)]
pub struct AppContext {
    pub config: Arc<Mutex<InstallConfig>>,
}

impl AppContext {
    pub fn new(config: Arc<Mutex<InstallConfig>>) -> Self {
        Self { config }
    }
}
