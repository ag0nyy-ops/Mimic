//! Saved presets and the preset store (`presets.json`), plus the little bit of
//! state that remembers which preset is currently live.
//!
//! The json layout is the one in `shared/preset.schema.json`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::activity::{
    build_activity, Activity, ActivityError, ActivityInput, ButtonInput, PartyInput, Timer,
};
use crate::storage::{self, Loaded};

pub const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameRef {
    /// discord application id, the ipc client_id
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresetAssets {
    pub large_image: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub game: GameRef,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub assets: PresetAssets,
    #[serde(default)]
    pub timer: Timer,
    #[serde(default)]
    pub party: Option<PartyInput>,
    #[serde(default)]
    pub buttons: Vec<ButtonInput>,
}

impl Preset {
    /// A blank preset for a game, named after it.
    pub fn new(game: GameRef) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: game.name.clone(),
            game,
            details: String::new(),
            state: String::new(),
            assets: PresetAssets::default(),
            timer: Timer::None,
            party: None,
            buttons: Vec::new(),
        }
    }

    /// `applied_at` is the unix time the preset went live, which anchors the
    /// elapsed timer.
    pub fn to_input(&self, applied_at: i64) -> ActivityInput {
        ActivityInput {
            details: self.details.clone(),
            state: self.state.clone(),
            large_image: self.assets.large_image.clone(),
            large_text: self.assets.large_text.clone(),
            small_image: self.assets.small_image.clone(),
            small_text: self.assets.small_text.clone(),
            start: self.timer.resolve(applied_at),
            party: self.party,
            buttons: self.buttons.clone(),
        }
    }

    pub fn build(&self, applied_at: i64) -> Result<Activity, ActivityError> {
        build_activity(&self.to_input(applied_at))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetStore {
    pub version: u32,
    /// The preset to put live on launch; none means start with no presence.
    #[serde(default)]
    pub last_preset_id: Option<String>,
    #[serde(default)]
    pub presets: Vec<Preset>,
}

impl Default for PresetStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            last_preset_id: None,
            presets: Vec::new(),
        }
    }
}

impl PresetStore {
    pub fn load(path: &Path) -> Loaded<PresetStore> {
        let mut loaded: Loaded<PresetStore> = storage::load_json(path);
        if loaded.value.version != STORE_VERSION {
            // written by a newer (or broken) build: don't touch it, but don't
            // pretend we understood it either
            loaded.warning = Some(format!(
                "{} has version {}, this build understands version {}; ignoring it",
                path.display(),
                loaded.value.version,
                STORE_VERSION
            ));
            loaded.value = PresetStore::default();
        }
        loaded
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        storage::save_json(path, self)
    }

    pub fn get(&self, id: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id == id)
    }

    /// Insert, or replace the preset with the same id (keeping its position).
    pub fn upsert(&mut self, preset: Preset) {
        match self.presets.iter_mut().find(|p| p.id == preset.id) {
            Some(slot) => *slot = preset,
            None => self.presets.push(preset),
        }
    }

    /// Remove a preset. Also forgets it as the last preset.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|p| p.id != id);
        if self.last_preset_id.as_deref() == Some(id) {
            self.last_preset_id = None;
        }
        self.presets.len() != before
    }

    /// Copy a preset right after the original, with a new id.
    pub fn duplicate(&mut self, id: &str) -> Option<Preset> {
        let pos = self.presets.iter().position(|p| p.id == id)?;
        let mut copy = self.presets[pos].clone();
        copy.id = Uuid::new_v4().to_string();
        copy.name = format!("{} copy", copy.name);
        self.presets.insert(pos + 1, copy.clone());
        Some(copy)
    }
}

/// The preset that is currently being shown on discord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub preset_id: String,
    /// Unix seconds. The elapsed timer counts from here (minus any offset).
    pub applied_at: i64,
    timer: Timer,
}

