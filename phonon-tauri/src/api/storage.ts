/**
 * Unified localStorage schema manager (spec §12).
 *
 * Replaces the 50+ scattered `localStorage.getItem('phonon-...')` calls with
 * a single typed registry. Each key declares:
 *   - a default value
 *   - an optional validator (parse + sanity-check)
 *   - an optional migrator (for schema version bumps)
 *
 * On read: if the stored value fails validation (corrupt JSON, wrong type,
 * out-of-range number), it is treated as missing — the default is returned
 * and the dirty entry is removed. This is the "降级" (graceful degradation)
 * requirement.
 *
 * On version bump: `migrate()` runs all migrators whose source version is
 * below the current `STORAGE_VERSION`, then bumps the recorded version.
 */

// ---------------------------------------------------------------------------
// Schema version.
// ---------------------------------------------------------------------------

/** Current localStorage schema version. Bump when changing key shapes. */
export const STORAGE_VERSION = 2;

const VERSION_KEY = 'phonon-storage-version';

// ---------------------------------------------------------------------------
// Type-safe storage entry descriptor.
// ---------------------------------------------------------------------------

/** Storage entry schema descriptor. */
type StorageEntry<T> = {
  /** localStorage key (must be unique across the registry). */
  key: string;
  /** Default value used when the key is absent or fails validation. */
  default: T;
  /**
   * Parse a raw string into `T` and validate it. Return `null` to signal
   * "invalid — fall back to default". Throwing is also treated as invalid.
   */
  parse?: (raw: string) => T | null;
  /** Serialize `T` for storage. Defaults to `JSON.stringify`. */
  serialize?: (value: T) => string;
  /**
   * Migrate from a prior raw representation. Receives the raw string and the
   * schema version it was written under; returns the new raw string (or null
   * to discard). Only invoked by `migrateStorage()`, not on regular reads.
   */
  migrate?: (raw: string, fromVersion: number) => string | null;
};

// ---------------------------------------------------------------------------
// Validator helpers.
// ---------------------------------------------------------------------------

