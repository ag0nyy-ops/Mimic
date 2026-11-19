//! Turning what a user typed into the activity json discord accepts.
//!
//! Rules (see `shared/README.md`):
//! - text is trimmed, empty text is dropped completely, never sent as `""`
//! - over-long text is cut to discord's limit; structural problems (too many
//!   buttons, bad urls, impossible party sizes) are errors instead, because
//!   silently changing those would be confusing
//! - the output only contains fields that have a value

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub const MAX_TEXT_LEN: usize = 128;
pub const MAX_IMAGE_KEY_LEN: usize = 256;
pub const MAX_BUTTONS: usize = 2;
pub const MAX_BUTTON_LABEL_LEN: usize = 32;
pub const MAX_BUTTON_URL_LEN: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlProblem {
    Missing,
    Unparseable,
    NotHttp,
    TooLong,
}

impl std::fmt::Display for UrlProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "is missing",
            Self::Unparseable => "isn't a valid url",
            Self::NotHttp => "has to start with http:// or https://",
            Self::TooLong => "is longer than 512 characters",
        })
    }
}

/// `index` is the 0-based position in the submitted button list, so a ui can
/// point at the right row.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActivityError {
    #[error("discord allows at most 2 buttons, got {count}")]
    TooManyButtons { count: usize },
    #[error("button {} needs a label", index + 1)]
    ButtonLabelMissing { index: usize },
    #[error("button {} label is {len} characters, the limit is 32", index + 1)]
    ButtonLabelTooLong { index: usize, len: usize },
    #[error("button {} url {problem}", index + 1)]
    BadButtonUrl { index: usize, problem: UrlProblem },
    #[error("party size is invalid: {0}")]
    BadParty(String),
}

