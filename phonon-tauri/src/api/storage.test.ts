/**
 * Unit tests for the unified localStorage schema manager (spec §12).
 *
 * Covers: read default, read valid, read invalid (degradation), write,
 * round-trip, range validation, and version migration.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import {
  readStorage,
  writeStorage,
  removeStorage,
  storageKeyOf,
  migrateStorage,
  getStorageVersion,
  STORAGE_VERSION,
} from './storage';

describe('storage: readStorage', () => {
  beforeEach(() => localStorage.clear());

  it('returns the default when the key is absent', () => {
    expect(readStorage('themePreset')).toBe('dark');
    expect(readStorage('accentOpacity')).toBe(1);
    expect(readStorage('bgEnabled')).toBe(true);
    expect(readStorage('customPresets')).toEqual({});
  });

  it('returns the stored value when valid', () => {
    localStorage.setItem('phonon-theme-preset', 'aurora');
    expect(readStorage('themePreset')).toBe('aurora');

    localStorage.setItem('phonon-accent-opacity', '0.5');
    expect(readStorage('accentOpacity')).toBe(0.5);

    localStorage.setItem('phonon-bg-enabled', 'false');
    expect(readStorage('bgEnabled')).toBe(false);
  });

  it('falls back to default and removes dirty entry when value is invalid (degradation)', () => {
    // Non-numeric opacity
    localStorage.setItem('phonon-accent-opacity', 'not-a-number');
    expect(readStorage('accentOpacity')).toBe(1);
    expect(localStorage.getItem('phonon-accent-opacity')).toBeNull();

    // Out-of-range opacity
    localStorage.setItem('phonon-accent-opacity', '2.5');
    expect(readStorage('accentOpacity')).toBe(1);
    expect(localStorage.getItem('phonon-accent-opacity')).toBeNull();

    // Corrupt JSON array
    localStorage.setItem('phonon-enabled-vis-scripts', '{not json');
    expect(readStorage('enabledVisScripts')).toEqual([]);
    expect(localStorage.getItem('phonon-enabled-vis-scripts')).toBeNull();

    // Non-array where array expected
    localStorage.setItem('phonon-enabled-vis-scripts', '"a string"');
    expect(readStorage('enabledVisScripts')).toEqual([]);
  });

  it('validates enum-like string values', () => {
    localStorage.setItem('phonon-volume-mode', 'Invalid');
    expect(readStorage('volumeMode')).toBe('Software');

    localStorage.setItem('phonon-volume-mode', 'Hardware');
    expect(readStorage('volumeMode')).toBe('Hardware');

    localStorage.setItem('phonon-playback-mode', 'repeat_one');
    expect(readStorage('playbackMode')).toBe('repeat_one');

    localStorage.setItem('phonon-playback-mode', 'bogus');
    expect(readStorage('playbackMode')).toBe('normal');
  });

  it('clamps font size to [10, 32]', () => {
    localStorage.setItem('phonon-font-size', '5');
    expect(readStorage('fontSize')).toBe(14);

    localStorage.setItem('phonon-font-size', '100');
    expect(readStorage('fontSize')).toBe(14);

    localStorage.setItem('phonon-font-size', '16');
    expect(readStorage('fontSize')).toBe(16);
  });
});

describe('storage: writeStorage', () => {
  beforeEach(() => localStorage.clear());

  it('serializes primitives as raw strings', () => {
    writeStorage('themePreset', 'light');
    expect(localStorage.getItem('phonon-theme-preset')).toBe('light');

    writeStorage('accentOpacity', 0.7);
    expect(localStorage.getItem('phonon-accent-opacity')).toBe('0.7');

    writeStorage('bgEnabled', false);
    expect(localStorage.getItem('phonon-bg-enabled')).toBe('false');
  });

  it('serializes objects/arrays as JSON', () => {
    writeStorage('enabledVisScripts', ['cosmos.js', 'bars.js']);
    expect(localStorage.getItem('phonon-enabled-vis-scripts')).toBe(
      JSON.stringify(['cosmos.js', 'bars.js']),
    );

    writeStorage('customPresets', { sunset: { accent: '#f00' } });
    expect(localStorage.getItem('phonon-custom-presets')).toBe(
      JSON.stringify({ sunset: { accent: '#f00' } }),
    );
  });
});

describe('storage: round-trip', () => {
  beforeEach(() => localStorage.clear());

  it('write then read returns the same value', () => {
    writeStorage('volume', 0.65);
    expect(readStorage('volume')).toBe(0.65);

    writeStorage('shortcuts', [{ id: '1', keys: 'KeyA', action: 'play' }]);
    expect(readStorage('shortcuts')).toEqual([
      { id: '1', keys: 'KeyA', action: 'play' },
    ]);
  });
});

describe('storage: removeStorage / storageKeyOf', () => {
  beforeEach(() => localStorage.clear());

  it('removes the entry', () => {
    writeStorage('themePreset', 'aurora');
    removeStorage('themePreset');
    expect(localStorage.getItem('phonon-theme-preset')).toBeNull();
    expect(readStorage('themePreset')).toBe('dark');
  });

  it('storageKeyOf returns the raw localStorage key', () => {
    expect(storageKeyOf('themePreset')).toBe('phonon-theme-preset');
    expect(storageKeyOf('enabledVisScripts')).toBe('phonon-enabled-vis-scripts');
  });
});

describe('storage: migrateStorage', () => {
  beforeEach(() => localStorage.clear());

  it('records the current version on first run', () => {
    expect(getStorageVersion()).toBe(0);
    const prev = migrateStorage();
    expect(prev).toBe(0);
    expect(getStorageVersion()).toBe(STORAGE_VERSION);
  });

  it('is a no-op when already at current version', () => {
    localStorage.setItem('phonon-storage-version', String(STORAGE_VERSION));
    const prev = migrateStorage();
    expect(prev).toBe(STORAGE_VERSION);
    expect(getStorageVersion()).toBe(STORAGE_VERSION);
  });

  it('handles a corrupt version key gracefully', () => {
    localStorage.setItem('phonon-storage-version', 'garbage');
    const prev = migrateStorage();
    expect(prev).toBe(0);
    expect(getStorageVersion()).toBe(STORAGE_VERSION);
  });
});
