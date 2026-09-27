/**
 * Unified API entry point — re-exports all domain modules.
 *
 * Per spec §12, the `api/` directory is split into domain files:
 *   player / devices / dsp / queue / settings / library / plugins / events
 *
 * Components should import from `../api` rather than calling `invoke`
 * directly, so that argument naming and types are centralized.
 *
 * ARG NAMING: Tauri v2 maps Rust snake_case params to camelCase JS keys
 * (e.g. Rust `device_id` ↔ JS `deviceId`). Single-word params are
 * unaffected. All wrappers here pass camelCase keys.
 *
 * Usage:
 *   import { player, library } from '../api';
 *   await player.play();
 *   const tracks = await library.getTracks();
 *
 * Or import a single module directly:
 *   import { useListen } from '../api/events';
 */

export * as player from './player';
export * as devices from './devices';
export * as dsp from './dsp';
export * as queue from './queue';
export * as settings from './settings';
export * as library from './library';
export * as plugins from './plugins';
export * as events from './events';
export * as storage from './storage';