impl ActivityError {
    /// Which form field the error is about, as a dotted path a ui can match:
    /// `buttons`, `buttons.0.label`, `buttons.1.url`, `party`.
    pub fn field(&self) -> String {
        match self {
            Self::TooManyButtons { .. } => "buttons".into(),
            Self::ButtonLabelMissing { index } | Self::ButtonLabelTooLong { index, .. } => {
                format!("buttons.{index}.label")
            }
            Self::BadButtonUrl { index, .. } => format!("buttons.{index}.url"),
            Self::BadParty(_) => "party".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ButtonInput {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyInput {
    pub current: u32,
    pub max: u32,
}

/// How the elapsed timer should behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Timer {
    /// No timestamp.
    #[default]
    None,
    /// Count from the moment the presence was applied.
    Now,
    /// Count from `offset_seconds` before the moment it was applied, e.g. 3600
    /// shows "01:00:00 elapsed" immediately.
    Offset {
        #[serde(rename = "offsetSeconds", default)]
        offset_seconds: u64,
    },
}

impl Timer {
    /// The `timestamps.start` value (unix seconds) for a presence that was
    /// applied at `applied_at`. Pass the same `applied_at` every time the same
    /// presence is re-sent, or the clock would restart on every edit.
    pub fn resolve(self, applied_at: i64) -> Option<i64> {
        match self {
            Timer::None => None,
            Timer::Now => Some(applied_at),
            Timer::Offset { offset_seconds } => {
                let offset = i64::try_from(offset_seconds).unwrap_or(i64::MAX);
                Some(applied_at.saturating_sub(offset).max(0))
            }
        }
    }
}

/// Raw user input. Any field may be blank.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActivityInput {
    pub details: String,
    pub state: String,
    pub large_image: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
    /// `timestamps.start` in unix seconds, already resolved from a [`Timer`].
    pub start: Option<i64>,
    pub party: Option<PartyInput>,
    pub buttons: Vec<ButtonInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timestamps {
    pub start: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Assets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_text: Option<String>,
}

impl Assets {
    fn is_empty(&self) -> bool {
        self.large_image.is_none()
            && self.large_text.is_none()
            && self.small_image.is_none()
            && self.small_text.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Party {
    pub size: [u32; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Button {
    pub label: String,
    pub url: String,
}

/// A validated activity, ready to serialize. Absent fields are skipped.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Activity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamps: Option<Timestamps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<Assets>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Party>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<Button>,
}

fn clean(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

fn check_url(index: usize, raw: &str) -> Result<String, ActivityError> {
    let fail = |problem| ActivityError::BadButtonUrl { index, problem };
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(fail(UrlProblem::Missing));
    }
    if raw.chars().count() > MAX_BUTTON_URL_LEN {
        return Err(fail(UrlProblem::TooLong));
    }
    let parsed = Url::parse(raw).map_err(|_| fail(UrlProblem::Unparseable))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(fail(UrlProblem::NotHttp));
    }
    if parsed.host_str().map_or(true, str::is_empty) {
        return Err(fail(UrlProblem::Unparseable));
    }
    Ok(raw.to_owned())
}

pub fn build_activity(input: &ActivityInput) -> Result<Activity, ActivityError> {
    // Buttons: fully blank rows are ignored, half-filled rows are errors.
    let filled: Vec<(usize, &ButtonInput)> = input
        .buttons
        .iter()
        .enumerate()
        .filter(|(_, b)| !(b.label.trim().is_empty() && b.url.trim().is_empty()))
        .collect();
    if filled.len() > MAX_BUTTONS {
        return Err(ActivityError::TooManyButtons {
            count: filled.len(),
        });
    }
    let mut buttons = Vec::with_capacity(filled.len());
    for (index, b) in filled {
        let label = b.label.trim();
        if label.is_empty() {
            return Err(ActivityError::ButtonLabelMissing { index });
        }
        let len = label.chars().count();
        if len > MAX_BUTTON_LABEL_LEN {
            return Err(ActivityError::ButtonLabelTooLong { index, len });
        }
        buttons.push(Button {
            label: label.to_owned(),
            url: check_url(index, &b.url)?,
        });
    }

    let party = match input.party {
        None => None,
        Some(p) => {
            if p.current < 1 || p.max < 1 {
                return Err(ActivityError::BadParty(
                    "both numbers have to be at least 1".into(),
                ));
            }
            if p.current > p.max {
                return Err(ActivityError::BadParty(format!(
                    "{} can't be bigger than the max of {}",
                    p.current, p.max
                )));
            }
            Some(Party {
                size: [p.current, p.max],
            })
        }
    };

    let assets = Assets {
        large_image: clean(&input.large_image, MAX_IMAGE_KEY_LEN),
        large_text: clean(&input.large_text, MAX_TEXT_LEN),
        small_image: clean(&input.small_image, MAX_IMAGE_KEY_LEN),
        small_text: clean(&input.small_text, MAX_TEXT_LEN),
    };

    Ok(Activity {
        details: clean(&input.details, MAX_TEXT_LEN),
        state: clean(&input.state, MAX_TEXT_LEN),
        timestamps: input.start.map(|start| Timestamps { start }),
        assets: (!assets.is_empty()).then_some(assets),
        party,
        buttons,
    })
}

pub fn new_nonce() -> String {
    Uuid::new_v4().to_string()
}

/// The json for a `SET_ACTIVITY` command. `None` clears the presence
/// (`"activity": null`).
pub fn set_activity_payload(pid: u32, activity: Option<&Activity>, nonce: &str) -> String {
    json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity },
        "nonce": nonce,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn to_json(a: &Activity) -> Value {
        serde_json::to_value(a).unwrap()
    }

    fn button(label: &str, url: &str) -> ButtonInput {
        ButtonInput {
            label: label.into(),
            url: url.into(),
        }
    }

    #[test]
    fn empty_input_builds_an_empty_object() {
        let a = build_activity(&ActivityInput::default()).unwrap();
        assert_eq!(to_json(&a), json!({}));
    }

    #[test]
    fn every_field() {
        let a = build_activity(&ActivityInput {
            details: "Ranked match".into(),
            state: "In queue".into(),
            large_image: "big".into(),
            large_text: "Big hover".into(),
            small_image: "small".into(),
            small_text: "Small hover".into(),
            start: Some(1_700_000_000),
            party: Some(PartyInput { current: 2, max: 4 }),
            buttons: vec![
                button("Join", "https://example.com/join"),
                button("Site", "http://example.org"),
            ],
        })
        .unwrap();
        assert_eq!(
            to_json(&a),
            json!({
                "details": "Ranked match",
                "state": "In queue",
                "timestamps": { "start": 1_700_000_000i64 },
                "assets": {
                    "large_image": "big", "large_text": "Big hover",
                    "small_image": "small", "small_text": "Small hover"
                },
                "party": { "size": [2, 4] },
                "buttons": [
                    { "label": "Join", "url": "https://example.com/join" },
                    { "label": "Site", "url": "http://example.org" }
                ]
            })
        );
    }

    #[test]
    fn blank_and_whitespace_fields_are_stripped_not_sent_empty() {
        let a = build_activity(&ActivityInput {
            details: "   ".into(),
            state: "\t\n".into(),
            large_image: "".into(),
            small_text: " ".into(),
            ..Default::default()
        })
        .unwrap();
        let text = serde_json::to_string(&a).unwrap();
        assert_eq!(text, "{}");
        assert!(!text.contains("\"\""));
    }

    #[test]
    fn partial_assets_only_contain_what_was_filled() {
        let a = build_activity(&ActivityInput {
            large_image: "only_big".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(to_json(&a), json!({ "assets": { "large_image": "only_big" } }));
    }

    #[test]
    fn text_is_trimmed() {
        let a = build_activity(&ActivityInput {
            details: "  hello  ".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(a.details.as_deref(), Some("hello"));
    }

    #[test]
    fn long_text_is_cut_to_128_characters() {
        let long = "é".repeat(300);
        let a = build_activity(&ActivityInput {
            details: long.clone(),
            state: long.clone(),
            large_text: long.clone(),
            small_text: long,
            large_image: "x".repeat(400),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(a.details.unwrap().chars().count(), 128);
        assert_eq!(a.state.unwrap().chars().count(), 128);
        let assets = a.assets.unwrap();
        assert_eq!(assets.large_text.unwrap().chars().count(), 128);
        assert_eq!(assets.small_text.unwrap().chars().count(), 128);
        assert_eq!(assets.large_image.unwrap().chars().count(), 256);
    }

    #[test]
    fn text_exactly_at_the_limit_is_untouched() {
        let s = "a".repeat(128);
        let a = build_activity(&ActivityInput {
            details: s.clone(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(a.details.unwrap(), s);
    }

    #[test]
    fn blank_button_rows_are_ignored() {
        let a = build_activity(&ActivityInput {
            buttons: vec![button("", ""), button("  ", " "), button("Go", "https://a.b")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(a.buttons.len(), 1);
    }

    #[test]
    fn three_buttons_is_an_error() {
        let err = build_activity(&ActivityInput {
            buttons: vec![
                button("a", "https://a.b"),
                button("b", "https://a.b"),
                button("c", "https://a.b"),
            ],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err, ActivityError::TooManyButtons { count: 3 });
    }

    #[test]
    fn button_label_limit_is_32() {
        let ok = build_activity(&ActivityInput {
            buttons: vec![button(&"a".repeat(32), "https://a.b")],
            ..Default::default()
        });
        assert!(ok.is_ok());
        let err = build_activity(&ActivityInput {
            buttons: vec![button(&"a".repeat(33), "https://a.b")],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err, ActivityError::ButtonLabelTooLong { index: 0, len: 33 });
    }

    #[test]
    fn button_with_url_but_no_label() {
        let err = build_activity(&ActivityInput {
            buttons: vec![button("", "https://a.b")],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err, ActivityError::ButtonLabelMissing { index: 0 });
    }

    #[test]
    fn button_with_label_but_no_url() {
        let err = build_activity(&ActivityInput {
            buttons: vec![button("Go", "  ")],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            err,
            ActivityError::BadButtonUrl {
                index: 0,
                problem: UrlProblem::Missing
            }
        );
    }

    #[test]
    fn bad_urls_are_typed_errors_with_the_row_index() {
        let cases = [
            ("not a url", UrlProblem::Unparseable),
            ("example.com", UrlProblem::Unparseable),
            ("ftp://example.com", UrlProblem::NotHttp),
            ("javascript:alert(1)", UrlProblem::NotHttp),
            ("file:///c:/windows", UrlProblem::NotHttp),
            ("https://", UrlProblem::Unparseable),
        ];
        for (url, problem) in cases {
            let err = build_activity(&ActivityInput {
                buttons: vec![button("ok", "https://fine.example"), button("Go", url)],
                ..Default::default()
            })
            .unwrap_err();
            assert_eq!(
                err,
                ActivityError::BadButtonUrl { index: 1, problem },
                "url {url:?}"
            );
        }
    }

    #[test]
    fn url_length_limit() {
        let long = format!("https://example.com/{}", "a".repeat(600));
        let err = build_activity(&ActivityInput {
            buttons: vec![button("Go", &long)],
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(
            err,
            ActivityError::BadButtonUrl {
                index: 0,
                problem: UrlProblem::TooLong
            }
        );
    }

    #[test]
    fn button_values_are_trimmed() {
        let a = build_activity(&ActivityInput {
            buttons: vec![button("  Go  ", "  https://example.com  ")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(a.buttons[0].label, "Go");
        assert_eq!(a.buttons[0].url, "https://example.com");
    }

    #[test]
    fn party_rules() {
        let ok = |current, max| {
            build_activity(&ActivityInput {
                party: Some(PartyInput { current, max }),
                ..Default::default()
            })
        };
        assert_eq!(ok(1, 1).unwrap().party.unwrap().size, [1, 1]);
        assert_eq!(ok(3, 8).unwrap().party.unwrap().size, [3, 8]);
        assert!(matches!(ok(0, 4), Err(ActivityError::BadParty(_))));
        assert!(matches!(ok(2, 0), Err(ActivityError::BadParty(_))));
        assert!(matches!(ok(5, 4), Err(ActivityError::BadParty(_))));
    }

    #[test]
    fn timer_resolution() {
        let now = 1_700_000_000;
        assert_eq!(Timer::None.resolve(now), None);
        assert_eq!(Timer::Now.resolve(now), Some(now));
        assert_eq!(
            Timer::Offset { offset_seconds: 3600 }.resolve(now),
            Some(now - 3600)
        );
        assert_eq!(Timer::Offset { offset_seconds: 0 }.resolve(now), Some(now));
        // absurd offsets don't go negative or overflow
        assert_eq!(Timer::Offset { offset_seconds: u64::MAX }.resolve(now), Some(0));
    }

    #[test]
    fn timer_json_matches_the_preset_schema() {
        let t: Timer = serde_json::from_str(r#"{"mode":"offset","offsetSeconds":300}"#).unwrap();
        assert_eq!(t, Timer::Offset { offset_seconds: 300 });
        let t: Timer = serde_json::from_str(r#"{"mode":"none"}"#).unwrap();
        assert_eq!(t, Timer::None);
        let t: Timer = serde_json::from_str(r#"{"mode":"now"}"#).unwrap();
        assert_eq!(t, Timer::Now);
        assert_eq!(
            serde_json::to_value(Timer::Offset { offset_seconds: 5 }).unwrap(),
            json!({ "mode": "offset", "offsetSeconds": 5 })
        );
    }

    #[test]
    fn set_activity_payload_shape() {
        let a = build_activity(&ActivityInput {
            details: "hi".into(),
            ..Default::default()
        })
        .unwrap();
        let text = set_activity_payload(4242, Some(&a), "nonce-1");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            json!({
                "cmd": "SET_ACTIVITY",
                "args": { "pid": 4242, "activity": { "details": "hi" } },
                "nonce": "nonce-1"
            })
        );
    }

    #[test]
    fn clearing_sends_an_explicit_null() {
        let v: Value = serde_json::from_str(&set_activity_payload(7, None, "n")).unwrap();
        assert_eq!(v["args"]["activity"], Value::Null);
        assert!(v["args"].as_object().unwrap().contains_key("activity"));
        assert_eq!(v["args"]["pid"], 7);
    }

    #[test]
    fn nonces_are_unique_uuids() {
        let a = new_nonce();
        let b = new_nonce();
        assert_ne!(a, b);
        assert!(Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn errors_know_which_field_they_belong_to() {
        assert_eq!(ActivityError::TooManyButtons { count: 3 }.field(), "buttons");
        assert_eq!(ActivityError::ButtonLabelMissing { index: 1 }.field(), "buttons.1.label");
        assert_eq!(
            ActivityError::ButtonLabelTooLong { index: 0, len: 40 }.field(),
            "buttons.0.label"
        );
        assert_eq!(
            ActivityError::BadButtonUrl { index: 1, problem: UrlProblem::Missing }.field(),
            "buttons.1.url"
        );
        assert_eq!(ActivityError::BadParty("x".into()).field(), "party");
    }

    #[test]
    fn error_messages_use_one_based_rows() {
        let e = ActivityError::BadButtonUrl {
            index: 1,
            problem: UrlProblem::NotHttp,
        };
        assert_eq!(e.to_string(), "button 2 url has to start with http:// or https://");
    }
}
