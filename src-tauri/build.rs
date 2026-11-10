fn main() {
    // Declaring the app's commands makes each one need a permission
    // (`allow-<command>`) in a capability, instead of being open to every window.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["operating_mode"])),
    )
    .expect("failed to run tauri-build");
}
