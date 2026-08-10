fn main() {
    // The nip55 plugin is app-internal (in-crate Rust + in-tree Kotlin), so
    // its permission set is generated here instead of shipping from a plugin
    // crate. capabilities/mobile.json grants `nip55:default` to the WebView.
    tauri_build::try_build(
        tauri_build::Attributes::new().plugin(
            "nip55",
            tauri_build::InlinedPlugin::new()
                .commands(&[
                    "get_installed_signer_apps",
                    "get_public_key",
                    "sign_event",
                    "nip04_encrypt",
                    "nip04_decrypt",
                    "nip44_encrypt",
                    "nip44_decrypt",
                ])
                .default_permission(tauri_build::DefaultPermissionRule::AllowAllCommands),
        ),
    )
    .expect("failed to run tauri-build");
}
