mod commands;
#[cfg(target_os = "macos")]
mod traffic_lights;

fn main() {
    // OpenSSH runs this executable as SSH_ASKPASS; answer before starting the window system.
    if std::env::var_os("SSH_MANAGER_ASKPASS").is_some() {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        let ok = ssh_manager_core::credentials::askpass(&prompt).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                let config = app
                    .config()
                    .app
                    .windows
                    .first()
                    .and_then(|w| w.traffic_light_position.clone());
                if let (Some(position), Some(window)) = (config, app.get_webview_window("main")) {
                    let ns_window = window.ns_window()? as *mut objc2_app_kit::NSWindow;
                    // setup runs on the main thread, where AppKit objects may be touched.
                    let ns_window = unsafe { objc2::rc::Retained::retain(ns_window) }
                        .expect("a created window has an NSWindow");
                    traffic_lights::pin(ns_window, position.x, position.y);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::overview,
            commands::history,
            commands::check,
            commands::trust_reset,
            commands::credential_set,
            commands::host_add,
            commands::host_update,
            commands::host_clear_issues,
            commands::host_remove,
            commands::import_preview,
            commands::import_apply,
            commands::settings_get,
            commands::settings_set,
            commands::reveal_data_dir,
            commands::open_terminal,
            commands::pick_key_file,
            commands::key_agents,
            commands::probe,
        ])
        .run(tauri::generate_context!())
        .expect("ssh-manager could not start its window");
}
