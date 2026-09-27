/// Saved acoustic-calibration state (PEQ bands + FIR plugin config).
///
/// The calibration plugin is an external wasm module — the main app has no
/// build-time dependency on it. These records are only re-applied at startup
/// when the plugin is present again, so removing the plugin cleanly removes
/// its effects from the audio path.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CalibrationSettings {
    pub peq_bands_l: Vec<EqBand>,
    pub peq_bands_r: Vec<EqBand>,
    pub bypass: bool,
    /// Raw FirConfig JSON last pushed to the calibration wasm plugin.
    pub fir_config: Option<String>,
    /// Fit id of the last applied result (display only).
    pub fit_id: Option<String>,
}
