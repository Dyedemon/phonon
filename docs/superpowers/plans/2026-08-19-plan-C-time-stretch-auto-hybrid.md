# C: TimeStretch Auto-Hybrid Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement 3-mode time-stretch (Auto / WSOLA / Phase Vocoder) with graceful initialization fallback, expose mode as a persistent setting, and add the UI 3-way switch next to the existing playback speed slider. When a plugin time-stretcher is loaded, it overrides the built-in selection (existing behavior preserved).

**Architecture:** Unified `time_stretch_factory(mode, speed, channels, sample_rate)` in `phonon-core/src/engine.rs` selects the optimal stretcher per decode session. `AppSettings.dsp.time_stretch_mode` (new nested struct with serde default) is persisted to disk. The decode thread applies `speed_changed` re-create logic already in the engine; we only modify the factory selection logic. UI side: the 3-way segmented control sits in `DspPanel.tsx` right of the existing speed `RangeInput`.

**Tech Stack:** Rust (timestretch crate, already in engine) + React + TS (existing components)

---

## File Structure (What Touches Where)

| File | Change | Responsibility |
|---|---|---|
| `phonon-core/src/engine.rs` | Modify (122-130, 222-288) | Add `TimeStretchMode` enum, unified `time_stretch_factory()`, fix `TimestretchStretcher` Send safety if needed, wire graceful init fallback |
| `phonon-core/tests/timestretch_pitch.rs` | Modify (add test at bottom) | Add 3 integration tests: auto mode mid-speed → WSOLA, auto extreme → phase vocoder, phase_vocoder invalid init → fallback to WSOLA |
| `phonon-tauri/src-tauri/src/state.rs` | Modify (AppSettings struct + Default impl) | Add `DspSettings { time_stretch_mode: TimeStretchMode }` nested struct with `#[serde(default)]` to `AppSettings` |
| `phonon-tauri/src/components/DspPanel.tsx` | Modify (after speed slider block) | Insert 3-way segmented control "变速引擎" with values `Auto / WSOLA / 相位声码器` |
| `phonon-tauri/src/api/dsp.ts` | Modify (getSettings / updateSettings types) | Expose `dsp.time_stretch_mode` in the typed Settings object, re-export `updateTimeStretchMode(mode)` helper |

---

### Task 1: Add TimeStretchMode enum + unified factory with fallback (phonon-core engine.rs)

**Files:**
- Modify: `phonon-core/src/engine.rs:100-290`
- Test: `phonon-core/tests/timestretch_pitch.rs`

- [ ] **Step 1: Write the failing tests for factory selection and fallback**

Append at the **bottom** of `phonon-core/tests/timestretch_pitch.rs`:

```rust
use phonon_core::engine::{TimeStretchMode, time_stretch_factory};

#[test]
fn auto_mode_selects_wsola_near_normal_speed() {
    // 0.8x - 1.25x range → prefer TimestretchStretcher (WSOLA hybrid)
    let factory = time_stretch_factory(TimeStretchMode::Auto);
    let stretcher = factory(1.0, 2, 44100);
    // Type-agnostic smoke check: push silence + flush produces >= output bytes
    let mut out = Vec::new();
    let silence = vec![0.0f32; 4096 * 2];
    stretcher.push(&silence, &mut out);
    stretcher.flush(&mut out);
    assert!(!out.is_empty(), "WSOLA auto mode failed to produce output");
}

#[test]
fn auto_mode_selects_phase_vocoder_at_extreme_speed() {
    // 0.5x (outside 0.8-1.25) → built-in Phase Vocoder
    let factory = time_stretch_factory(TimeStretchMode::Auto);
    let stretcher = factory(0.5, 2, 44100);
    let mut out = Vec::new();
    let silence = vec![0.0f32; 4096 * 2];
    stretcher.push(&silence, &mut out);
    stretcher.flush(&mut out);
    assert!(!out.is_empty(), "Phase Vocoder auto extreme failed to produce output");
}

#[test]
fn explicit_phase_vocoder_mode_works() {
    let factory = time_stretch_factory(TimeStretchMode::PhaseVocoder);
    let stretcher = factory(1.5, 2, 48000);
    let mut out = Vec::new();
    let silence = vec![0.0f32; 4096 * 2];
    stretcher.push(&silence, &mut out);
    stretcher.flush(&mut out);
    assert!(!out.is_empty(), "Phase Vocoder explicit failed");
}
```

