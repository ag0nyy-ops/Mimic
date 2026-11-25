//! macOS and Linux: discord listens on a unix domain socket called
//! `discord-ipc-N` in its temp or runtime dir.
//!
//! Where exactly depends on how discord was installed, so a connect walks a
//! list of directories and uses the first socket that answers:
//!
//! - the plain spot: `$XDG_RUNTIME_DIR`, `$TMPDIR`, `$TMP`, `$TEMP`, then `/tmp`
//!   (macOS sets `$TMPDIR` to the per-user `/var/folders/.../T/`, which is
//!   where discord puts it)
//! - linux only, under each of those: the flatpak builds
//!   (`app/com.discordapp.Discord/`, canary, vesktop) and the snap builds
//!   (`snap.discord/`, `snap.discord-canary/`)

use std::path::{Path, PathBuf};

/// Env vars that can point at the directory discord's socket is in, in the
/// order discord's own clients check them.
pub const DIR_VARS: [&str; 4] = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"];

/// Where sandboxed linux installs of discord put the socket, relative to one of
/// the dirs above. The empty entry is the dir itself.
const LINUX_SUBDIRS: [&str; 7] = [
    "",
    "app/com.discordapp.Discord",
    "app/com.discordapp.DiscordCanary",
    "app/dev.vencord.Vesktop",
    ".flatpak/dev.vencord.Vesktop/xdg-run",
    "snap.discord",
    "snap.discord-canary",
];

/// `discord-ipc-N`
pub fn socket_name(index: u8) -> String {
    format!("discord-ipc-{index}")
}

