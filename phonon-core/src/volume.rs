//! Volume control strategy.
//!
//! Prioritizes hardware volume, falls back to software volume when
//! hardware control is not available (e.g., WASAPI exclusive mode).

/// Volume control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeMode {
    /// Hardware volume control (DAC level, no PCM modification).
    Hardware,
    /// Software volume control (applied as last DSP node).
    Software,
}

/// Volume control manager.
/// Stores separate levels for HW and SW modes so switching preserves each.
pub struct VolumeControl {
    mode: VolumeMode,
    /// Hardware volume level [0.0, 1.0].
    hw_level: f32,
    /// Software volume level [0.0, 1.0].
    sw_level: f32,
    /// Whether the device supports hardware volume.
    hardware_supported: bool,
    /// Optional WASAPI hardware volume controller (Windows only).
    #[cfg(windows)]
    hw_controller: Option<crate::wasapi::WasapiEndpointVolume>,
}

impl VolumeControl {
    /// Create a new volume control.
    pub fn new(hardware_supported: bool) -> Self {
        Self {
            mode: if hardware_supported {
                VolumeMode::Hardware
            } else {
                VolumeMode::Software
            },
            hw_level: 1.0,
            sw_level: 1.0,
            hardware_supported,
            #[cfg(windows)]
            hw_controller: None,
        }
    }

    /// Get the current volume mode.
    pub fn mode(&self) -> VolumeMode {
        self.mode
    }

    /// Get the current volume level [0.0, 1.0] (for the active mode).
    pub fn level(&self) -> f32 {
        match self.mode {
            VolumeMode::Hardware => self.hw_level,
            VolumeMode::Software => self.sw_level,
        }
    }

    /// Set the volume level for the currently active mode.
    /// Does NOT change the volume mode (use set_mode for that).
    /// Returns the current mode.
    pub fn set_volume(&mut self, level: f32) -> VolumeMode {
        let clamped = level.clamp(0.0, 1.0);
        match self.mode {
            VolumeMode::Hardware => self.hw_level = clamped,
            VolumeMode::Software => self.sw_level = clamped,
        }
        self.mode
    }

    /// Apply volume attenuation to a buffer of samples.
    /// Uses the current mode's level.
    pub fn apply_software_volume(&self, samples: &mut [f32]) {
        let level = self.level();
        if level < 1.0 {
            for sample in samples.iter_mut() {
                *sample *= level;
            }
        }
    }

    /// Check if hardware volume is supported.
    pub fn is_hardware_supported(&self) -> bool {
        self.hardware_supported
    }

    /// Update hardware volume support status.
    pub fn set_hardware_supported(&mut self, supported: bool) {
        self.hardware_supported = supported;
        if !supported && self.mode == VolumeMode::Hardware {
            self.mode = VolumeMode::Software;
        }
    }

    /// Explicitly set the volume mode.
    /// Hardware mode is only valid when hardware is supported.
    pub fn set_mode(&mut self, mode: VolumeMode) {
        if mode == VolumeMode::Hardware && !self.hardware_supported {
            self.mode = VolumeMode::Software;
        } else {
            self.mode = mode;
        }
    }

    /// Set the WASAPI hardware volume controller.
    /// Syncs the current hw_level from the device's current volume.
    #[cfg(windows)]
    pub fn set_hw_controller(&mut self, controller: crate::wasapi::WasapiEndpointVolume) {
        // Sync current level from the device
        if let Ok(sys_vol) = controller.get_master_volume() {
            self.hw_level = sys_vol.clamp(0.0, 1.0);
        }
        self.hw_controller = Some(controller);
    }

    /// Apply hardware volume via the system API.
    /// Returns true if HW volume was applied successfully.
    #[cfg(windows)]
    pub fn apply_hw_volume(&self) -> bool {
        if let Some(ref hw) = self.hw_controller {
            hw.set_master_volume(self.hw_level).is_ok()
        } else {
            false
        }
    }