/** Parse JSON; return null on failure (never throws). */
function parseJsonOrNull(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/** Validate a value is a string; return null otherwise. */
function asString(raw: string): string | null {
  // Strings are stored raw (no JSON) for backward compat with existing keys.
  return raw.length > 0 ? raw : null;
}

/** Validate a value is a finite number; return null otherwise. */
function asNumber(raw: string): number | null {
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

/** Validate a value is a number within [min, max]; return null otherwise. */
function asNumberInRange(raw: string, min: number, max: number): number | null {
  const n = asNumber(raw);
  return n !== null && n >= min && n <= max ? n : null;
}

/** Validate a value is a boolean stored as 'true'/'false'. */
function asBool(raw: string): boolean | null {
  if (raw === 'true') return true;
  if (raw === 'false') return false;
  return null;
}

/** Validate a value is a JSON array; return null otherwise. */
function asJsonArray<T>(raw: string): T[] | null {
  const v = parseJsonOrNull(raw);
  return Array.isArray(v) ? (v as T[]) : null;
}

/** Validate a value is a JSON object; return null otherwise. */
function asJsonObject<T>(raw: string): T | null {
  const v = parseJsonOrNull(raw);
  return v !== null && typeof v === 'object' && !Array.isArray(v) ? (v as T) : null;
}

// ---------------------------------------------------------------------------
// Registry — the single source of truth for every phonon-* localStorage key.
// ---------------------------------------------------------------------------

/** Every known localStorage key with its type, default, and validator. */
const REGISTRY: { [name: string]: StorageEntry<any> } = {
  // ── Appearance / theme ──────────────────────────────────────────────
  themePreset: {
    key: 'phonon-theme-preset',
    default: 'dark',
    parse: (r: string) => (r.length > 0 ? r : null),
  },
  accentColor: { key: 'phonon-accent-color', default: '', parse: asString },
  bgColor: { key: 'phonon-bg-color', default: '', parse: asString },
  cardBgColor: { key: 'phonon-card-bg-color', default: '', parse: asString },
  borderColor: { key: 'phonon-border-color', default: '', parse: asString },
  textColor: { key: 'phonon-text-color', default: '', parse: asString },
  textBrightColor: { key: 'phonon-text-bright-color', default: '', parse: asString },
  textDimColor: { key: 'phonon-text-dim-color', default: '', parse: asString },
  placeholderColor: { key: 'phonon-placeholder-color', default: '', parse: asString },

  accentOpacity: {
    key: 'phonon-accent-opacity',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  bgOpacity: {
    key: 'phonon-bg-opacity',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  cardBgOpacity: {
    key: 'phonon-cardbg-opacity',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  borderOpacity: {
    key: 'phonon-border-opacity',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  textDimOpacity: {
    key: 'phonon-text-dim-opacity',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  placeholderOpacity: {
    key: 'phonon-placeholder-opacity',
    default: 0.5,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },

  albumArtRadius: { key: 'phonon-album-art-radius', default: '', parse: asString },
  albumArtBlur: { key: 'phonon-album-art-blur', default: '', parse: asString },

  customCss: { key: 'phonon-custom-css', default: '', parse: asString },
  customPresets: {
    key: 'phonon-custom-presets',
    // Actual stored format is Record<string, {...}> (object map keyed by
    // preset name), NOT an array. Matches Settings.tsx's PresetColors shape.
    default: {} as Record<string, Record<string, string | number>>,
    parse: (r: string) => asJsonObject(r),
  },

  fontSize: {
    key: 'phonon-font-size',
    default: 14,
    parse: (r: string) => asNumberInRange(r, 10, 32),
  },
  fontFamily: { key: 'phonon-font-family', default: '', parse: asString },

  bgEnabled: {
    key: 'phonon-bg-enabled',
    default: true,
    parse: asBool,
  },
  bgImages: {
    key: 'phonon-bg-images',
    default: [] as string[],
    parse: (r: string) => asJsonArray<string>(r),
  },
  bgImageIndex: {
    key: 'phonon-bg-image-index',
    default: 0,
    parse: (r: string) => asNumberInRange(r, 0, 9999),
  },
  bgImageData: { key: 'phonon-bg-image-data', default: '', parse: asString },
  bgImageUrl: { key: 'phonon-bg-image-url', default: '', parse: asString },

  // ── Playback / volume ───────────────────────────────────────────────
  volume: {
    key: 'phonon-volume',
    default: 1,
    parse: (r: string) => asNumberInRange(r, 0, 1),
  },
  volumeMode: {
    key: 'phonon-volume-mode',
    default: 'Software',
    parse: (r: string) => (r === 'Software' || r === 'Hardware' ? r : null),
  },
  playbackMode: {
    key: 'phonon-playback-mode',
    default: 'normal',
    parse: (r: string) =>
      ['normal', 'repeat_all', 'repeat_one', 'shuffle'].includes(r) ? r : null,
  },

  // ── Spectrum ────────────────────────────────────────────────────────
  spectrumColorscheme: {
    key: 'phonon-spectrum-colorscheme',
    default: 'aurora',
    parse: (r: string) => (['aurora', 'warm', 'cool', 'mono'].includes(r) ? r : null),
  },
  spectrumCustomColors: {
    key: 'phonon-spectrum-custom-colors',
    default: [] as string[],
    parse: (r: string) => asJsonArray<string>(r),
  },

  // ── Visualization ───────────────────────────────────────────────────
  visMode: {
    key: 'phonon-vis-mode',
    default: 'none',
    parse: (r: string) => (r.length > 0 ? r : null),
  },
  enabledVisScripts: {
    key: 'phonon-enabled-vis-scripts',
    default: [] as string[],
    parse: (r: string) => asJsonArray<string>(r),
  },
  enabledPlugins: {
    key: 'phonon-enabled-plugins',
    default: [] as string[],
    parse: (r: string) => asJsonArray<string>(r),
  },

  // ── Smart effect ────────────────────────────────────────────────────
  smartEffectAuto: {
    key: 'phonon-smart-effect-auto',
    default: true,
    parse: asBool,
  },

  // ── Shortcuts ───────────────────────────────────────────────────────
  shortcuts: {
    key: 'phonon_shortcuts',
    default: [] as Array<{ id: string; keys: string; action: string }>,
    parse: (r: string) => asJsonArray(r),
  },

  // ── Legal / EULA ───────────────────────────────────────────────────
  /**
   *  Whether the user has accepted the End-User License Agreement.
   *  Defaults to `false` so the first-launch modal is shown. Once accepted
   *  (and persisted), the modal never re-appears unless the user clicks
   *  "Review EULA again" in Settings → About.
   */
  eulaAccepted: {
    key: 'phonon-eula-accepted',
    default: false,
    parse: asBool,
  },

  // ── Network / API whitelist gates (§14 合规) ──────────────────────
  /**
   * 用户是否主动启用在线歌词搜索 API。
   * 默认 false —— 未启用时歌词 API URL 字段在 UI 中置灰，前端不会发起任何外部请求。
   * 必须由用户显式打开才会激活（合规白名单策略）。
   */
  lyricsApiEnabled: {
    key: 'phonon-lyrics-api-enabled',
    default: false,
    parse: asBool,
  },
};

/** Union of all registered storage keys (for type-safe access). */
export type StorageKey = keyof typeof REGISTRY;

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

/**
 * Read a typed value from localStorage. Falls back to the registered default
 * if the key is missing or fails validation; in the latter case the dirty
 * entry is also removed (degradation).
 */
export function readStorage<K extends StorageKey>(name: K): (typeof REGISTRY)[K]['default'] {
  const entry = REGISTRY[name];
  const raw = localStorage.getItem(entry.key);
  if (raw === null) return entry.default;
  try {
    const parsed = entry.parse ? entry.parse(raw) : (raw as unknown as typeof entry.default);
    if (parsed === null || parsed === undefined) {
      localStorage.removeItem(entry.key);
      return entry.default;
    }
    return parsed as typeof entry.default;
  } catch {
    localStorage.removeItem(entry.key);
    return entry.default;
  }
}

/**
 * Write a typed value to localStorage. Uses the entry's serializer (or
 * `JSON.stringify` for objects/arrays, raw `String()` for primitives).
 */
export function writeStorage<K extends StorageKey>(name: K, value: (typeof REGISTRY)[K]['default']): void {
  const entry = REGISTRY[name];
  const serialized =
    typeof value === 'string'
      ? (value as string)
      : typeof value === 'number' || typeof value === 'boolean'
        ? String(value)
        : JSON.stringify(value);
  localStorage.setItem(entry.key, serialized);
}

/** Remove a storage entry. */
export function removeStorage<K extends StorageKey>(name: K): void {
  localStorage.removeItem(REGISTRY[name].key);
}

/** Return the raw localStorage key string for a registered entry. */
export function storageKeyOf<K extends StorageKey>(name: K): string {
  return REGISTRY[name].key;
}

// ---------------------------------------------------------------------------
// Schema migration.
// ---------------------------------------------------------------------------

/**
 * Run any pending schema migrations and bump the recorded version.
 *
 * Reads the previously-stored version; if it's below `STORAGE_VERSION`,
 * invokes each entry's `migrate` hook (if any) with the old version,
 * then writes the new version. Safe to call on every app start — it's a
 * no-op when the version is already current.
 *
 * Returns the previous version (0 if first run) for diagnostics.
 */
export function migrateStorage(): number {
  const raw = localStorage.getItem(VERSION_KEY);
  const parsed = raw !== null ? Number(raw) : 0;
  // Treat missing/NaN/corrupt version as 0 (first run).
  const prev = Number.isFinite(parsed) ? parsed : 0;
  if (prev >= STORAGE_VERSION) {
    // Already current — ensure the version key is a clean number.
    if (raw !== String(STORAGE_VERSION)) {
      localStorage.setItem(VERSION_KEY, String(STORAGE_VERSION));
    }
    return prev;
  }

  // Run migrators. Each migrator receives the raw old value and the old
  // version; it returns the new raw string or null to discard.
  for (const name of Object.keys(REGISTRY) as StorageKey[]) {
    const entry = REGISTRY[name];
    if (!entry.migrate) continue;
    const oldRaw = localStorage.getItem(entry.key);
    if (oldRaw === null) continue;
    try {
      const migrated = entry.migrate(oldRaw, prev);
      if (migrated === null) {
        localStorage.removeItem(entry.key);
      } else {
        localStorage.setItem(entry.key, migrated);
      }
    } catch {
      // Migrator failed — drop the dirty value; default will be used.
      localStorage.removeItem(entry.key);
    }
  }

  localStorage.setItem(VERSION_KEY, String(STORAGE_VERSION));
  return prev;
}

/** Return the recorded storage version (0 if unset or corrupt). */
export function getStorageVersion(): number {
  const raw = localStorage.getItem(VERSION_KEY);
  const v = raw !== null ? Number(raw) : 0;
  return Number.isFinite(v) ? v : 0;
}
