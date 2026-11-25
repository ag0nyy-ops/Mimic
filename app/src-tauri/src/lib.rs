mod commands;
mod tray;

use std::sync::Arc;

use mimic_core::manager::{Manager, ManagerConfig};
use tauri::{Emitter, Manager as _, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use mimic_core::app::AppState;

/// Named pipes on windows, a unix socket on macOS and Linux.
use mimic_core::ipc::PlatformConnector;

pub fn run() {
    let start_hidden = std::env::args().any(|a| a == "--minimized");

    let result = tauri::Builder::default()
        // must be first: a second launch just brings the running one forward
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("mimic_core", log::LevelFilter::Debug)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(move |app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;

            let (manager, presence) =
                Manager::new(PlatformConnector::default(), ManagerConfig::new(std::process::id()));
            tauri::async_runtime::spawn(manager.run());

            let state = Arc::new(AppState::load(dir, presence.clone()));
            for warning in &state.warnings {
                log::warn!("{warning}");
            }
            app.manage(state.clone());

            tray::init(app.handle())?;

            // push connection changes to the ui and the tray
            let handle = app.handle().clone();
            let mut rx = presence.subscribe();
            tauri::async_runtime::spawn(async move {
                while rx.changed().await.is_ok() {
                    let current = rx.borrow_and_update().clone();
                    log::debug!("connection state: {current:?}");
                    let _ = handle.emit("connection-state", current);
                    tray::refresh(&handle);
                }
            });

            // bring back whatever was live last time
            if state.restore_last().is_some() {
                tray::refresh(app.handle());
            }

            if !start_hidden {
                tray::show_main_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(state) = window.app_handle().try_state::<Arc<AppState>>() else {
                return;
            };
            match event {
                WindowEvent::CloseRequested { api, .. } if state.settings().close_to_tray => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                WindowEvent::Resized(_)
                    if state.settings().minimize_to_tray && window.is_minimized().unwrap_or(false) =>
                {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::search_games,
            commands::refresh_games,
            commands::create_preset,
            commands::save_preset,
            commands::duplicate_preset,
            commands::delete_preset,
            commands::go_live,
            commands::stop_live,
            commands::retry_connection,
            commands::save_settings,
            commands::set_autostart,
        ])
        .build(tauri::generate_context!());

    let app = match result {
        Ok(app) => app,
        Err(e) => {
            eprintln!("mimic failed to start: {e}");
            std::process::exit(1);
        }
    };

    app.run(|_app, _event| {
        // macOS: clicking the dock icon while the window is hidden (closed or
        // minimized to the tray) brings it back, like any other mac app
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { has_visible_windows: false, .. } = _event {
            tray::show_main_window(_app);
        }
    });
}