impl Live {
    pub fn start(preset: &Preset, now: i64) -> Self {
        Self {
            preset_id: preset.id.clone(),
            applied_at: now,
            timer: preset.timer,
        }
    }

    /// Call when the live preset is edited. The clock keeps running through
    /// edits to text and buttons; it restarts only if the timer setting itself
    /// changed, because the user asked for a different clock.
    pub fn refresh(&mut self, preset: &Preset, now: i64) {
        if self.timer != preset.timer {
            self.timer = preset.timer;
            self.applied_at = now;
        }
    }

    pub fn build(&self, preset: &Preset) -> Result<Activity, ActivityError> {
        preset.build(self.applied_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::UrlProblem;
    use serde_json::{json, Value};
    use std::collections::BTreeSet;

    fn game() -> GameRef {
        GameRef {
            id: "1402418491272986635".into(),
            name: "Minecraft".into(),
        }
    }

    fn full_preset() -> Preset {
        let mut p = Preset::new(game());
        p.details = "Building a base".into();
        p.state = "Survival".into();
        p.assets = PresetAssets {
            large_image: "big".into(),
            large_text: "Big".into(),
            small_image: "small".into(),
            small_text: "Small".into(),
        };
        p.timer = Timer::Offset { offset_seconds: 90 };
        p.party = Some(PartyInput { current: 2, max: 4 });
        p.buttons = vec![ButtonInput {
            label: "Join".into(),
            url: "https://example.com".into(),
        }];
        p
    }

    #[test]
    fn new_presets_are_named_after_the_game_and_get_unique_ids() {
        let a = Preset::new(game());
        let b = Preset::new(game());
        assert_eq!(a.name, "Minecraft");
        assert_ne!(a.id, b.id);
        assert_eq!(a.timer, Timer::None);
        assert!(a.buttons.is_empty() && a.party.is_none());
    }

    #[test]
    fn builds_an_activity_with_the_timer_anchored_to_applied_at() {
        let p = full_preset();
        let a = p.build(1_700_000_000).unwrap();
        assert_eq!(a.timestamps.unwrap().start, 1_700_000_000 - 90);
        assert_eq!(a.details.as_deref(), Some("Building a base"));
        assert_eq!(a.party.unwrap().size, [2, 4]);
        assert_eq!(a.buttons.len(), 1);
    }

    #[test]
    fn invalid_presets_surface_the_builder_error() {
        let mut p = full_preset();
        p.buttons[0].url = "nope".into();
        let err = p.build(0).unwrap_err();
        assert_eq!(
            err,
            ActivityError::BadButtonUrl {
                index: 0,
                problem: UrlProblem::Unparseable
            }
        );
    }

    #[test]
    fn json_matches_the_shared_schema_field_names() {
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../shared/preset.schema.json"))
                .unwrap(),
        )
        .unwrap();
        let keys = |v: &Value| -> BTreeSet<String> { v.as_object().unwrap().keys().cloned().collect() };

        let store = PresetStore {
            version: 1,
            last_preset_id: Some("x".into()),
            presets: vec![full_preset()],
        };
        let out = serde_json::to_value(&store).unwrap();
        assert_eq!(keys(&out), keys(&schema["properties"]));

        let preset = &out["presets"][0];
        assert_eq!(keys(preset), keys(&schema["$defs"]["preset"]["properties"]));
        assert_eq!(
            keys(&preset["assets"]),
            keys(&schema["$defs"]["preset"]["properties"]["assets"]["properties"])
        );
        assert_eq!(
            keys(&preset["game"]),
            keys(&schema["$defs"]["preset"]["properties"]["game"]["properties"])
        );
        assert_eq!(
            keys(&preset["timer"]),
            keys(&schema["$defs"]["preset"]["properties"]["timer"]["properties"])
        );
        assert_eq!(
            keys(&preset["party"]),
            keys(&schema["$defs"]["preset"]["properties"]["party"]["properties"])
        );
        assert_eq!(
            keys(&preset["buttons"][0]),
            keys(&schema["$defs"]["preset"]["properties"]["buttons"]["items"]["properties"])
        );
    }

    #[test]
    fn timer_none_serializes_without_an_offset() {
        let p = Preset::new(game());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["timer"], json!({"mode": "none"}));
        assert_eq!(v["party"], Value::Null);
    }