- [ ] **Step 2: Run tests to verify they fail (enum + new factory don't exist yet)**

Run from project root `d:\Phonon`:
```bash
cargo test -p phonon-core timestretch_pitch -- --nocapture
```
Expected output: compile errors `unresolved import `phonon_core::engine::TimeStretchMode`` / `unresolved import `phonon_core::engine::time_stretch_factory``.

- [ ] **Step 3: Add enum + unified factory (insert between `default_time_stretch_factory` block L122-128 and the PluginTimeStretcher L130)**

In `phonon-core/src/engine.rs`, insert:

```rust
// ── Time-stretch mode selector ────────────────────────────────

/// User-facing time-stretch algorithm choice. Stored in AppSettings.dsp.time_stretch_mode.
///
/// - Auto (default): hybrid. 0.8x ≤ speed ≤ 1.25x → timestretch crate (WSOLA hybrid, best transient sharpness for speech/percussion). Outside range → built-in Phase Vocoder (less smearing on extreme +/− 4x). **Auto always succeeds** — if the timestretch crate throws an init panic/error at construction time, it silently falls back to the built-in Phase Vocoder on a per-call basis.
/// - Wsola: force timestretch crate for all speeds. Falls back to Phase Vocoder on init failure.
/// - PhaseVocoder: force built-in Phase Vocoder for all speeds. Zero dependencies on timestretch crate init path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]   // ← CRITICAL: TS side uses 'auto' | 'wsola' | 'phase_vocoder'.
                                        // Without this, serde default serializes PascalCase "Auto" / "Wsola" / "PhaseVocoder"
                                        // which mismatches the TS literal union → TS runtime reads wrong values.
pub enum TimeStretchMode {
    #[default]
    Auto,
    Wsola,
    PhaseVocoder,
}

/// Returns the unified factory: (speed, channels, sample_rate) → Box<dyn TimeStretch>.
///
/// **Selection rules (see `TimeStretchMode` doc comment for rationale):**
/// ```text
/// Mode | 0.8 ≤ speed ≤ 1.25        | speed < 0.8 OR speed > 1.25
/// -----+---------------------------+----------------------------
/// Auto | TimestretchStretcher (WSOLA hybrid) | TimeStretcher (phase vocoder built-in)
/// Wsola| TimestretchStretcher (FALLBACK on panic: PhaseVocoder)
/// PV   | PhaseVocoder (zero-fallback)
/// ```
///
/// **IMPORTANT — Fallback at construction time:** when `TimestretchStretcher::new` is selected
/// but panics or throws an error at init, we catch via `std::panic::catch_unwind`
/// (the timestretch crate wraps libsamplerate bindings; some Windows ARM builds can fail at
///  FFI load) and silently substitute `TimeStretcher::new` instead. This function NEVER
/// returns an error / never panics — decoder threads rely on guaranteed stretcher creation.
pub fn time_stretch_factory(mode: TimeStretchMode) -> TimeStretchFactory {
    Arc::new(move |speed, channels, sample_rate| {
        let use_wsola = match mode {
            TimeStretchMode::Auto => (0.8..=1.25).contains(&speed),
            TimeStretchMode::Wsola => true,
            TimeStretchMode::PhaseVocoder => false,
        };

        if use_wsola {
            // Wrap in catch_unwind — timestretch crate::StreamProcessor::new runs FFI-heavy
            // param validation that has been observed to panic on bad rate/ch combos in CI.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                TimestretchStretcher::new(speed, channels, sample_rate)
            }));
            match result {
                Ok(stretcher) => {
                    log::debug!(
                        "[TimeStretch] selected WSOLA hybrid (mode={:?}, speed={:.2}x, ch={}, sr={})",
                        mode, speed, channels, sample_rate
                    );
                    return Box::new(stretcher);
                }
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .unwrap_or("<non-string panic payload>");
                    log::warn!(
                        "[TimeStretch] WSOLA init FAILED ({msg}), falling back to built-in Phase Vocoder"
                    );
                }
            }
        }
        // Fallback / forced path
        log::debug!(
            "[TimeStretch] selected built-in Phase Vocoder (mode={:?}, speed={:.2}x)",
            mode, speed
        );
        Box::new(TimeStretcher::new(speed, channels as usize))
    })
}
```

Then:
- Delete the old standalone `pub fn default_time_stretch_factory() -> TimeStretchFactory` at lines 126–128 (its job is replaced by `time_stretch_factory(TimeStretchMode::Auto)`) and in its place leave a one-line re-export so existing callers don't break:
```rust
/// Backwards-compat shim — `default_time_stretch_factory()` = Auto mode.
pub fn default_time_stretch_factory() -> TimeStretchFactory { time_stretch_factory(TimeStretchMode::Auto) }
```

And **fix `TimestretchStretcher`** so it implements `Send` (it already should via `StreamProcessor: Send` — if Rust complains, add a trivial unsafe impl Send bounded by `channels: usize` + `sample_rate: u32` being Send; 99% chance this compiles fine but note it at review time).

- [ ] **Step 4: Run the new tests + existing tests → green**

```bash
cargo test -p phonon-core timestretch_pitch -- --nocapture
cargo test -p phonon-core              # run all engine tests (spectrum, pipeline, ringbuf)
```
Expected: 3 new tests + all existing tests PASS.

- [ ] **Step 5: Commit**

```bash
git add phonon-core/src/engine.rs phonon-core/tests/timestretch_pitch.rs
git commit -m "feat(C): unified TimeStretch factory with Auto/Wsola/PhaseVocoder 3-mode + fallback"
```

---

### Task 2: Wire the new setting into AppState + commands persist path (state.rs + commands.rs)

**Files:**
- Modify: `phonon-tauri/src-tauri/src/state.rs:26-123` (AppSettings) and `state.rs:125-179` (Default impl)
- Modify: `phonon-tauri/src-tauri/src/commands.rs:L1460-1475` (update_settings needs to push new mode into engine on change)

- [ ] **Step 1: Add failing compile test (build phonon-tauri with non-existent `dsp.time_stretch_mode` field)**

Actually, just confirm current AppSettings has NO `dsp` field: grep in `phonon-tauri/src-tauri/src/state.rs` for "struct AppSettings" — the current struct is flat (lines 28-123).

- [ ] **Step 2: Add `DspSettings` nested struct + AppSettings.dsp field with serde default**

Insert ABOVE `pub struct AppSettings {` (line 27).
CROSS-PLAN ORDER CONVENTION (IMPORTANT — 3 plans modify the same AppSettings, insert nested structs & fields in EXACTLY this order to avoid merge conflicts / duplicate field warnings):
  (1) DspSettings  nested struct definition  ← (this file / Plan C)
  (2) HotplugSettings nested struct definition ← (Plan B)
  (3) LibrarySettings nested struct definition ← (Plan A)
  Then inside AppSettings struct (at bottom of field list, just before `}`):
  (1) pub dsp: DspSettings,       ← C
  (2) pub hotplug: HotplugSettings, ← B
  (3) pub library: LibrarySettings, ← A
  Same order inside impl Default for AppSettings (at bottom before `}`).

```rust
/// Nested DSP-specific settings. All fields #[serde(default)] so old JSON configs
/// (saved before this struct was introduced) parse cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspSettings {
    /// Time-stretch algorithm selector used when no plugin stretcher is loaded.
    #[serde(default)]
    pub time_stretch_mode: phonon_core::engine::TimeStretchMode,
}
impl Default for DspSettings {
    fn default() -> Self { Self { time_stretch_mode: phonon_core::engine::TimeStretchMode::Auto } }
}
```

Then add a new field to the bottom of `AppSettings` (line 122, right before closing `}` of the struct):
```rust
    /// DSP sub-config (nested for forward compatibility; #[serde(default)] fills defaults for old JSONs).
    #[serde(default)]
    pub dsp: DspSettings,
