/**
 * Calibration API — acoustic calibration via external WASM plugin.
 *
 * The calibration plugin provides:
 * 1. FIR residual convolution DSP (realtime, loaded in the DSP chain after PEQ)
 * 2. Biquad + FIR fit computation via plugin_command("run_fit", ...)
 * 3. Target curve lookup via plugin_command("get_target", ...)
 *
 * The plugin is fully optional: the main app has no build-time dependency on
 * the calibration crates. Everything here assumes the plugin is present and
 * loaded (the panel checks via listPlugins before rendering); removing the
 * plugin simply leaves these functions uncalled.
 */

import { invoke } from '@tauri-apps/api/core';
import { callPluginCommand, setPluginConfig, getPluginConfig } from './plugins';
import {
  getPeqState,
  setPeqBands,
  setPeqBypass,
  listDspProcessors,
  type PeqState,
  type EqBand,
} from './dsp';
import { getOutputSampleRate } from './settings';

const PLUGIN_NAME = 'calibration';
/** DSP chain id the FIR processor is registered under (PluginManager convention). */
const FIR_CHAIN_ID = 'wasm_calibration';

// ---------------------------------------------------------------------------
// Types (mirror the current calibration-types crate ABI)
// ---------------------------------------------------------------------------

/** 256-point log-spaced curve on the standard grid (freq[0]=20Hz, freq[255]=20kHz). */
export interface FitCurves {
  freq: number[];
  mag_db: number[];
}

/**
 * Fit request (mirrors calibration_types::FitRequest). The engine cleans and
 * resamples the raw measurement onto its internal 256-pt grid by itself.
 */
export interface FitRequest {
  /** Raw measurement samples (Hz, dB) — unordered, out-of-band points allowed. */
  measurement: [number, number][];
  /** Target curve id from the engine registry (see TARGET_CURVES). */
  target_curve_id: string;
  /** Optional user-supplied target; overrides the registry entry when present. */
  custom_target: FitCurves | null;
  /** Maximum number of biquad bands to fit (engine range 12..20). */
  max_bands: number;
  /** FIR residual filter length in taps (engine range 64..8192). */
  fir_taps: number;
  /** Running sample rate used to design biquad coefficients. */
  sample_rate: number;
}

/** Fit result (mirrors calibration_types::FitResult). */
export interface FitResult {
  fit_id: string;
  target_curve_id: string;
  bands_L: EqBand[];
  bands_R: EqBand[];
  fir_L: number[];
  fir_R: number[];
  /** Fitted response on the standard 256-pt log grid (UI canvas). */
  curves_L: FitCurves;
  curves_R: FitCurves;
  rms_error_db: number;
  peak_error_db: number;
  /** serde SystemTime shape ({secs_since_epoch, nanos_since_epoch}) when applied. */
  applied_at: { secs_since_epoch: number; nanos_since_epoch: number } | null;
}

/** FIR config JSON sent to the plugin via setPluginConfig (version-poll pull). */
export interface FirConfig {
  sample_rate: number;
  taps: number;
  fir_L: number[];
  fir_R: number[];
  bypass: boolean;
}

/** Target curve presets exposed by the panel (ids from the engine registry). */
export interface TargetCurvePreset {
  id: string;
  /** i18n key suffix under `calibration.target.` */
  labelKey: string;
}

/**
 * Registry ids understood by calibration-engine::curves::resolve_target.
 * "平直" maps to the engine's diffuse-field-flat table, which is a 0 dB line —
 * using engine ids (instead of computing the 256-pt log grid in JS) keeps the
 * displayed target bit-identical to what the engine fits against.
 */
export const TARGET_CURVES: TargetCurvePreset[] = [
  { id: 'diffuse_field_flat', labelKey: 'flat' },
  { id: 'harman_ie_2018_v2', labelKey: 'harmanIE2018' },
  { id: 'harman_oe_2019_v2', labelKey: 'harmanOE2019' },
  { id: 'harman_ie_2019_final', labelKey: 'harmanIE2019' },
  { id: 'free_field_flat', labelKey: 'freeField' },
];

// ---------------------------------------------------------------------------
// Fit computation (runs inside the WASM plugin, not the host)
// ---------------------------------------------------------------------------

/** Run a calibration fit through the plugin's non-realtime command channel. */
export async function runCalibrationFit(request: FitRequest): Promise<FitResult> {
  const outputJson = await callPluginCommand(PLUGIN_NAME, 'run_fit', JSON.stringify(request));
  return JSON.parse(outputJson) as FitResult;
}

/**
 * Fetch a target curve on the standard 256-pt log grid, exactly as the engine
 * resolves it for fitting (single source of truth — no JS-side grid math).
 */
export async function getTargetCurve(targetCurveId: string): Promise<FitCurves> {
  const outputJson = await callPluginCommand(
    PLUGIN_NAME,
    'get_target',
    JSON.stringify({ target_curve_id: targetCurveId }),
  );
  return JSON.parse(outputJson) as FitCurves;
}

/** Sample rate to design filters for — the live engine output rate. */
export async function calibrationSampleRate(): Promise<number> {
  try {
    const sr = await getOutputSampleRate();
    if (sr && sr > 0) return sr;
  } catch {
    // engine not running — fall through to the default
  }
  return 48000;
}