    #[test]
    fn minimal_json_loads_with_defaults() {
        let p: Preset = serde_json::from_str(
            r#"{"id":"a","name":"n","game":{"id":"1402418491272986635","name":"Minecraft"}}"#,
        )
        .unwrap();
        assert_eq!(p.details, "");
        assert_eq!(p.timer, Timer::None);
        assert!(p.buttons.is_empty());
    }

    #[test]
    fn store_round_trips_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let mut store = PresetStore::default();
        store.upsert(full_preset());
        store.last_preset_id = Some(store.presets[0].id.clone());
        store.save(&path).unwrap();
        let loaded = PresetStore::load(&path);
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.value, store);
    }

    #[test]
    fn missing_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = PresetStore::load(&dir.path().join("presets.json"));
        assert_eq!(loaded.value, PresetStore::default());
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn corrupt_store_starts_fresh_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        std::fs::write(&path, "[[[").unwrap();
        let loaded = PresetStore::load(&path);
        assert_eq!(loaded.value, PresetStore::default());
        assert!(loaded.warning.is_some());
    }

    #[test]
    fn a_store_from_the_future_is_ignored_not_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        std::fs::write(&path, r#"{"version":99,"presets":[]}"#).unwrap();
        let loaded = PresetStore::load(&path);
        assert_eq!(loaded.value, PresetStore::default());
        assert!(loaded.warning.unwrap().contains("99"));
    }

    #[test]
    fn upsert_replaces_in_place_and_appends_new() {
        let mut store = PresetStore::default();
        let a = Preset::new(game());
        let b = Preset::new(game());
        store.upsert(a.clone());
        store.upsert(b.clone());
        let mut a2 = a.clone();
        a2.details = "changed".into();
        store.upsert(a2);
        assert_eq!(store.presets.len(), 2);
        assert_eq!(store.presets[0].details, "changed");
        assert_eq!(store.presets[1].id, b.id);
    }

    #[test]
    fn remove_forgets_the_last_preset_too() {
        let mut store = PresetStore::default();
        let a = Preset::new(game());
        store.upsert(a.clone());
        store.last_preset_id = Some(a.id.clone());
        assert!(store.remove(&a.id));
        assert!(store.last_preset_id.is_none());
        assert!(!store.remove(&a.id));
    }

    #[test]
    fn duplicate_goes_right_after_the_original() {
        let mut store = PresetStore::default();
        let a = Preset::new(game());
        let b = Preset::new(game());
        store.upsert(a.clone());
        store.upsert(b.clone());
        let copy = store.duplicate(&a.id).unwrap();
        assert_ne!(copy.id, a.id);
        assert_eq!(copy.name, "Minecraft copy");
        assert_eq!(store.presets[1].id, copy.id);
        assert_eq!(store.presets[2].id, b.id);
        assert!(store.duplicate("missing").is_none());
    }

    #[test]
    fn live_timer_survives_text_edits_but_restarts_when_the_timer_changes() {
        let mut p = Preset::new(game());
        p.timer = Timer::Now;
        let mut live = Live::start(&p, 1000);
        assert_eq!(live.build(&p).unwrap().timestamps.unwrap().start, 1000);

        p.details = "new text".into();
        live.refresh(&p, 1500);
        assert_eq!(live.build(&p).unwrap().timestamps.unwrap().start, 1000);

        p.timer = Timer::Offset { offset_seconds: 60 };
        live.refresh(&p, 2000);
        assert_eq!(live.build(&p).unwrap().timestamps.unwrap().start, 1940);

        p.timer = Timer::None;
        live.refresh(&p, 2500);
        assert!(live.build(&p).unwrap().timestamps.is_none());
    }
}