    /// Sync the current volume level from the hardware device.
    /// Returns the new level if successful.
    #[cfg(windows)]
    pub fn sync_from_device(&mut self) -> Option<f32> {
        if let Some(ref hw) = self.hw_controller {
            if let Ok(sys_vol) = hw.get_master_volume() {
                let clamped = sys_vol.clamp(0.0, 1.0);
                self.hw_level = clamped;
                return Some(clamped);
            }
        }
        None
    }

    /// Read the current device volume directly (does not update internal level).
    /// Returns the device level if successful.
    #[cfg(windows)]
    pub fn read_device_volume(&self) -> Option<f32> {
        if let Some(ref hw) = self.hw_controller {
            hw.get_master_volume().ok().map(|v| v.clamp(0.0, 1.0))
        } else {
            None
        }
    }

    /// Check if the hardware volume controller is available.
    #[cfg(windows)]
    pub fn has_hw_controller(&self) -> bool {
        self.hw_controller.is_some()
    }

    #[cfg(not(windows))]
    pub fn has_hw_controller(&self) -> bool {
        false
    }

    /// Set the hardware volume level directly (without changing mode).
    pub fn set_hw_level(&mut self, level: f32) {
        self.hw_level = level.clamp(0.0, 1.0);
    }

    /// Get the hardware volume level (without changing mode).
    pub fn get_hw_level(&self) -> f32 {
        self.hw_level
    }

    /// Get the software volume level (without changing mode).
    pub fn get_sw_level(&self) -> f32 {
        self.sw_level
    }

    /// Set the software volume level directly (without changing mode).
    pub fn set_sw_level(&mut self, level: f32) {
        self.sw_level = level.clamp(0.0, 1.0);
    }

    /// Apply hardware volume via an external endpoint volume controller.
    #[cfg(windows)]
    pub fn apply_hw_volume_with(&self, hw: &crate::wasapi::WasapiEndpointVolume) -> bool {
        hw.set_master_volume(self.hw_level).is_ok()
    }

    /// Apply hardware volume (no-op on non-Windows).
    #[cfg(not(windows))]
    pub fn apply_hw_volume(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_volume() {
        let mut vc = VolumeControl::new(true);
        assert_eq!(vc.mode(), VolumeMode::Hardware);
        assert_eq!(vc.level(), 1.0);

        vc.set_volume(0.5);
        assert_eq!(vc.level(), 0.5);
        assert_eq!(vc.mode(), VolumeMode::Hardware);
    }

    #[test]
    fn test_software_volume() {
        let mut vc = VolumeControl::new(false);
        assert_eq!(vc.mode(), VolumeMode::Software);

        vc.set_volume(0.5);
        let mut samples = vec![1.0f32, 0.5, -0.5, -1.0];
        vc.apply_software_volume(&mut samples);

        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - 0.25).abs() < 0.001);
        assert!((samples[2] + 0.25).abs() < 0.001);
        assert!((samples[3] + 0.5).abs() < 0.001);
    }

    #[test]
    fn test_volume_clamp() {
        let mut vc = VolumeControl::new(false);
        vc.set_volume(1.5);
        assert_eq!(vc.level(), 1.0);
        vc.set_volume(-0.5);
        assert_eq!(vc.level(), 0.0);
    }

    #[test]
    fn test_separate_hw_sw_levels() {
        let mut vc = VolumeControl::new(true);
        assert_eq!(vc.mode(), VolumeMode::Hardware);

        // Set HW level to 0.8
        vc.set_volume(0.8);
        assert_eq!(vc.level(), 0.8);

        // Switch to SW, level should be 1.0 (default)
        vc.set_mode(VolumeMode::Software);
        assert_eq!(vc.mode(), VolumeMode::Software);
        assert_eq!(vc.level(), 1.0);

        // Set SW level to 0.3
        vc.set_volume(0.3);
        assert_eq!(vc.level(), 0.3);

        // Switch back to HW, should restore 0.8
        vc.set_mode(VolumeMode::Hardware);
        assert_eq!(vc.level(), 0.8);
    }
}
