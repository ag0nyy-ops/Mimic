//! Everything the app keeps in memory, and the operations on it that both the
//! ui commands and the tray menu need. This has no ui framework in it, so it's
//! tested here with the rest of the core.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::activity::ActivityError;
use crate::games::{self, Game, GameList};
use crate::manager::PresenceHandle;
use crate::message::ProtocolError;
use crate::preset::{GameRef, Live, Preset, PresetStore};
use crate::settings::Settings;
use crate::storage;
use serde::Serialize;

/// The game list that ships in the binary (`shared/games.json`).
const BUNDLED_GAMES: &str = include_str!("../../../shared/games.json");

pub const DETECTABLE_URL: &str = "https://discord.com/api/v9/applications/detectable";

/// What a command sends back when it fails. `field` lets the ui put the
/// message next to the right input.
#[derive(Debug, Clone, Serialize)]
pub struct CmdError {
    pub field: Option<String>,
    pub message: String,
}

impl CmdError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            field: None,
            message: message.into(),
        }
    }
}

impl From<ActivityError> for CmdError {
    fn from(e: ActivityError) -> Self {
        Self {
            field: Some(e.field()),
            message: e.to_string(),
        }
    }
}

impl From<ProtocolError> for CmdError {
    fn from(e: ProtocolError) -> Self {
        Self::msg(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveDto {
    pub preset_id: String,
    pub applied_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GamesSource {
    Bundled,
    Downloaded,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamesInfo {
    pub count: usize,
    pub source: GamesSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub preset: Preset,
    /// Set when the preset is saved fine but can't be sent to discord as is.
    pub error: Option<CmdError>,
    /// Present when this preset is the live one (its clock may have restarted).
    pub live: Option<LiveDto>,
}

pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn presets(&self) -> PathBuf {
        self.dir.join("presets.json")
    }
    pub fn settings(&self) -> PathBuf {
        self.dir.join("settings.json")
    }
    pub fn games_cache(&self) -> PathBuf {
        self.dir.join("games.json")
    }
}

pub struct AppState {
    pub presence: PresenceHandle,
    pub paths: Paths,
    pub warnings: Vec<String>,
    games: RwLock<(GameList, GamesSource)>,
    store: Mutex<PresetStore>,
    live: Mutex<Option<Live>>,
    settings: RwLock<Settings>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // a panic while holding one of these locks shouldn't brick the whole app
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl AppState {
    pub fn load(dir: PathBuf, presence: PresenceHandle) -> Self {
        let paths = Paths { dir };
        let mut warnings = Vec::new();

        let games = match std::fs::read_to_string(paths.games_cache())
            .ok()
            .and_then(|raw| GameList::from_json(&raw).ok())
        {
            Some(list) => (list, GamesSource::Downloaded),
            None => (
                GameList::from_json(BUNDLED_GAMES).expect("bundled games.json is valid"),
                GamesSource::Bundled,
            ),
        };

        let store = PresetStore::load(&paths.presets());
        warnings.extend(store.warning);
        let settings = storage::load_json::<Settings>(&paths.settings());
        warnings.extend(settings.warning);

        Self {
            presence,
            paths,
            warnings,
            games: RwLock::new(games),
            store: Mutex::new(store.value),
            live: Mutex::new(None),
            settings: RwLock::new(settings.value),
        }
    }

    // ---- games

    pub fn search_games(&self, query: &str, limit: usize) -> Vec<Game> {
        let guard = self.games.read().unwrap_or_else(|e| e.into_inner());
        guard.0.search(query, limit.clamp(1, 200))
    }

    pub fn games_info(&self) -> GamesInfo {
        let guard = self.games.read().unwrap_or_else(|e| e.into_inner());
        GamesInfo {
            count: guard.0.len(),
            source: guard.1,
        }
    }

    /// Parse a downloaded response, and if it's sane keep it in memory and on disk.
    pub fn install_downloaded_games(&self, raw: &str) -> Result<GamesInfo, CmdError> {
        let parsed = games::parse_games(raw).map_err(|e| CmdError::msg(e.to_string()))?;
        if let Err(e) = storage::save_json(&self.paths.games_cache(), &parsed) {
            log::warn!("couldn't cache the game list: {e}");
        }
        *self.games.write().unwrap_or_else(|e| e.into_inner()) =
            (GameList::new(parsed), GamesSource::Downloaded);
        Ok(self.games_info())
    }

    pub fn game_exists(&self, id: &str) -> bool {
        self.games
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .get(id)
            .is_some()
    }

    // ---- settings

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_settings(&self, new: Settings) {
        if let Err(e) = storage::save_json(&self.paths.settings(), &new) {
            log::error!("couldn't save settings: {e}");
        }
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = new;
    }

    // ---- presets

    pub fn presets(&self) -> Vec<Preset> {
        lock(&self.store).presets.clone()
    }

    pub fn last_preset_id(&self) -> Option<String> {
        lock(&self.store).last_preset_id.clone()
    }

    pub fn preset(&self, id: &str) -> Option<Preset> {
        lock(&self.store).get(id).cloned()
    }

    fn persist(&self, store: &PresetStore) {
        if let Err(e) = store.save(&self.paths.presets()) {
            log::error!("couldn't save presets: {e}");
        }
    }

    pub fn create_preset(&self, game: GameRef) -> Result<Preset, CmdError> {
        crate::message::validate_client_id(&game.id)?;
        let preset = Preset::new(game);
        let mut store = lock(&self.store);
        store.upsert(preset.clone());
        self.persist(&store);
        Ok(preset)
    }

    pub fn duplicate_preset(&self, id: &str) -> Result<Preset, CmdError> {
        let mut store = lock(&self.store);
        let copy = store
            .duplicate(id)
            .ok_or_else(|| CmdError::msg("that preset doesn't exist anymore"))?;
        self.persist(&store);
        Ok(copy)
    }

    pub fn delete_preset(&self, id: &str) {
        let was_live = lock(&self.live).as_ref().is_some_and(|l| l.preset_id == id);
        if was_live {
            self.stop_live();
        }
        let mut store = lock(&self.store);
        store.remove(id);
        self.persist(&store);
    }

    /// Save an edited preset. If it's the one that's live, also push the change
    /// to discord (the connection manager debounces and rate limits that).
    pub fn save_preset(&self, mut preset: Preset) -> Result<SaveResult, CmdError> {
        crate::message::validate_client_id(&preset.game.id)?;
        if preset.name.trim().is_empty() {
            preset.name = preset.game.name.clone();
        }
        {
            let mut store = lock(&self.store);
            store.upsert(preset.clone());
            self.persist(&store);
        }

        let mut error = None;
        let mut live_dto = None;
        let mut live = lock(&self.live);
        match live.as_mut().filter(|l| l.preset_id == preset.id) {
            Some(live) => {
                live.refresh(&preset, unix_now());
                live_dto = Some(LiveDto {
                    preset_id: live.preset_id.clone(),
                    applied_at: live.applied_at,
                });
                match live.build(&preset) {
                    Ok(activity) => self.presence.set(&preset.game.id, Some(activity))?,
                    // keep whatever discord is showing; tell the ui why it isn't updating
                    Err(e) => error = Some(CmdError::from(e)),
                }
            }
            // not live: nothing to send, but the editor still wants to know
            // about invalid fields right away
            None => error = preset.build(unix_now()).err().map(CmdError::from),
        }
        Ok(SaveResult {
            preset,
            error,
            live: live_dto,
        })
    }

    // ---- going live

    pub fn live(&self) -> Option<LiveDto> {
        lock(&self.live).as_ref().map(|l| LiveDto {
            preset_id: l.preset_id.clone(),
            applied_at: l.applied_at,
        })
    }

    pub fn go_live(&self, id: &str) -> Result<LiveDto, CmdError> {
        let preset = self
            .preset(id)
            .ok_or_else(|| CmdError::msg("that preset doesn't exist anymore"))?;
        let live = Live::start(&preset, unix_now());
        let activity = live.build(&preset)?;
        self.presence.set(&preset.game.id, Some(activity))?;
        let dto = LiveDto {
            preset_id: live.preset_id.clone(),
            applied_at: live.applied_at,
        };
        *lock(&self.live) = Some(live);

        let mut store = lock(&self.store);
        store.last_preset_id = Some(id.to_owned());
        self.persist(&store);
        Ok(dto)
    }

    /// Stop showing a presence. This closes the connection instead of sending a
    /// "clear" command: discord removes a presence the moment its socket closes,
    /// whereas a clear would be queued behind the update rate limit.
    pub fn stop_live(&self) {
        self.presence.disconnect();
        *lock(&self.live) = None;
        let mut store = lock(&self.store);
        store.last_preset_id = None;
        self.persist(&store);
    }

    /// Put the last live preset back on launch. Returns it if there was one.
    pub fn restore_last(&self) -> Option<LiveDto> {
        let id = self.last_preset_id()?;
        match self.go_live(&id) {
            Ok(dto) => Some(dto),
            Err(e) => {
                log::warn!("couldn't restore the last preset: {}", e.message);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{ButtonInput, Timer};
    use crate::manager::{Connector, Manager, ManagerConfig};
    use std::io;
    use tokio::io::DuplexStream;

    /// The tests only exercise state handling, so no pipe ever exists.
    struct NoPipes;
    impl Connector for NoPipes {
        type Stream = DuplexStream;
        async fn connect(&self, _index: u8) -> io::Result<DuplexStream> {
            Err(io::ErrorKind::NotFound.into())
        }
    }

    fn state_in(dir: &std::path::Path) -> AppState {
        // commands to a manager nobody runs are simply dropped, which is fine here
        let (_manager, handle) = Manager::new(NoPipes, ManagerConfig::new(1));
        AppState::load(dir.to_path_buf(), handle)
    }

    fn minecraft(state: &AppState) -> GameRef {
        let game = state.search_games("minecraft", 5).into_iter().find(|g| g.name == "Minecraft").unwrap();
        GameRef { id: game.id, name: game.name }
    }

    #[test]
    fn the_bundled_list_is_loaded_and_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let info = state.games_info();
        assert_eq!(info.source, GamesSource::Bundled);
        assert!(info.count > 1000);
        assert!(state.game_exists(&minecraft(&state).id));
        assert!(!state.game_exists("123456789012345678"));
    }

    #[test]
    fn create_validates_the_id_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let bad = GameRef { id: "nope".into(), name: "x".into() };
        assert!(state.create_preset(bad).is_err());

        let preset = state.create_preset(minecraft(&state)).unwrap();
        assert_eq!(preset.name, "Minecraft");

        // a fresh process sees it
        let again = state_in(dir.path());
        assert_eq!(again.presets().len(), 1);
        assert_eq!(again.presets()[0].id, preset.id);
    }

    #[test]
    fn saving_an_invalid_preset_keeps_it_but_reports_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let mut preset = state.create_preset(minecraft(&state)).unwrap();
        preset.buttons = vec![ButtonInput { label: "Go".into(), url: "nope".into() }];

        let result = state.save_preset(preset.clone()).unwrap();
        let err = result.error.expect("invalid url is reported");
        assert_eq!(err.field.as_deref(), Some("buttons.0.url"));
        assert!(result.live.is_none());
        // the user's work wasn't thrown away
        assert_eq!(state_in(dir.path()).presets()[0].buttons[0].url, "nope");
    }

    #[test]
    fn blank_names_fall_back_to_the_game_name() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let mut preset = state.create_preset(minecraft(&state)).unwrap();
        preset.name = "   ".into();
        assert_eq!(state.save_preset(preset).unwrap().preset.name, "Minecraft");
    }

    #[test]
    fn going_live_is_remembered_and_restored_on_the_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let preset = state.create_preset(minecraft(&state)).unwrap();
        assert!(state.live().is_none());

        let live = state.go_live(&preset.id).unwrap();
        assert_eq!(live.preset_id, preset.id);
        assert_eq!(state.last_preset_id().as_deref(), Some(preset.id.as_str()));

        let relaunched = state_in(dir.path());
        assert!(relaunched.live().is_none(), "nothing is live until restore runs");
        let restored = relaunched.restore_last().expect("last preset comes back");
        assert_eq!(restored.preset_id, preset.id);
        assert!(relaunched.live().is_some());
    }

    #[test]
    fn stopping_forgets_the_last_preset() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let preset = state.create_preset(minecraft(&state)).unwrap();
        state.go_live(&preset.id).unwrap();
        state.stop_live();
        assert!(state.live().is_none());
        assert!(state.last_preset_id().is_none());
        assert!(state_in(dir.path()).restore_last().is_none(), "a stopped presence doesn't come back");
        assert_eq!(state_in(dir.path()).presets().len(), 1, "the preset itself is kept");
    }

    #[test]
    fn going_live_with_an_invalid_preset_fails_with_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let mut preset = state.create_preset(minecraft(&state)).unwrap();
        preset.buttons = vec![ButtonInput { label: "".into(), url: "https://example.com".into() }];
        state.save_preset(preset.clone()).unwrap();
        let err = state.go_live(&preset.id).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("buttons.0.label"));
        assert!(state.live().is_none());
        assert!(state.last_preset_id().is_none());
    }

