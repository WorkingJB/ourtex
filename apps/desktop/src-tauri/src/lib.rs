#![forbid(unsafe_code)]

mod commands;
mod onboarding;
mod settings;
mod state;
mod watch;
mod workspaces;

pub fn run() {
    // Load the workspace registry before Tauri starts so first-run
    // detection (empty registry → show picker) is a cheap synchronous
    // check in the frontend.
    let app_state = tauri::async_runtime::block_on(async {
        state::default_state()
            .await
            .expect("failed to load workspace registry")
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::workspace_list,
            commands::workspace_add,
            commands::workspace_connect_remote,
            commands::workspace_unlock,
            commands::workspace_lock,
            commands::workspace_crypto_state,
            commands::workspace_activate,
            commands::workspace_remove,
            commands::workspace_rename,
            commands::vault_info,
            commands::doc_list,
            commands::doc_read,
            commands::doc_write,
            commands::doc_delete,
            commands::graph_snapshot,
            commands::settings_status,
            commands::settings_set_api_key,
            commands::onboarding_chat,
            commands::onboarding_finalize,
            commands::onboarding_save,
            commands::token_list,
            commands::token_issue,
            commands::token_revoke,
            commands::audit_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mytex-desktop");
}