// ---------------------------------------------------------------------------
// Apply / bypass
// ---------------------------------------------------------------------------

/** Clamp bands into the native PEQ node's validation envelope (±24 dB, Q 0.1..10). */
function clampPeqBands(bands: EqBand[]): EqBand[] {
  return bands
    .filter(b => Number.isFinite(b.frequency) && b.frequency >= 10 && b.frequency <= 96000)
    .map(b => ({
      frequency: Math.min(96000, Math.max(10, b.frequency)),
      gain_db: Math.min(24, Math.max(-24, b.gain_db)),
      q: Math.min(10, Math.max(0.1, b.q)),
      filter_type: b.filter_type,
    }));
}

/** Ensure the FIR wasm processor is in the DSP chain (idempotent). */
async function ensureFirInChain(): Promise<void> {
  const procs = await listDspProcessors();
  if (procs.some(p => p.id === FIR_CHAIN_ID)) return;
  await invoke('add_wasm_dsp', { name: PLUGIN_NAME, id: FIR_CHAIN_ID, latency: 0.0 });
}

/**
 * Apply a fit result:
 * 1. PEQ biquad bands via the native peq node (immediate, next buffer)
 * 2. FIR residual coefficients pushed as plugin config (picked up on the
 *    plugin's next config-version poll, ~1 block)
 * 3. FIR processor added to the chain when missing (auto-loads the plugin)
 * 4. State persisted to settings.json so it survives restarts
 */
export async function applyCalibrationResult(
  bandsL: EqBand[],
  bandsR: EqBand[],
  firL: number[],
  firR: number[],
  fitId: string,
  sampleRate: number,
): Promise<void> {
  const clampedL = clampPeqBands(bandsL);
  const clampedR = clampPeqBands(bandsR);

  await setPeqBands(clampedL, clampedR);
  await setPeqBypass(false);

  const firConfig: FirConfig = {
    sample_rate: sampleRate,
    taps: firL.length,
    fir_L: firL,
    fir_R: firR,
    bypass: false,
  };
  await ensureFirInChain();
  await setPluginConfig(PLUGIN_NAME, JSON.stringify(firConfig));

  await invoke('set_calibration_persist', {
    bandsL: clampedL,
    bandsR: clampedR,
    bypass: false,
    firConfig: JSON.stringify(firConfig),
    fitId,
  });
}

/**
 * Bypass calibration: PEQ skip + FIR passthrough config, persisted so the
 * bypassed state survives restarts (bands are kept for a quick re-apply).
 */
export async function bypassCalibration(): Promise<void> {
  await setPeqBypass(true);
  const sr = await calibrationSampleRate();
  const firConfig: FirConfig = {
    sample_rate: sr,
    taps: 0,
    fir_L: [],
    fir_R: [],
    bypass: true,
  };
  const firJson = JSON.stringify(firConfig);
  await setPluginConfig(PLUGIN_NAME, firJson);

  // Keep the last bands on record; persistConfig refreshes them.
  let bandsL: EqBand[] = [];
  let bandsR: EqBand[] = [];
  try {
    const peq = await getPeqState();
    bandsL = peq.bands_L;
    bandsR = peq.bands_R;
  } catch {
    // PEQ node unreachable — persist the bypass with empty bands
  }
  await invoke('set_calibration_persist', {
    bandsL,
    bandsR,
    bypass: true,
    firConfig: firJson,
    fitId: null,
  });
}

/** Current calibration state (PEQ snapshot + FIR plugin config). */
export async function getCalibrationState(): Promise<{
  peq: PeqState;
  firConfig: FirConfig | null;
}> {
  const [peq, firRaw] = await Promise.all([getPeqState(), getPluginConfig(PLUGIN_NAME)]);
  let firConfig: FirConfig | null = null;
  if (firRaw) {
    try {
      firConfig = JSON.parse(firRaw) as FirConfig;
    } catch {
      firConfig = null;
    }
  }
  return { peq, firConfig };
}

// ---------------------------------------------------------------------------
// Measurement file parsing (REW-style TXT / CSV / JSON)
// ---------------------------------------------------------------------------

export interface MeasurementCurve {
  frequencies: number[];
  db: number[];
}

/**
 * Parse a measurement file: `freq  dB` per line, separators = whitespace /
 * comma / semicolon / tab; `*` and `#` lines are comments (REW convention).
 */
export function parseMeasurement(text: string): MeasurementCurve {
  const lines = text.split('\n').filter(l => l.trim() && !l.startsWith('*') && !l.startsWith('#'));
  const freqs: number[] = [];
  const dbs: number[] = [];
  for (const line of lines) {
    const parts = line.trim().split(/[\s,;\t]+/);
    if (parts.length >= 2) {
      const f = parseFloat(parts[0]);
      const db = parseFloat(parts[1]);
      if (!isNaN(f) && !isNaN(db) && f > 0) {
        freqs.push(f);
        dbs.push(db);
      }
    }
  }
  return { frequencies: freqs, db: dbs };
}

/** Convert a parsed curve into the (Hz, dB) pair list the fit engine expects. */
export function toMeasurementPairs(curve: MeasurementCurve): [number, number][] {
  return curve.frequencies.map((f, i) => [f, curve.db[i] ?? 0]);
}
