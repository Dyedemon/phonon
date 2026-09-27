/**
 * DSP API — DSP chain, Smart Effect, Surround Sound, and Equalizer.
 *
 * Wraps the `list_dsp_processors`/`set_eq_band`/... commands.
 */

import { invoke } from '@tauri-apps/api/core';
import type { TimeStretchMode, AppSettings } from './settings';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** DSP processor info (mirrors `state::DspInfo`). */
export interface DspInfo {
  id: string;
  name: string;
  enabled: boolean;
  latency_ms: number;
}

/** EQ biquad filter type (mirrors `phonon_core::eq::FilterType`). */
export type FilterType =
  | 'Peaking'
  | 'LowShelf'
  | 'HighShelf'
  | 'LowPass'
  | 'HighPass';

/** EQ band (mirrors `phonon_core::eq::EqBand`). */
export interface EqBand {
  frequency: number; // Hz
  gain_db: number; // -24..+24 dB
  q: number; // 0.1..10.0
  filter_type: FilterType;
}

/** EQ mode (mirrors `phonon_core::eq::EqMode`). */
export type EqMode = 'Graphic' | 'Parametric';

/** EQ state snapshot (mirrors `commands::EqState`). */
export interface EqState {
  bands: EqBand[];
  mode: EqMode;
  presets: string[];
  geq_band_count: number;
  preamp_db: number;
}

/** Smart Effect modes (spec: nine modes). */
export type SmartEffectMode =
  | 'off' | 'auto' | 'pop' | 'rock' | 'jazz'
  | 'classical' | 'electronic' | 'vocal' | 'bass_boost';

/** Surround Sound modes (spec: six modes). */
export type SurroundSoundMode =
  | 'off' | 'concert' | 'theater' | 'studio' | 'spacious' | 'immersive';

// ---------------------------------------------------------------------------
// DSP chain
// ---------------------------------------------------------------------------

export function listDspProcessors(): Promise<DspInfo[]> {
  return invoke('list_dsp_processors');
}

export function enableDsp(id: string): Promise<void> {
  return invoke('enable_dsp', { id });
}

export function disableDsp(id: string): Promise<void> {
  return invoke('disable_dsp', { id });
}

export function reorderDsp(from: number, to: number): Promise<void> {
  return invoke('reorder_dsp', { from, to });
}

/** Total DSP chain latency in milliseconds. */
export function getDspLatency(): Promise<number> {
  return invoke('get_dsp_latency');
}

// ---------------------------------------------------------------------------
// Smart Effect
// ---------------------------------------------------------------------------

export function getSmartEffectMode(): Promise<SmartEffectMode> {
  return invoke('get_smart_effect_mode');
}

export function setSmartEffectMode(mode: SmartEffectMode): Promise<void> {
  return invoke('set_smart_effect_mode', { mode });
}

// ---------------------------------------------------------------------------
// Surround Sound
// ---------------------------------------------------------------------------

export function getSurroundSoundMode(): Promise<SurroundSoundMode> {
  return invoke('get_surround_sound_mode');
}

export function setSurroundSoundMode(mode: SurroundSoundMode): Promise<void> {
  return invoke('set_surround_sound_mode', { mode });
}

// ---------------------------------------------------------------------------
// Wasm DSP plugins
// ---------------------------------------------------------------------------

export function addWasmDsp(name: string, id: string, latency: number): Promise<void> {
  return invoke('add_wasm_dsp', { name, id, latency });
}

export function removeWasmDsp(id: string): Promise<void> {
  return invoke('remove_wasm_dsp', { id });
}

// ---------------------------------------------------------------------------
// Equalizer
// ---------------------------------------------------------------------------

export function getEqState(): Promise<EqState> {
  return invoke('get_eq_state');
}

export function setEqBand(
  index: number,
  frequency: number,
  gainDb: number,
  q: number,
  filterType: FilterType,
): Promise<void> {
  // Tauri v2 camelCase key mapping (Rust params gain_db / filter_type).
  return invoke('set_eq_band', { index, frequency, gainDb, q, filterType });
}

export function setEqMode(mode: EqMode): Promise<void> {
  return invoke('set_eq_mode', { mode });
}

export function loadEqPreset(name: string): Promise<void> {
  return invoke('load_eq_preset', { name });
}

export function saveEqPreset(name: string): Promise<void> {
  return invoke('save_eq_preset', { name });
}

/** Returns true if the preset existed and was deleted. */
export function deleteEqPreset(name: string): Promise<boolean> {
  return invoke('delete_eq_preset', { name });
}

export function addEqBand(
  frequency: number,
  gainDb: number,
  q: number,
  filterType: FilterType,
): Promise<void> {
  return invoke('add_eq_band', { frequency, gainDb, q, filterType });
}

export function removeEqBand(index: number): Promise<void> {
  return invoke('remove_eq_band', { index });
}

export function setGeqBandCount(count: number): Promise<void> {
  return invoke('set_geq_band_count', { count });
}

export function setEqPreamp(db: number): Promise<void> {
  return invoke('set_eq_preamp', { db });
}

// ---------------------------------------------------------------------------
// PEQ (Parametric EQ) — high-precision biquad EQ
// ---------------------------------------------------------------------------

/** PEQ state snapshot (mirrors the return value of `get_peq_state`). */
export interface PeqState {
  enabled: boolean;
  bands_L: EqBand[];
  bands_R: EqBand[];
}

/** Get the current PEQ state (bands + enabled flag). */
export function getPeqState(): Promise<PeqState> {
  return invoke('get_peq_state');
}

/** Set PEQ bands for both channels. */
export function setPeqBands(bandsL: EqBand[], bandsR: EqBand[]): Promise<void> {
  return invoke('set_peq_bands', { bandsL, bandsR });
}

/** Toggle PEQ bypass (enabled = not bypassed). */
export function setPeqBypass(bypass: boolean): Promise<void> {
  return invoke('set_peq_bypass', { bypass });
}

// ---------------------------------------------------------------------------
// TimeStretch mode (变速引擎)
// ---------------------------------------------------------------------------

/**
 * 切换内置 TimeStretch 算法（Auto / WSOLA / Phase Vocoder）。
 * 走 get_settings → mutate → update_settings 路径，与 EQ 面板一致。
 * 后端 update_settings 检测到 dsp.time_stretch_mode 变化会热替换内置工厂
 * （仅在无插件 time-stretcher 时；插件加载时本调用为 no-op）。
 */
export async function setTimeStretchMode(mode: TimeStretchMode): Promise<void> {
  const settings = await invoke<AppSettings>('get_settings');
  settings.dsp = settings.dsp ?? { time_stretch_mode: 'auto' };
  settings.dsp.time_stretch_mode = mode;
  await invoke('update_settings', { settings });
}

/** 读取当前 TimeStretch 模式。 */
export async function getTimeStretchMode(): Promise<TimeStretchMode> {
  const settings = await invoke<AppSettings>('get_settings');
  return settings.dsp?.time_stretch_mode ?? 'auto';
}
