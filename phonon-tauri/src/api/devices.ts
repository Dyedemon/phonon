/**
 * Devices API — audio device enumeration, selection, and exclusive mode.
 *
 * Wraps the `list_devices`/`set_device`/... Tauri commands.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  AppSettings,
  DeviceInsertedStrategy,
  DeviceRemovedStrategy,
  HotplugSettings,
} from './settings';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Audio device info (mirrors `commands::DeviceInfoJson`). */
export interface DeviceInfo {
  name: string;
  id: string;
  is_default: boolean;
  /** Supported sample rates in Hz. */
  sample_rates: number[];
  max_channels: number;
  supports_exclusive: boolean;
}

/** Aggregated hotplug status for the Settings UI card. */
export interface HotplugStatus {
  strategy: {
    on_device_removed: DeviceRemovedStrategy;
    on_new_device_inserted: DeviceInsertedStrategy;
  };
  last_used_device_id: string | null;
  current_device_id: string | null;
  devices: Array<{ id: string; name: string; is_default: boolean }>;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/** List all available audio output devices. */
export function listDevices(): Promise<DeviceInfo[]> {
  return invoke('list_devices');
}

/** Set the active output device by name.
 *
 * Tauri v2 matches JS keys to Rust params via snake→camelCase conversion,
 * so the key must be `deviceId` (Rust `device_id`), NOT `device_id`.
 */
export function setDevice(deviceId: string): Promise<void> {
  return invoke('set_device', { deviceId });
}

/** Get the currently active device, or null if none. */
export function getCurrentDevice(): Promise<DeviceInfo | null> {
  return invoke('get_current_device');
}

/** Query whether exclusive mode is enabled. */
export function getExclusiveMode(): Promise<boolean> {
  return invoke('get_exclusive_mode');
}

/** Enable or disable WASAPI/CoreAudio exclusive mode. */
export function setExclusiveMode(enabled: boolean): Promise<void> {
  return invoke('set_exclusive_mode', { enabled });
}

/** Start the device hotplug monitor (polls for device changes). */
export function startHotplugMonitor(): Promise<void> {
  return invoke('start_hotplug_monitor');
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

export function getHotplugNotifications(): Promise<boolean> {
  return invoke('get_hotplug_notifications');
}

export function setHotplugNotifications(enabled: boolean): Promise<void> {
  return invoke('set_hotplug_notifications', { enabled });
}

// ---------------------------------------------------------------------------
// Hotplug strategies (Plan B) — get_settings → mutate → update_settings
// ---------------------------------------------------------------------------

const DEFAULT_HOTPLUG: HotplugSettings = {
  on_device_removed: 'pause',
  on_new_device_inserted: 'ignore',
  last_used_device_id: null,
};

/** Set the device-removed strategy. Mutates settings.hotplug.on_device_removed. */
export async function setOnDeviceRemoved(v: DeviceRemovedStrategy): Promise<void> {
  const s = await invoke<AppSettings>('get_settings');
  s.hotplug = s.hotplug ?? { ...DEFAULT_HOTPLUG };
  s.hotplug.on_device_removed = v;
  await invoke('update_settings', { settings: s });
}

/** Set the device-inserted strategy. Mutates settings.hotplug.on_new_device_inserted. */
export async function setOnNewDeviceInserted(v: DeviceInsertedStrategy): Promise<void> {
  const s = await invoke<AppSettings>('get_settings');
  s.hotplug = s.hotplug ?? { ...DEFAULT_HOTPLUG };
  s.hotplug.on_new_device_inserted = v;
  await invoke('update_settings', { settings: s });
}

/** Fetch an aggregated hotplug status snapshot for the Settings card. */
export async function getHotplugStatus(): Promise<HotplugStatus> {
  const [s, devices] = await Promise.all([
    invoke<AppSettings>('get_settings'),
    invoke<DeviceInfo[]>('list_devices'),
  ]);
  const current = devices.find((d) => d.is_default)?.id ?? devices[0]?.id ?? null;
  const hp = s.hotplug ?? DEFAULT_HOTPLUG;
  return {
    strategy: {
      on_device_removed: hp.on_device_removed,
      on_new_device_inserted: hp.on_new_device_inserted,
    },
    last_used_device_id: hp.last_used_device_id ?? null,
    current_device_id: current,
    devices: devices.map((d) => ({ id: d.id, name: d.name, is_default: d.is_default })),
  };
}

/**
 * DevTools / QA harness: inject a synthetic hotplug event without physical
 * plug/unplug. Backed by the `debug_inject_hotplug_event` Tauri command
 * (dev-only — release builds reject the call).
 */
export async function debugInjectHotplug(
  kind: 'added' | 'removed' | 'default_changed',
  deviceId: string,
): Promise<void> {
  await invoke('debug_inject_hotplug_event', { kind, deviceId });
}

/** Host OS ("windows" | "macos" | "linux" | ...), cached after first call. */
let cachedPlatform: string | null = null;
export async function getPlatform(): Promise<string> {
  if (cachedPlatform) return cachedPlatform;
  cachedPlatform = await invoke<string>('get_platform');
  return cachedPlatform;
}