```

AND add the same default line at the bottom of `impl Default for AppSettings` (line 176, right before `}`):
```rust
            dsp: DspSettings::default(),
```

- [ ] **Step 3: Wire `commands::update_settings` so setting change actually takes effect immediately (recreate factory on speed change flow)**

In `phonon-tauri/src-tauri/src/commands.rs`, inside `update_settings()` (grep `pub fn update_settings` → line ~1468), find where it sets `*state.settings.lock().unwrap() = settings.clone();`, and INSERT AFTER:

```rust
// ── C-feature: time-stretch mode change → swap built-in factory WITHOUT touching plugin ──
// We only set the built-in factory when no TimeStretch plugin is loaded.
// `get_time_stretch_factory()` returns Some only for a plugin factory; None means
// "use built-in factory" (which we replace here with the new mode-selected one).
// If a plugin IS loaded, the built-in factory is ignored anyway; leaving this branch
// ensures toggling the UI switch during plugin-loaded mode produces no state mismatch.
{
    let has_plugin_factory = state.engine.get_time_stretch_factory().is_some();
    if !has_plugin_factory {
        let new_factory = phonon_core::engine::time_stretch_factory(settings.dsp.time_stretch_mode);
        state.engine.set_time_stretch_factory(Some(new_factory));
        // Speed-change flag → next decode frame recreates stretcher with the new factory
        state.engine.speed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
        log::info!("[commands::update_settings] built-in time-stretch mode = {:?}", settings.dsp.time_stretch_mode);
    } else {
        log::info!("[commands::update_settings] plugin time-stretch loaded; ignoring UI mode = {:?}", settings.dsp.time_stretch_mode);
    }
}
```

ALSO in `state.rs AppState::new()` (line 397, where engine is created): right BEFORE returning Ok(Self{...}), push the default factory so the engine doesn't start with `None` built-in factory (today it relies on decode thread falling back; let's make it explicit):
```rust
        // Default built-in TimeStretch factory (Auto mode; plugin overrides later)
        engine.set_time_stretch_factory(Some(phonon_core::engine::time_stretch_factory(
            settings.dsp.time_stretch_mode
        )));
