//! App settings that live in `settings.json`. (Launch at login isn't here: the
//! os registry is the source of truth for that.)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Closing the window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Minimizing the window hides it to the tray.
    pub minimize_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            minimize_to_tray: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hide_to_tray() {
        let s = Settings::default();
        assert!(s.close_to_tray && s.minimize_to_tray);
    }

    #[test]
    fn partial_files_fill_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{"closeToTray":false}"#).unwrap();
        assert!(!s.close_to_tray);
        assert!(s.minimize_to_tray);
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn json_keys_are_camel_case() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert!(v.get("closeToTray").is_some() && v.get("minimizeToTray").is_some());
    }
}
