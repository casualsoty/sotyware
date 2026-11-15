fn main() {
    request_recorder_guard();
    test_profile_guard();
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
            "plan_cloud_operation",
            "discard_cloud_operation_plan",
            "start_cloud_operation",
            "cancel_cloud_operation",
            "retry_cloud_operation",
            "create_cloud_folder",
            "cloud_object_properties",
            "preview_cloud_file",
            "search_cloud_folder",
            "list_cloud_deleted",
            "share_cloud_file",
            "upload_cloud_folder",
            "download_cloud_items",
            "cancel_cloud_items_download",
            "retry_cloud_items_download",
            "list_cloud_places",
            "set_cloud_favorite",
            "record_cloud_recent",
            "clear_cloud_recent",
            "watch_cloud_drops",
            "unwatch_cloud_drops",
            "upload_cloud_drop",
            "list_cloud_recovery",
            "inspect_cloud_recovery",
            "dismiss_cloud_recovery",
            "list_cloud_incomplete_uploads",
            "abort_cloud_incomplete_upload",
            "list_backups",
            "save_backup_job",
            "remove_backup_job",
            "start_backup_task",
            "cancel_backup_task",
            "list_backup_snapshots",
            "restore_backup",
            "export_backup_manifest",
            "check_backup_protection",
            "account_status",
            "account_register",
            "account_verify",
            "account_sign_in",
            "account_sign_out",
            "account_set_up_sync",
            "account_check_recovery_key",
            "account_unlock",
            "account_request_approval",
            "account_approval_status",
            "account_enrollments",
            "account_answer_enrollment",
            "account_devices",
            "account_revoke_device",
            "account_rotate_keys",
            "account_sync",
            "account_resolve_conflict",
            "account_change_password",
            "account_reset_password",
            "account_delete",
            "update_status",
            "check_for_update",
            "install_update",
            "cancel_update",
            "postpone_update",
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

/// A test-profile build (ADR-027) must not replace `target/release/sotyware`
/// either: its own target directory, named `test-profile` (or `recorder`,
/// which implies the feature).
fn test_profile_guard() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_PROFILE");
    if std::env::var_os("CARGO_FEATURE_TEST_PROFILE").is_none() {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let separate = out.ancestors().any(|p| {
        p.file_name()
            .is_some_and(|n| n == "test-profile" || n == "recorder")
    });
    if !separate {
        eprintln!(
            "error: the test-profile feature needs its own target directory: \
             build with --target-dir target/test-profile (ADR-027)"
        );
        std::process::exit(1);
    }
}
