// Crate root of the AI Code Session Widget (fork of AI Session Widget).
// Later tasks add tauri::command handlers and register them here.

mod autostart;
mod background;
mod com;
mod commands;
mod codex;
mod deeplink;
mod sessions;
mod pid;
mod status;
mod model;
mod focus;
mod usage;

use tauri::Manager;

/// Second start (desktop icon, Start menu, autostart while running): bring
/// the running widget forward, also when it is hidden in the tray.
fn show_running_widget(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // Fork: only one widget at a time. The new process hands over to the
    // running one and exits before it opens a window (named mutex + hidden
    // message window, all local). Release builds only, so `tauri dev` still
    // runs next to the installed widget. Must be the first plugin.
    if !cfg!(debug_assertions) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_running_widget(app)
        }));
    }

    builder
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::focus_session,
            commands::kill_orphan_process,
            commands::set_always_on_top,
            commands::close_app,
            commands::open_desktop_session,
            commands::get_autostart,
            commands::set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
