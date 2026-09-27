# Calibration plugin — parked assets

Acoustic calibration is **on hold**. This directory is where its app-side code
went so the main app stays clean: nothing here is wired into `phonon-tauri`, it
is a storage/reference folder only.

Nothing in this directory affects the build. It contains no `Cargo.toml`, so it
is not a cargo workspace member, and it sits outside `phonon-tauri/tsconfig`
include paths, so `tsc` ignores it.

## Why parked

- The calibration panel framework (the "声学校准" card) was removed from the
  app UI. Even without the plugin installed it showed a placeholder card, and
  the project is deferred, so the card should not occupy the DSP page.
- Keeping the code here — next to the plugin it belongs to — means resuming is
  a matter of copying files back, not reconstructing them.

## Layout

| Path | What it is |
|---|---|
| `calibration.wasm` | Backup of the deployed plugin binary. The live copy is `phonon-tauri/plugins/calibration.wasm` (gitignored — this is the only copy in git). |
| `ui/src/components/CalibrationPanel.tsx` | The calibration panel (measurement import, target curve, fit params, diagnostics, apply/bypass). |
| `ui/src/components/calibration/CurveCanvas.tsx` | Canvas renderer for the measured / target / fitted / residual curves. |
| `ui/src/api/calibration.ts` | TS API layer: `run_fit` + `get_target` plugin commands, PEQ + FIR apply, bypass, persistence, measurement parsing. |
| `ui/snippets/i18n-calibration-keys.ts` | The `calibration.*` + `plugins.calibration.*` keys (zh + en, 84 lines) to paste into `i18n.ts`. |
| `ui/snippets/app-calibration.css` | The `.calibration-*` CSS block (85 lines) to append to `App.css`. |
| `ui/snippets/backend-state.rs` | `CalibrationSettings` struct for `AppSettings.dsp` in `state.rs`. |
| `ui/snippets/backend-commands.rs` | `set_calibration_persist` command + `restore_calibration_state` startup hook. |
| `ui/snippets/backend-lib-registration.txt` | `generate_handler!` registration lines. |
| `ui/snippets/backend-lib-startup.txt` | The startup restore block (called after `load_session`). |
| `ui/snippets/pluginmanager-carveout.ts` | `isCalibrationPlugin` helper — used to hide the plugin card's 删除 button. |

## Restoring it

The complete, working integration is preserved in the **`校准插件` branch**,
commit `53a800c` (with `7a9abcb` underneath). That is faster and less
error-prone than re-assembling from the snippets:

```bash
git checkout 校准插件 -- phonon-tauri/src/components/CalibrationPanel.tsx \
                        phonon-tauri/src/components/calibration/CurveCanvas.tsx \
                        phonon-tauri/src/api/calibration.ts
# then re-apply the small integration edits listed below
```

Or bring the whole branch over and diff against it:
`git diff main 校准插件 -- phonon-tauri phonon-core`

## Integration points (checklist for a re-integration)

1. `phonon-tauri/src/components/DspPanel.tsx` — import `CalibrationPanel` and
   render `<CalibrationPanel />` just above `<EqPanel … />`.
2. `phonon-tauri/src/i18n.ts` — paste the `ui/snippets/i18n-calibration-keys.ts`
   block into **both** the `zh` and `en` dictionaries.
3. `phonon-tauri/src/App.css` — append `ui/snippets/app-calibration.css`.
4. `phonon-tauri/src/components/PluginManager.tsx` — add `isCalibrationPlugin`,
   and use it to (a) hide the 删除 button and (b) show a description + a
   "前往 DSP 页校准" jump button. Also keep calibration out of `isDspPlugin` so
   the card does not offer a meaningless "+DSP" button.
5. Backend (`phonon-tauri/src-tauri/src/`): add `CalibrationSettings` to
   `state.rs`, the two functions from `backend-commands.rs`, the
   `generate_handler!` line, and the startup call after `load_session`.
6. `OnNavigate` is already plumbed from `ExtensionsPanel` into `PluginManager`,
   so the jump button needs no new prop wiring.

## Design constraints worth keeping

- **The main app must not depend on the calibration crates.** Fitting maths
  (rustfft) lives in the wasm plugin; `phonon-tauri` deliberately has no
  `calibration-types` / `calibration-engine` dependency. Do not re-add them.
- **Uninstalling the plugin must not leave hidden DSP effects.** The panel
  detects the plugin going away and bypasses the native PEQ + FIR; the startup
  restore is gated on the plugin being present. Preserve both.
- **The plugin is referenced by name, not path.** The wasm polls its config
  every block, so the file must stay named `calibration.wasm`.

## Rebuilding the plugin binary

The sources are not on `main`; they live on the `校准插件` branch under
`workspace/plugins/calibration/` and `workspace/crates/calibration-{engine,types}/`.

```bash
git switch 校准插件          # brings the plugin sources and workspace members back
cargo build -p calibration --profile wasm-release --target wasm32-wasip1
cp target/wasm32-wasip1/wasm-release/calibration.wasm phonon-tauri/plugins/
```

The deployed binary must be rebuilt from those sources — it carries the
`get_target` command the panel uses to draw the target curve.
