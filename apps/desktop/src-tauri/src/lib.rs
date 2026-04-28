#![forbid(unsafe_code)]

mod commands;
mod onboarding;
mod orgs;
mod proposals;
mod settings;
mod state;
mod teams;
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
            commands::settings_status,
            commands::settings_set_api_key,
            commands::onboarding_chat,
            commands::onboarding_finalize,
            commands::onboarding_save,
            commands::token_list,
            commands::token_issue,
            commands::token_revoke,
            commands::audit_list,
            proposals::proposal_list,
            proposals::proposal_approve,
            proposals::proposal_reject,
            orgs::auth_me,
            orgs::auth_logout,
            orgs::auth_account_update,
            orgs::auth_password_change,
            orgs::orgs_list,
            orgs::org_create,
            orgs::org_get,
            orgs::org_update,
            orgs::org_members,
            orgs::org_member_update,
            orgs::org_member_remove,
            orgs::org_pending,
            orgs::org_pending_approve,
            orgs::org_pending_reject,
            orgs::org_invitations,
            orgs::org_invite,
            orgs::org_invitation_delete,
            orgs::org_logo_get,
            orgs::org_logo_upload,
            orgs::org_logo_delete,
            teams::teams_list,
            teams::team_create,
            teams::team_get,
            teams::team_update,
            teams::team_delete,
            teams::team_members,
            teams::team_member_add,
            teams::team_member_update,
            teams::team_member_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running orchext-desktop");
}
