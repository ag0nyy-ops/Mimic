//! Tiny json-on-disk helpers: atomic writes, and a loader that never loses
//! user data silently (an unreadable file is moved aside, not overwritten).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Write `value` as pretty json by writing a sibling temp file and renaming it
/// over the target, so a crash can't leave half a file behind.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp: PathBuf = path.to_path_buf();
    let mut name = tmp.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    tmp.set_file_name(name);
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

pub struct Loaded<T> {
    pub value: T,
    /// Set when the file existed but couldn't be used.
    pub warning: Option<String>,
}

/// Load json from `path`. Missing file: the default. Unreadable or invalid
/// file: the default, after copying the bad file to `<name>.corrupt-<unix time>`
/// so nothing is lost, plus a warning to show or log.
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Loaded<T> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                value: T::default(),
                warning: None,
            }
        }
        Err(e) => {
            return Loaded {
                value: T::default(),
                warning: Some(format!("couldn't read {}: {e}", path.display())),
            }
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Loaded {
            value,
            warning: None,
        },
        Err(e) => {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mut backup = path.to_path_buf();
            let mut name = backup.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".corrupt-{stamp}"));
            backup.set_file_name(name);
            let saved = fs::copy(path, &backup).is_ok();
            Loaded {
                value: T::default(),
                warning: Some(format!(
                    "{} was unreadable ({e}); starting fresh{}",
                    path.display(),
                    if saved {
                        format!(", old file kept as {}", backup.display())
                    } else {
                        String::new()
                    }
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Thing {
        n: u32,
        s: String,
    }

    #[test]
    fn round_trip_and_atomic_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("thing.json");
        save_json(&path, &Thing { n: 1, s: "a".into() }).unwrap();
        save_json(&path, &Thing { n: 2, s: "b".into() }).unwrap();
        let loaded: Loaded<Thing> = load_json(&path);
        assert_eq!(loaded.value, Thing { n: 2, s: "b".into() });
        assert!(loaded.warning.is_none());
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["thing.json".to_string()], "no temp file left behind");
    }

    #[test]
    fn missing_file_is_just_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let loaded: Loaded<Thing> = load_json(&dir.path().join("nope.json"));
        assert_eq!(loaded.value, Thing::default());
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn corrupt_file_is_kept_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thing.json");
        fs::write(&path, "{ this is not json").unwrap();
        let loaded: Loaded<Thing> = load_json(&path);
        assert_eq!(loaded.value, Thing::default());
        assert!(loaded.warning.is_some());
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let kept = fs::read_to_string(dir.path().join(&backups[0])).unwrap();
        assert_eq!(kept, "{ this is not json");
    }
}
