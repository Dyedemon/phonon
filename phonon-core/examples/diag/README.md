# WASAPI diagnostics (Windows only, not auto-built)

These probe exclusive-mode WASAPI behavior (format negotiation, event vs
polling consumption) against the real device. They are excluded from the
automatic example build because they import windows-only APIs
unconditionally (CI builds example targets on Linux/macOS too).

To run on Windows, temporarily copy the file up to the examples root:

```
cp examples/diag/wasapi_probe.rs examples/wasapi_probe.rs
cargo run --example wasapi_probe -p phonon-core
rm examples/wasapi_probe.rs
```

`wasapi_prod_verify.rs` exercises the production `WasapiExclusiveStream`
(polling pump) end to end with a 440 Hz sine — use it to confirm a device
consumes audio before debugging the app itself.