    #[test]
    fn editing_the_live_preset_keeps_its_clock() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let mut preset = state.create_preset(minecraft(&state)).unwrap();
        preset.timer = Timer::Now;
        state.save_preset(preset.clone()).unwrap();
        let live = state.go_live(&preset.id).unwrap();

        preset.details = "typed after going live".into();
        let result = state.save_preset(preset).unwrap();
        assert!(result.error.is_none());
        assert_eq!(result.live.unwrap().applied_at, live.applied_at);
    }

    #[test]
    fn editing_a_preset_that_is_not_live_reports_no_live_info() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let a = state.create_preset(minecraft(&state)).unwrap();
        let b = state.duplicate_preset(&a.id).unwrap();
        state.go_live(&a.id).unwrap();
        let result = state.save_preset(b).unwrap();
        assert!(result.live.is_none());
        assert_eq!(state.live().unwrap().preset_id, a.id);
    }

    #[test]
    fn deleting_the_live_preset_stops_the_presence() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let preset = state.create_preset(minecraft(&state)).unwrap();
        state.go_live(&preset.id).unwrap();
        state.delete_preset(&preset.id);
        assert!(state.live().is_none());
        assert!(state.presets().is_empty());
        assert!(state.last_preset_id().is_none());
    }

    #[test]
    fn a_missing_preset_is_a_clean_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        assert!(state.go_live("nope").is_err());
        assert!(state.duplicate_preset("nope").is_err());
    }

    fn big_game_list(extra_name: &str) -> String {
        let mut items: Vec<String> = (0..150u64)
            .map(|i| format!(r#"{{"id":"{}","name":"Filler {i}"}}"#, 700_000_000_000_000_000u64 + i))
            .collect();
        items.push(format!(r#"{{"id":"800000000000000001","name":"{extra_name}"}}"#));
        format!("[{}]", items.join(","))
    }

    #[test]
    fn refreshing_the_game_list_swaps_it_in_and_caches_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let info = state.install_downloaded_games(&big_game_list("Brand New Game")).unwrap();
        assert_eq!(info.source, GamesSource::Downloaded);
        assert_eq!(info.count, 151);
        assert_eq!(state.search_games("brand new", 5)[0].name, "Brand New Game");

        // the cache is picked up on the next launch
        let again = state_in(dir.path());
        assert_eq!(again.games_info().source, GamesSource::Downloaded);
        assert_eq!(again.games_info().count, 151);
    }

    #[test]
    fn a_bad_download_leaves_the_current_list_alone() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let before = state.games_info().count;
        assert!(state.install_downloaded_games("<html>429 too many requests</html>").is_err());
        assert!(state.install_downloaded_games(r#"[{"id":"123456789012345678","name":"Only one"}]"#).is_err());
        assert_eq!(state.games_info().count, before);
        assert_eq!(state.games_info().source, GamesSource::Bundled);
        assert!(!dir.path().join("games.json").exists());
    }

    #[test]
    fn settings_persist() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        assert!(state.settings().close_to_tray);
        state.set_settings(Settings { close_to_tray: false, minimize_to_tray: true });
        assert!(!state_in(dir.path()).settings().close_to_tray);
    }

    #[test]
    fn unreadable_files_produce_a_warning_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("presets.json"), "{{{").unwrap();
        let state = state_in(dir.path());
        assert!(state.presets().is_empty());
        assert_eq!(state.warnings.len(), 1);
    }
}
