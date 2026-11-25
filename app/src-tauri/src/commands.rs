//! The functions the frontend can call.

use std::sync::Arc;
use std::time::Duration;

use mimic_core::games::Game;
use mimic_core::manager::ConnectionState;
use mimic_core::preset::{GameRef, Preset};
use mimic_core::settings::Settings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;

use mimic_core::app::{AppState, CmdError, GamesInfo, LiveDto, SaveResult, DETECTABLE_URL};
use crate::tray;

type S<'a> = State<'a, Arc<AppState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    version: &'static str,
    presets: Vec<Preset>,
    last_preset_id: Option<String>,
    live: Option<LiveDto>,
    connection: ConnectionState,
    settings: Settings,
    autostart: bool,
    games: GamesInfo,
    warnings: Vec<String>,
    data_dir: String,
}

#[tauri::command]
pub fn bootstrap(app: AppHandle, state: S) -> Bootstrap {
    Bootstrap {
        version: env!("CARGO_PKG_VERSION"),
        presets: state.presets(),
        last_preset_id: state.last_preset_id(),
        live: state.live(),
        connection: state.presence.state(),
        settings: state.settings(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        games: state.games_info(),
        warnings: state.warnings.clone(),
        data_dir: state.paths.dir.display().to_string(),
    }
}

#[tauri::command]
pub fn search_games(state: S, query: String, limit: Option<usize>) -> Vec<Game> {
    state.search_games(&query, limit.unwrap_or(50))
}

#[tauri::command]
pub async fn refresh_games(state: S<'_>) -> Result<GamesInfo, CmdError> {
    let client = reqwest::Client::builder()
        .user_agent("mimic")
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| CmdError::msg(e.to_string()))?;
    let response = client
        .get(DETECTABLE_URL)
        .send()
        .await
        .map_err(|e| CmdError::msg(format!("couldn't reach discord: {e}")))?;
    if !response.status().is_success() {
        return Err(CmdError::msg(format!(
            "discord answered {}, keeping the current list",
            response.status()
        )));
    }
    let raw = response
        .text()
        .await
        .map_err(|e| CmdError::msg(format!("the download was cut off: {e}")))?;
    state.install_downloaded_games(&raw)
}

#[tauri::command]
pub fn create_preset(app: AppHandle, state: S, game: GameRef) -> Result<Preset, CmdError> {
    if !state.game_exists(&game.id) {
        return Err(CmdError::msg("that game isn't in the list"));
    }
    let preset = state.create_preset(game)?;
    tray::refresh(&app);
    Ok(preset)
}

#[tauri::command]
pub fn save_preset(app: AppHandle, state: S, preset: Preset) -> Result<SaveResult, CmdError> {
    let result = state.save_preset(preset)?;
    tray::refresh(&app);
    Ok(result)
}

#[tauri::command]
pub fn duplicate_preset(app: AppHandle, state: S, id: String) -> Result<Preset, CmdError> {
    let preset = state.duplicate_preset(&id)?;
    tray::refresh(&app);
    Ok(preset)
}

#[tauri::command]
pub fn delete_preset(app: AppHandle, state: S, id: String) {
    let was_live = state.live().is_some_and(|l| l.preset_id == id);
    state.delete_preset(&id);
    if was_live {
        let _ = app.emit("live-changed", Option::<LiveDto>::None);
    }
    tray::refresh(&app);
}

#[tauri::command]
pub fn go_live(app: AppHandle, state: S, id: String) -> Result<LiveDto, CmdError> {
    let live = state.go_live(&id)?;
    let _ = app.emit("live-changed", Some(live.clone()));
    tray::refresh(&app);
    Ok(live)
}

#[tauri::command]
pub fn stop_live(app: AppHandle, state: S) {
    state.stop_live();
    let _ = app.emit("live-changed", Option::<LiveDto>::None);
    tray::refresh(&app);
}

#[tauri::command]
pub fn retry_connection(state: S) {
    state.presence.retry_now();
}

#[tauri::command]
pub fn save_settings(state: S, settings: Settings) {
    state.set_settings(settings);
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, CmdError> {
    let launch = app.autolaunch();
    let result = if enabled { launch.enable() } else { launch.disable() };
    result.map_err(|e| CmdError::msg(format!("couldn't change launch at login: {e}")))?;
    tray::refresh(&app);
    Ok(launch.is_enabled().unwrap_or(enabled))
}