```

- [ ] **Step 4: Compile test (phonon-tauri src-tauri build)**

```bash
cd d:\Phonon\phonon-tauri\src-tauri
cargo check --package phonon-tauri-src-tauri 2>&1 | head -50
```
Expected: 0 errors (types for DspSettings, TimeStretchMode, serde(default) all resolve).

- [ ] **Step 5: Commit**

```bash
git add phonon-tauri/src-tauri/src/state.rs phonon-tauri/src-tauri/src/commands.rs
git commit -m "feat(C): wire dsp.time_stretch_mode into AppSettings + engine factory hot-swap"
```

---

### Task 3: Frontend 3-way switch in DspPanel + typed wrapper in api/dsp.ts

**Files:**
- Modify: `phonon-tauri/src/api/dsp.ts` (export `TimeStretchMode` union type + helpers)
- Modify: `phonon-tauri/src/components/DspPanel.tsx` (insert 3-way switch to right of speed slider)
- Modify: `phonon-tauri/src/api/settings.ts` (if AppSettings TS type is defined here, add dsp.time_stretch_mode)

- [ ] **Step 1: Add TS types + helper in api/dsp.ts**

First grep `phonon-tauri/src/api/settings.ts` for existing `AppSettings` TS interface (it should be there, since `update_settings` returns one per commands.rs L1463-1473). Edit the TS type definition (wherever `interface AppSettings` lives in `api/settings.ts`) to ADD:
```ts
export type TimeStretchMode = 'auto' | 'wsola' | 'phase_vocoder';

export interface DspSettings {
  time_stretch_mode: TimeStretchMode;
}

