// Acoustic calibration plugin persistence
//
// The calibration wasm plugin is optional — the main app has no build-time
// dependency on it. These commands only record / restore the last applied
// state so a re-enable after restart picks up where it left off. Restore
// is gated on plugin presence: uninstalling the plugin removes its effects
// from the audio path instead of leaving hidden filters behind.
// ──────────────────────────────────────────────────────────────────────

/// Persist the calibration panel's applied state (PEQ bands + FIR plugin
/// config + bypass flag). Called by the frontend after apply / bypass.
#[tauri::command]
pub fn set_calibration_persist(
    state: State<'_, AppState>,
    bands_l: Vec<EqBand>,
    bands_r: Vec<EqBand>,
    bypass: bool,
    fir_config: Option<String>,
    fit_id: Option<String>,
) -> Result<(), String> {
    persist_settings_after(&state, |s| {
        s.dsp.calibration = crate::state::CalibrationSettings {
            peq_bands_l: bands_l,
            peq_bands_r: bands_r,
            bypass,
            fir_config,
            fit_id,
        };
    })
    .map_err(|e| e.to_string())
}

fn is_calibration_plugin_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n == "calibration" || n == "acoustic calibration"
}

/// Restore calibration state at startup (called after load_session, when the
/// plugin scan has run). Skipped entirely when the calibration plugin is no
/// longer installed; the stale records are then cleared so a later reinstall
/// starts clean and the native PEQ is never driven by a ghost config.
pub(crate) fn restore_calibration_state(state: &State<'_, AppState>) {
    let calib = state.settings.lock().unwrap().dsp.calibration.clone();
    if calib.peq_bands_l.is_empty() && calib.peq_bands_r.is_empty() && calib.fir_config.is_none() {
        return;
    }

    let present = state
        .plugin_runtime
        .list_plugins()
        .iter()
        .any(|p| is_calibration_plugin_name(&p.manifest.name));
    if !present {
        log::info!(
            "[startup] calibration settings exist but the calibration plugin is not installed — clearing them"
        );
        if let Err(e) = persist_settings_after(state, |s| {
            s.dsp.calibration = crate::state::CalibrationSettings::default();
        }) {
            log::warn!("[startup] failed to clear stale calibration settings: {}", e);
        }
        return;
    }

    let sr = state.engine.sample_rate();
    if !calib.peq_bands_l.is_empty() || !calib.peq_bands_r.is_empty() {
        if let Err(e) = state
            .peq_handle
            .update_bands(calib.peq_bands_l.clone(), calib.peq_bands_r.clone(), sr)
        {
            // Half-restored filters would be worse than none — fail to bypass.
            log::warn!("[startup] calibration PEQ restore rejected ({e}) — bypassing");
            state
                .peq_handle
                .bypass
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
    }
    state
        .peq_handle
        .bypass
        .store(calib.bypass, std::sync::atomic::Ordering::Relaxed);

    // FIR config is keyed by plugin name in the runtime store; it takes
    // effect whenever the plugin loads (session restore may load it later).
    if let Some(cfg) = &calib.fir_config {
        state.plugin_runtime.set_plugin_config("calibration", cfg);
    }
    log::info!(
        "[startup] calibration state restored ({} L bands, bypass={})",
        calib.peq_bands_l.len(),
        calib.bypass
    );
}
