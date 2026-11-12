fn main() {
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
        ]),
    ))
    .expect("failed to run tauri-build");
}