// Patch AppSettings (if flat currently, add the nested field):
interface AppSettings {
  // ... all existing fields untouched ...
  dsp: DspSettings;
}
```

Then append to `api/dsp.ts`:
```ts
import { invoke } from '@tauri-apps/api/core';
import type { TimeStretchMode } from './settings';

export async function setTimeStretchMode(mode: TimeStretchMode): Promise<void> {
  // Reuse get_settings → mutate → update_settings pattern (matches eq panel today)
  const settings: AppSettings = await invoke('get_settings');
  settings.dsp = settings.dsp ?? { time_stretch_mode: 'auto' };
  settings.dsp.time_stretch_mode = mode;
  await invoke('update_settings', { settings });
}
```

- [ ] **Step 2: Add 3-way segmented control in DspPanel.tsx — right of playback speed slider**

Find the speed section in `phonon-tauri/src/components/DspPanel.tsx` (grep for `speed` / `setSpeed` / `RangeInput`). After the speed slider row, add:

```tsx
import type { TimeStretchMode } from '../api/settings';
import { setTimeStretchMode } from '../api/dsp';

// ── Inside DSP panel JSX, after speed slider: ──
<div className="dsp-row dsp-row-time-stretch">
  <span className="dsp-label">变速引擎</span>
  <div className="segmented-control" role="tablist">
    {([
      ['auto',           '自动',   '0.8x – 1.25x 瞬态优先，其他倍率抗涂抹'],
      ['wsola',          'WSOLA',  '全速率时域，打击乐/人声最锐利'],
      ['phase_vocoder',  '相位',   '极端变速 0.25x / 4x 更稳定'],
    ] as const).map(([val, label, hint]) => (
      <button
        key={val}
        role="tab"
        aria-selected={settings?.dsp?.time_stretch_mode === val}
        title={hint}
        onClick={async () => { await setTimeStretchMode(val as TimeStretchMode); await refreshSettings(); }}
      >
        {label}
      </button>
    ))}
  </div>
</div>
```

Note: reuse existing `.segmented-control` class if already present (check `DspPanel.tsx` for EQ band / mode toggle). If the CSS class doesn't exist, append 10 lines to `App.css` — minimal style: 3 pills, 4px radius, active = `background: var(--accent)`, text-white; reuse project tokens.

- [ ] **Step 3: Vitest smoke-run + Playwright screen check**

```bash
cd d:\Phonon\phonon-tauri
npm run test
npx vitest run
```
Expected: 0 failures. Open the app dev server briefly and visually confirm: the 3-way switch renders next to the speed slider; click WSOLA → round-trips → invokes `update_settings` → no console errors.

- [ ] **Step 4: Commit**

```bash
git add phonon-tauri/src/api/dsp.ts phonon-tauri/src/api/settings.ts phonon-tauri/src/components/DspPanel.tsx
git add phonon-tauri/src/App.css  # only if .segmented-control needed additions
git commit -m "feat(C): add 变速引擎 3-way UI switch next to speed slider"
```

---

## Self-Review Checklist (run yourself before handoff, or CI will)

- [ ] Spec §3 coverage: 3-mode enum ✓, Auto hybrid threshold 0.8-1.25x ✓, init panic fallback to PV ✓, plugin overrides setting (no-op when plugin loaded) ✓
- [ ] Spec §6 coverage: nested `dsp.time_stretch_mode` with serde default (old JSON auto-fills `auto`) ✓
- [ ] Spec §8.1 coverage (C acceptance #4): "设置 → DSP 面板→变速引擎三态切换 → 立即生效，无重启" ✓
- [ ] Placeholder scan: 0 TODO / TBD in inserted code blocks
- [ ] Type consistency: TS `TimeStretchMode` = 'auto' | 'wsola' | 'phase_vocoder' matches Rust enum Serialize output (lowercase variants match serde default — verify; if Rust serde serializes uppercase "Auto", add `#[serde(rename_all = "snake_case")]` to Rust `TimeStretchMode` enum)

---

## Next (after C done → go to Plan B: Hotplug)
