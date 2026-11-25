//! The tray icon: status line, quick switch between presets, clear, quit.

use std::sync::Arc;

use mimic_core::manager::ConnectionState;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use mimic_core::app::{AppState, LiveDto};

const TRAY_ID: &str = "main";

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn status_text(state: &AppState) -> String {
    let live_name = state
        .live()
        .and_then(|l| state.preset(&l.preset_id))
        .map(|p| p.game.name);
    match (state.presence.state(), live_name) {
        (ConnectionState::Idle, _) => "Not live".into(),
        (ConnectionState::Connecting, _) => "Connecting to Discord…".into(),
        (ConnectionState::Connected { .. }, Some(game)) => format!("Live: {game}"),
        (ConnectionState::Connected { .. }, None) => "Connected".into(),
        (ConnectionState::DiscordNotRunning { .. }, _) => "Discord isn't running, retrying".into(),
        (ConnectionState::BadAppId { .. }, _) => "Discord rejected this game's app id".into(),
        (ConnectionState::RateLimited { .. }, Some(game)) => format!("Live: {game} (update queued)"),
        (ConnectionState::RateLimited { .. }, None) => "Rate limited, waiting".into(),
        (ConnectionState::Error { message }, _) => format!("Error: {message}"),
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<Arc<AppState>>();
    let live_id = state.live().map(|l| l.preset_id);

    let status = MenuItem::with_id(app, "status", status_text(&state), false, None::<&str>)?;

    let presets = state.presets();
    let preset_items: Vec<CheckMenuItem<Wry>> = presets
        .iter()
        .map(|p| {
            CheckMenuItem::with_id(
                app,
                format!("preset:{}", p.id),
                format!("{}  ·  {}", p.name, p.game.name),
                true,
                live_id.as_deref() == Some(p.id.as_str()),
                None::<&str>,
            )
        })
        .collect::<tauri::Result<_>>()?;
    let none_yet = MenuItem::with_id(app, "no-presets", "No presets yet", false, None::<&str>)?;
    let mut preset_refs: Vec<&dyn IsMenuItem<Wry>> = Vec::new();
    for item in &preset_items {
        preset_refs.push(item);
    }
    if preset_refs.is_empty() {
        preset_refs.push(&none_yet);
    }
    let switch = Submenu::with_id_and_items(app, "switch", "Quick switch", true, &preset_refs)?;

    let clear = MenuItem::with_id(app, "stop", "Clear presence", live_id.is_some(), None::<&str>)?;
    let open = MenuItem::with_id(app, "show", "Open mimic", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Launch at login",
        true,
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit mimic", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    Menu::with_items(
        app,
        &[&status, &sep1, &switch, &clear, &sep2, &open, &autostart, &sep3, &quit],
    )
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("mimic")
        .menu(&menu)
        // windows: left click opens the window, right click the menu.
        // macOS menu bar items open their menu on a left click. linux tray
        // icons only ever show the menu, and "Open mimic" is in it.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if !cfg!(target_os = "macos") {
                    show_main_window(tray.app_handle());
                }
            }
        })
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()));
    builder = with_icon(app, builder)?;
    builder.build(app)?;
    Ok(())
}

/// macOS: a black on transparent template image, which the menu bar tints to
/// match light and dark mode like the system icons next to it.
#[cfg(target_os = "macos")]
fn with_icon(
    _app: &AppHandle,
    builder: TrayIconBuilder<Wry>,
) -> tauri::Result<TrayIconBuilder<Wry>> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template@2x.png"))?;
    Ok(builder.icon(icon).icon_as_template(true))
}

/// windows and linux: the app icon.
#[cfg(not(target_os = "macos"))]
fn with_icon(
    app: &AppHandle,
    builder: TrayIconBuilder<Wry>,
) -> tauri::Result<TrayIconBuilder<Wry>> {
    Ok(match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone()),
        None => builder,
    })
}

/// Rebuild the menu and tooltip after presets, the live preset or the
/// connection changed.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::warn!("couldn't rebuild the tray menu: {e}"),
    }
    let state = app.state::<Arc<AppState>>();
    let _ = tray.set_tooltip(Some(format!("mimic - {}", status_text(&state))));
}

fn on_menu_event(app: &AppHandle, id: &str) {
    let state = app.state::<Arc<AppState>>();
    match id {
        "show" => show_main_window(app),
        "quit" => app.exit(0),
        "stop" => {
            state.stop_live();
            let _ = app.emit("live-changed", Option::<LiveDto>::None);
            refresh(app);
        }
        "autostart" => {
            let launch = app.autolaunch();
            let enabled = launch.is_enabled().unwrap_or(false);
            let result = if enabled { launch.disable() } else { launch.enable() };
            if let Err(e) = result {
                log::warn!("couldn't toggle launch at login: {e}");
            }
            refresh(app);
        }
        other => {
            if let Some(preset_id) = other.strip_prefix("preset:") {
                match state.go_live(preset_id) {
                    Ok(live) => {
                        let _ = app.emit("live-changed", Some(live));
                    }
                    Err(e) => log::warn!("tray switch failed: {}", e.message),
                }
                refresh(app);
            }
        }
    }
}
