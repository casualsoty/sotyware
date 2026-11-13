fn main() {
    request_recorder_guard();
    // Declaring the app's commands makes each one need a permission
    // (`allow-<command>`) in a capability, instead of being open to every window.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "operating_mode",
            "list_features",
            "set_feature_enabled",
            "list_cloud_providers",
            "list_cloud_accounts",
            "add_cloud_account",
            "update_cloud_account",
            "remove_cloud_account",
            "check_cloud_account",
            "list_cloud_folder",
            "download_cloud_file",
            "cancel_cloud_download",
            "retry_cloud_download",
            "upload_cloud_files",
            "cancel_cloud_upload",
            "retry_cloud_upload",
        ]),
    ))
    .expect("failed to run tauri-build");
}

/// The request recorder (ADR-024) is for controlled integration runs only:
/// a build with the feature must go to its own target directory, named
/// `recorder`, so it never replaces `target/release/sotyware` (which
/// `tauri build` packages).
fn request_recorder_guard() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_REQUEST_RECORDER");
    if std::env::var_os("CARGO_FEATURE_REQUEST_RECORDER").is_none() {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let separate = out
        .ancestors()
        .any(|p| p.file_name().is_some_and(|n| n == "recorder"));
    if !separate {
        eprintln!(
            "error: the request-recorder feature needs its own target directory: \
             build with --target-dir target/recorder (ADR-024)"
        );
        std::process::exit(1);
    }
}