/// Every directory to look for the socket in, most likely first, without
/// duplicates. `env` reads an environment variable (a parameter so tests
/// don't have to touch the real environment). `linux` adds the flatpak and
/// snap locations.
pub fn candidate_dirs(env: impl Fn(&str) -> Option<String>, linux: bool) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = DIR_VARS
        .iter()
        .filter_map(|var| env(var))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect();
    bases.push(PathBuf::from("/tmp"));

    let subdirs: &[&str] = if linux { &LINUX_SUBDIRS } else { &[""] };

    let mut dirs: Vec<PathBuf> = Vec::new();
    for sub in subdirs {
        for base in &bases {
            let dir = if sub.is_empty() {
                base.clone()
            } else {
                base.join(sub)
            };
            if !dirs.iter().any(|d| same_dir(d, &dir)) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// `/tmp` and `/tmp/` are the same place.
fn same_dir(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
}

#[cfg(unix)]
pub use imp::UnixConnector;

#[cfg(unix)]
mod imp {
    use std::io;
    use std::path::PathBuf;

    use tokio::net::UnixStream;

    use super::{candidate_dirs, socket_name};
    use crate::manager::Connector;

    /// Connects to discord's unix socket.
    #[derive(Debug, Clone)]
    pub struct UnixConnector {
        dirs: Vec<PathBuf>,
    }

    impl UnixConnector {
        /// Look in the directories the environment points at.
        pub fn from_env() -> Self {
            let dirs = candidate_dirs(|var| std::env::var(var).ok(), cfg!(target_os = "linux"));
            log::debug!("looking for discord's socket in {dirs:?}");
            Self { dirs }
        }

        /// Look in exactly these directories, in this order.
        pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
            Self { dirs }
        }

        pub fn dirs(&self) -> &[PathBuf] {
            &self.dirs
        }
    }

    impl Default for UnixConnector {
        fn default() -> Self {
            Self::from_env()
        }
    }

    impl Connector for UnixConnector {
        type Stream = UnixStream;

        async fn connect(&self, index: u8) -> io::Result<UnixStream> {
            let name = socket_name(index);
            let mut last = None;
            for dir in &self.dirs {
                let path = dir.join(&name);
                match UnixStream::connect(&path).await {
                    Ok(stream) => {
                        log::debug!("found discord at {}", path.display());
                        return Ok(stream);
                    }
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no socket dirs")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |var| {
            pairs
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn socket_names() {
        assert_eq!(socket_name(0), "discord-ipc-0");
        assert_eq!(socket_name(9), "discord-ipc-9");
    }

    #[test]
    fn mac_uses_tmpdir_then_tmp() {
        let env = [("TMPDIR", "/var/folders/xy/abc123/T/")];
        let dirs = candidate_dirs(env_of(&env), false);
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/var/folders/xy/abc123/T/"),
                PathBuf::from("/tmp")
            ]
        );
    }

    #[test]
    fn nothing_set_falls_back_to_tmp() {
        assert_eq!(
            candidate_dirs(env_of(&[]), false),
            vec![PathBuf::from("/tmp")]
        );
    }

    #[test]
    fn empty_and_duplicate_values_are_skipped() {
        let env = [
            ("XDG_RUNTIME_DIR", ""),
            ("TMPDIR", "/tmp/"),
            ("TMP", "/tmp"),
        ];
        assert_eq!(
            candidate_dirs(env_of(&env), false),
            vec![PathBuf::from("/tmp/")]
        );
    }

    #[test]
    fn linux_checks_plain_spots_before_flatpak_and_snap() {
        let env = [("XDG_RUNTIME_DIR", "/run/user/1000")];
        let dirs = candidate_dirs(env_of(&env), true);
        assert_eq!(dirs[0], PathBuf::from("/run/user/1000"));
        assert_eq!(dirs[1], PathBuf::from("/tmp"));
        assert!(dirs.contains(&PathBuf::from("/run/user/1000/app/com.discordapp.Discord")));
        assert!(dirs.contains(&PathBuf::from("/run/user/1000/snap.discord")));
        assert!(dirs.contains(&PathBuf::from(
            "/run/user/1000/.flatpak/dev.vencord.Vesktop/xdg-run"
        )));
        let plain = dirs.iter().position(|d| d == Path::new("/tmp")).unwrap();
        let flatpak = dirs
            .iter()
            .position(|d| d.ends_with("app/com.discordapp.Discord"))
            .unwrap();
        assert!(plain < flatpak);
    }

    #[cfg(unix)]
    mod live {
        use super::super::*;
        use crate::activity::{build_activity, ActivityInput};
        use crate::frame::{encode, FrameReader, Opcode};
        use crate::manager::{spawn, ConnectionState, Connector, ManagerConfig};
        use serde_json::{json, Value};
        use std::time::Duration;
        use tokio::io::AsyncWriteExt;
        use tokio::net::UnixListener;
        use tokio::time::timeout;

        #[tokio::test]
        async fn connects_to_the_first_dir_that_has_the_socket() {
            let missing = tempfile::tempdir().unwrap();
            let real = tempfile::tempdir().unwrap();
            let _listener = UnixListener::bind(real.path().join("discord-ipc-3")).unwrap();

            let connector = UnixConnector::with_dirs(vec![
                missing.path().to_path_buf(),
                real.path().to_path_buf(),
            ]);
            assert!(connector.connect(3).await.is_ok());
            let err = connector.connect(0).await.unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        }

        /// The whole manager against a fake discord on a real unix socket.
        #[tokio::test]
        async fn goes_live_over_a_real_socket() {
            let dir = tempfile::tempdir().unwrap();
            let listener = UnixListener::bind(dir.path().join("discord-ipc-1")).unwrap();
            let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();

            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (read_half, mut w) = tokio::io::split(stream);
                let mut reader = FrameReader::new(read_half);
                let hello = reader.read_frame().await.unwrap();
                assert_eq!(hello.opcode, Opcode::Handshake);
                let hello: Value = serde_json::from_str(&hello.payload).unwrap();
                seen_tx.send(hello).unwrap();
                let ready = json!({"cmd":"DISPATCH","data":{"v":1,"config":{},"user":{"username":"tester"}},"evt":"READY","nonce":null});
                w.write_all(&encode(Opcode::Frame, &ready.to_string()).unwrap())
                    .await
                    .unwrap();
                while let Ok(frame) = reader.read_frame().await {
                    if frame.opcode != Opcode::Frame {
                        continue;
                    }
                    let v: Value = serde_json::from_str(&frame.payload).unwrap();
                    let reply =
                        json!({"cmd":"SET_ACTIVITY","data":{},"evt":null,"nonce":v["nonce"]});
                    w.write_all(&encode(Opcode::Frame, &reply.to_string()).unwrap())
                        .await
                        .unwrap();
                    seen_tx.send(v).unwrap();
                }
            });

            let handle = spawn(
                UnixConnector::with_dirs(vec![dir.path().to_path_buf()]),
                ManagerConfig::new(4321),
            );
            let activity = build_activity(&ActivityInput {
                details: "over a unix socket".into(),
                ..Default::default()
            })
            .unwrap();
            handle.set("1402418491272986635", Some(activity)).unwrap();

            let hello = timeout(Duration::from_secs(10), seen_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(hello["client_id"], "1402418491272986635");
            let set = timeout(Duration::from_secs(10), seen_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(set["cmd"], "SET_ACTIVITY");
            assert_eq!(set["args"]["activity"]["details"], "over a unix socket");

            let mut rx = handle.subscribe();
            timeout(
                Duration::from_secs(10),
                rx.wait_for(|s| matches!(s, ConnectionState::Connected { .. })),
            )
            .await
            .unwrap()
            .unwrap();
        }
    }
}
