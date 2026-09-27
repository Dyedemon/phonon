fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_bundle)");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../phonon-tauri/src-tauri/icons/icon.ico");

    // App bundle embedding: copy the zip to OUT_DIR/app_bundle.bin so
    // include_bytes!(concat!(env!("OUT_DIR"), "/app_bundle.bin")) can find it.
    let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| "target/out".to_string());
    let bundle_dst = std::path::Path::new(&out_dir).join("app_bundle.bin");
    let _ = std::fs::remove_file(&bundle_dst);

    if let Ok(zip_path) = std::env::var("PHONON_APP_ZIP") {
        // Watch the FILE, not just the env value: the build script always
        // receives the same temp path, so rerun-if-env-changed alone never
        // fired after the first build — cargo skipped re-embedding and the
        // installer kept carrying the first zip it ever saw (stale app).
        println!("cargo:rerun-if-changed={zip_path}");
        if std::path::Path::new(&zip_path).exists() {
            match std::fs::copy(&zip_path, &bundle_dst) {
                Ok(_) => {
                    println!("cargo:rustc-cfg=embedded_bundle");
                    println!("cargo:warning=App bundle embedded from: {}", zip_path);
                }
                Err(e) => {
                    println!("cargo:warning=Failed to copy app bundle: {}", e);
                }
            }
        } else {
            println!("cargo:warning=PHONON_APP_ZIP file not found: {}", zip_path);
        }
    } else {
        println!("cargo:warning=PHONON_APP_ZIP not set; building without app bundle");
    }

    println!("cargo:rerun-if-env-changed=PHONON_APP_ZIP");
    println!("cargo:rerun-if-changed=../phonon-tauri/src-tauri/icons/icon.ico");

    // Require Administrator privilege for per-machine installs.
    res.set_manifest(
        r#"
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>
"#
    );
    res.compile().expect("Failed to compile Windows resources");
}
