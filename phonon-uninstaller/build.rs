fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../phonon-tauri/src-tauri/icons/icon.ico");
    res.compile().expect("Failed to embed icon resource");
}
