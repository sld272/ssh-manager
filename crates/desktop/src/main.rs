mod commands;

fn main() {
    // OpenSSH runs this executable as SSH_ASKPASS; answer before starting the window system.
    if std::env::var_os("SSH_MANAGER_ASKPASS").is_some() {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        let ok = ssh_manager_core::credentials::askpass(&prompt).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::overview,
            commands::history,
            commands::check,
            commands::trust_scan,
            commands::trust_accept,
            commands::trust_import,
            commands::credential_set,
            commands::host_add,
            commands::host_update,
            commands::host_remove,
            commands::project_save,
            commands::project_remove,
            commands::import_preview,
            commands::import_apply,
            commands::agents,
        ])
        .run(tauri::generate_context!())
        .expect("ssh-manager could not start its window");
}
