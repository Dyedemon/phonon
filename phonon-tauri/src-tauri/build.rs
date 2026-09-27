fn main() {
    // Stamp the git commit into the binary so an installed app can be
    // identified against the source it was built from (About tab).
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=PHONON_BUILD_SHA={sha}");
    tauri_build::build()
}
