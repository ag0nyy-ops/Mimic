//! The connection manager.
//!
//! One background task owns the (single) connection to discord. A ui talks to
//! it through a cheap, cloneable [`PresenceHandle`] and watches a
//! [`ConnectionState`] to show what's going on.
//!
//! What it does:
//! - tries pipes `0..10` and uses the first one that connects
//! - handshakes with the wanted `client_id` and waits for READY
//! - sends the wanted activity, debounced (see [`crate::debounce`])
//! - answers pings, handles close frames and ERROR dispatches
//! - reconnects with exponential backoff when discord goes away, and re-sends
//!   the wanted activity once it is back
//! - one `client_id` per connection: asking for a different game drops the old
//!   connection and opens a fresh one (discord clears the old presence itself
//!   when its socket closes)
//!
//! The transport is abstracted behind [`Connector`], so tests drive the whole
//! thing over in-memory pipes and the windows build plugs in named pipes.

use std::future::Future;
use std::io;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::activity::{new_nonce, set_activity_payload, Activity};
use crate::debounce::{DebounceConfig, Debouncer};
use crate::frame::{write_frame, Frame, FrameReader, Opcode, ReadError};
use crate::message::{
    handshake_frame, parse_incoming, pong_frame, validate_client_id, CloseInfo, Incoming,
    ProtocolError, ReadyInfo, RpcError,
};

/// discord-ipc-0 through discord-ipc-9.
pub const PIPE_COUNT: u8 = 10;

/// Opens the local ipc endpoint with the given index (the `N` in
/// `discord-ipc-N`). An error means "nothing there".
pub trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn connect(&self, index: u8) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

/// What the ui should show. Times are "from now" snapshots in milliseconds, so
/// the ui can count down locally without sharing a clock with us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectionState {
    /// No game selected, nothing to connect for.
    Idle,
    /// Couldn't reach discord (not running, or it went away). Retrying.
    DiscordNotRunning { retry_in_ms: u64 },
    /// Looking for the pipe / handshaking.
    Connecting,
    /// Connected and READY.
    Connected { username: Option<String> },
    /// Discord rejected the application id. Won't retry until a different
    /// game is picked or the user hits retry.
    BadAppId { message: String },
    /// An update is waiting out the rate limit, or discord told us to slow down.
    RateLimited { retry_in_ms: u64 },
    /// Discord refused an activity, or the handshake never finished.
    Error { message: String },
}

#[derive(Debug, Clone, Copy)]
pub struct ManagerConfig {
    /// Our process id, sent as `pid` with every activity.
    pub pid: u32,
    pub debounce: DebounceConfig,
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    /// How long to wait for READY after sending the handshake.
    pub handshake_timeout: Duration,
}

impl ManagerConfig {
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            debounce: DebounceConfig::default(),
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug)]
enum Command {
    Set {
        client_id: String,
        activity: Option<Activity>,
    },
    Clear,
    Disconnect,
    RetryNow,
    Shutdown,
}

/// Cloneable remote control for the manager task.
#[derive(Clone)]
pub struct PresenceHandle {
    tx: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<ConnectionState>,
}

impl PresenceHandle {
    /// Show `activity` as `client_id`. If `client_id` differs from the current
    /// connection's, that connection is replaced. `None` connects but shows
    /// nothing.
    pub fn set(&self, client_id: &str, activity: Option<Activity>) -> Result<(), ProtocolError> {
        validate_client_id(client_id)?;
        let _ = self.tx.send(Command::Set {
            client_id: client_id.to_owned(),
            activity,
        });
        Ok(())
    }

    /// Clear the presence but stay connected as the same game.
    pub fn clear(&self) {
        let _ = self.tx.send(Command::Clear);
    }

    /// Close the connection (which also removes the presence) and go idle.
    pub fn disconnect(&self) {
        let _ = self.tx.send(Command::Disconnect);
    }

    /// Skip any backoff wait and try again now. Also un-sticks "bad app id".
    pub fn retry_now(&self) {
        let _ = self.tx.send(Command::RetryNow);
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Command::Shutdown);
    }

    pub fn state(&self) -> ConnectionState {
        self.state.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }
}

/// Spawn the manager on the current tokio runtime.
pub fn spawn<C: Connector>(connector: C, config: ManagerConfig) -> PresenceHandle {
    let (manager, handle) = Manager::new(connector, config);
    tokio::spawn(manager.run());
    handle
}

#[derive(Debug, Default)]
struct Desired {
    client_id: Option<String>,
    activity: Option<Activity>,
}

/// What a command changed, from the connection's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// the target client id changed (or we were told to disconnect)
    Client,
    /// same client, different activity
    Activity,
    Retry,
    Shutdown,
    Noop,
}

struct Inbox {
    rx: mpsc::UnboundedReceiver<Command>,
    desired: Desired,
}

impl Inbox {
    fn apply(&mut self, cmd: Command) -> Change {
        match cmd {
            Command::Set {
                client_id,
                activity,
            } => {
                if self.desired.client_id.as_deref() != Some(client_id.as_str()) {
                    self.desired = Desired {
                        client_id: Some(client_id),
                        activity,
                    };
                    Change::Client
                } else if self.desired.activity != activity {
                    self.desired.activity = activity;
                    Change::Activity
                } else {
                    Change::Noop
                }
            }
            Command::Clear => {
                if self.desired.activity.take().is_some() {
                    Change::Activity
                } else {
                    Change::Noop
                }
            }
            Command::Disconnect => {
                if self.desired.client_id.take().is_some() {
                    self.desired.activity = None;
                    Change::Client
                } else {
                    Change::Noop
                }
            }
            Command::RetryNow => Change::Retry,
            Command::Shutdown => Change::Shutdown,
        }
    }

    /// Wait for the next command and apply it. `None` means every handle was
    /// dropped, i.e. nobody can talk to us anymore. Cancel safe.
    async fn next_change(&mut self) -> Option<Change> {
        let cmd = self.rx.recv().await?;
        Some(self.apply(cmd))
    }
}

/// How a connection attempt or session ended.
#[derive(Debug)]
enum Outcome {
    /// A different client was wanted (or disconnect): start over right away.
    Switched,
    Shutdown,
    /// Permanent for this client id; wait for the user.
    Rejected(String),
    Lost(Lost),
}

#[derive(Debug, Clone, Copy)]
enum Lost {
    /// no pipe answered
    NotRunning,
    /// connected, then the pipe went away
    Dropped,
    HandshakeTimeout,
    RateLimited,
}

pub struct Manager<C: Connector> {
    connector: C,
    cfg: ManagerConfig,
    inbox: Inbox,
    state_tx: watch::Sender<ConnectionState>,
    backoff: Duration,
}

impl<C: Connector> Manager<C> {
    pub fn new(connector: C, cfg: ManagerConfig) -> (Self, PresenceHandle) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(ConnectionState::Idle);
        let manager = Self {
            connector,
            cfg,
            inbox: Inbox {
                rx,
                desired: Desired::default(),
            },
            state_tx,
            backoff: cfg.backoff_initial,
        };
        let handle = PresenceHandle {
            tx,
            state: state_rx,
        };
        (manager, handle)
    }

    fn publish(&self, state: ConnectionState) {
        self.state_tx.send_if_modified(|current| {
            if *current == state {
                false
            } else {
                *current = state;
                true
            }
        });
    }

    /// Runs until shutdown (or until every handle is dropped).
    pub async fn run(mut self) {
        loop {
            let Some(client_id) = self.inbox.desired.client_id.clone() else {
                self.publish(ConnectionState::Idle);
                if self.inbox.next_change().await.is_none() {
                    break;
                }
                continue;
            };

            match self.attempt(&client_id).await {
                Outcome::Shutdown => break,
                Outcome::Switched => {
                    self.backoff = self.cfg.backoff_initial;
                }
                Outcome::Rejected(message) => {
                    log::warn!("discord rejected application id {client_id}: {message}");
                    self.publish(ConnectionState::BadAppId { message });
                    loop {
                        match self.inbox.next_change().await {
                            None | Some(Change::Shutdown) => return,
                            Some(Change::Client) | Some(Change::Retry) => break,
                            Some(_) => {}
                        }
                    }
                    self.backoff = self.cfg.backoff_initial;
                }
                Outcome::Lost(why) => {
                    let wait = match why {
                        Lost::RateLimited => self.cfg.debounce.min_interval,
                        _ => self.backoff,
                    };
                    let retry_in_ms = wait.as_millis() as u64;
                    self.publish(match why {
                        Lost::NotRunning | Lost::Dropped => {
                            ConnectionState::DiscordNotRunning { retry_in_ms }
                        }
                        Lost::HandshakeTimeout => ConnectionState::Error {
                            message: "discord accepted the connection but never finished the handshake"
                                .into(),
                        },
                        Lost::RateLimited => ConnectionState::RateLimited { retry_in_ms },
                    });
                    if !matches!(why, Lost::RateLimited) {
                        self.backoff = (self.backoff * 2).min(self.cfg.backoff_max);
                    }
                    if !self.sleep_or_wake(wait).await {
                        break;
                    }
                }
            }
        }
        self.publish(ConnectionState::Idle);
    }

    /// Sleep, but wake early for a client change or a retry request. Returns
    /// false if we should shut down.
    async fn sleep_or_wake(&mut self, wait: Duration) -> bool {
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            let change = tokio::select! {
                _ = &mut sleep => return true,
                change = self.inbox.next_change() => change,
            };
            match change {
                None | Some(Change::Shutdown) => return false,
                Some(Change::Client) | Some(Change::Retry) => {
                    self.backoff = self.cfg.backoff_initial;
                    return true;
                }
                Some(_) => {}
            }
        }
    }

    async fn open_pipe(&self) -> Option<C::Stream> {
        for index in 0..PIPE_COUNT {
            match self.connector.connect(index).await {
                Ok(stream) => {
                    log::info!("connected to discord-ipc-{index}");
                    return Some(stream);
                }
                Err(e) => log::trace!("discord-ipc-{index}: {e}"),
            }
        }
        None
    }

    async fn attempt(&mut self, client_id: &str) -> Outcome {
        self.publish(ConnectionState::Connecting);

        let Some(stream) = self.open_pipe().await else {
            return Outcome::Lost(Lost::NotRunning);
        };
        let (read_half, mut writer) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);

        let handshake = match handshake_frame(client_id) {
            Ok(bytes) => bytes,
            Err(e) => return Outcome::Rejected(e.to_string()),
        };
        if writer.write_all(&handshake).await.is_err() || writer.flush().await.is_err() {
            return Outcome::Lost(Lost::Dropped);
        }

        let ready = match self.await_ready(&mut reader, &mut writer).await {
            Ok(info) => info,
            Err(outcome) => return outcome,
        };
        self.session(&mut reader, &mut writer, ready).await
    }

    async fn await_ready<R, W>(
        &mut self,
        reader: &mut FrameReader<R>,
        writer: &mut W,
    ) -> Result<ReadyInfo, Outcome>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        enum Ev {
            Cmd(Option<Change>),
            Frame(Result<Frame, ReadError>),
            Timeout,
        }
        let timeout = tokio::time::sleep(self.cfg.handshake_timeout);
        tokio::pin!(timeout);
        loop {
            let ev = tokio::select! {
                c = self.inbox.next_change() => Ev::Cmd(c),
                f = reader.read_frame() => Ev::Frame(f),
                _ = &mut timeout => Ev::Timeout,
            };
            match ev {
                Ev::Cmd(None) | Ev::Cmd(Some(Change::Shutdown)) => return Err(Outcome::Shutdown),
                Ev::Cmd(Some(Change::Client)) => return Err(Outcome::Switched),
                Ev::Cmd(Some(_)) => {}
                Ev::Timeout => return Err(Outcome::Lost(Lost::HandshakeTimeout)),
                Ev::Frame(Err(e)) => {
                    log::debug!("read failed during handshake: {e}");
                    return Err(Outcome::Lost(Lost::Dropped));
                }
                Ev::Frame(Ok(frame)) => match parse_incoming(&frame) {
                    Ok(Incoming::Ready(info)) => return Ok(info),
                    Ok(Incoming::Ping(payload)) => {
                        if send_pong(writer, &payload).await.is_err() {
                            return Err(Outcome::Lost(Lost::Dropped));
                        }
                    }
                    Ok(Incoming::Close(close)) => return Err(classify_close(&close)),
                    // while handshaking, discord only complains about the app id
                    Ok(Incoming::Error(RpcError { message, .. })) => {
                        return Err(Outcome::Rejected(message))
                    }
                    Ok(_) => {}
                    Err(e) => log::debug!("ignoring unparseable frame during handshake: {e}"),
                },
            }
        }
    }

    async fn session<R, W>(
        &mut self,
        reader: &mut FrameReader<R>,
        writer: &mut W,
        ready: ReadyInfo,
    ) -> Outcome
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        enum Ev {
            Cmd(Option<Change>),
            Frame(Result<Frame, ReadError>),
            Due,
        }

        self.backoff = self.cfg.backoff_initial;
        let username = ready.username;
        let mut deb: Debouncer<Option<Activity>> = Debouncer::new(self.cfg.debounce);
        if let Some(activity) = self.inbox.desired.activity.clone() {
            // fresh connection: send straight away, no quiet period
            deb.submit_now(Instant::now(), Some(activity));
        }
        let mut last_error: Option<String> = None;

        loop {
            let now = Instant::now();
            let deadline = deb.deadline();
            self.publish(if deb.is_throttled() {
                ConnectionState::RateLimited {
                    retry_in_ms: deadline
                        .map(|d| d.saturating_duration_since(now).as_millis() as u64)
                        .unwrap_or(0),
                }
            } else if let Some(message) = &last_error {
                ConnectionState::Error {
                    message: message.clone(),
                }
            } else {
                ConnectionState::Connected {
                    username: username.clone(),
                }
            });

            let wake_at = deadline.unwrap_or_else(|| now + Duration::from_secs(3600));
            let ev = tokio::select! {
                c = self.inbox.next_change() => Ev::Cmd(c),
                f = reader.read_frame() => Ev::Frame(f),
                _ = tokio::time::sleep_until(wake_at), if deadline.is_some() => Ev::Due,
            };

            match ev {
                Ev::Cmd(None) | Ev::Cmd(Some(Change::Shutdown)) => return Outcome::Shutdown,
                Ev::Cmd(Some(Change::Client)) => return Outcome::Switched,
                Ev::Cmd(Some(Change::Activity)) => {
                    deb.submit(Instant::now(), self.inbox.desired.activity.clone());
                }
                Ev::Cmd(Some(Change::Retry)) | Ev::Cmd(Some(Change::Noop)) => {}

                Ev::Frame(Err(e)) => {
                    log::info!("connection to discord ended: {e}");
                    return Outcome::Lost(Lost::Dropped);
                }
                Ev::Frame(Ok(frame)) => match parse_incoming(&frame) {
                    Ok(Incoming::Ping(payload)) => {
                        if send_pong(writer, &payload).await.is_err() {
                            return Outcome::Lost(Lost::Dropped);
                        }
                    }
                    Ok(Incoming::Close(close)) => return classify_close(&close),
                    Ok(Incoming::Error(err)) => {
                        log::warn!("discord error {}: {}", err.code, err.message);
                        if looks_rate_limited(&err) {
                            // our update was dropped: queue the wanted state again
                            // behind a full interval
                            let now = Instant::now();
                            deb.penalize(now);
                            deb.submit(now, self.inbox.desired.activity.clone());
                        } else {
                            last_error = Some(err.message);
                        }
                    }
                    Ok(Incoming::Response { .. }) => last_error = None,
                    Ok(_) => {}
                    Err(e) => log::debug!("ignoring unparseable frame: {e}"),
                },

                Ev::Due => {
                    if let Some(activity) = deb.take_due(Instant::now()) {
                        let payload =
                            set_activity_payload(self.cfg.pid, activity.as_ref(), &new_nonce());
                        if write_frame(writer, Opcode::Frame, &payload).await.is_err() {
                            return Outcome::Lost(Lost::Dropped);
                        }
                        log::debug!("sent activity");
                    }
                }
            }
        }
    }
}

async fn send_pong<W: AsyncWrite + Unpin>(writer: &mut W, ping_payload: &str) -> io::Result<()> {
    let bytes = pong_frame(ping_payload).map_err(io::Error::other)?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

fn classify_close(close: &CloseInfo) -> Outcome {
    match close.code {
        4000 => Outcome::Rejected(if close.message.is_empty() {
            "Invalid Client ID".into()
        } else {
            close.message.clone()
        }),
        4002 => Outcome::Lost(Lost::RateLimited),
        _ => Outcome::Lost(Lost::Dropped),
    }
}

/// Discord doesn't document a distinct code for "slow down" on a command
/// response, so also look at the text.
fn looks_rate_limited(err: &RpcError) -> bool {
    err.code == 4002 || err.message.to_ascii_lowercase().contains("rate limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{build_activity, ActivityInput};
    use crate::frame::{encode, FrameReader};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::DuplexStream;
    use tokio::time::{sleep, timeout};

    const GOOD_A: &str = "1402418491272986635";
    const GOOD_B: &str = "1445982610752012384";
    const BAD_ID: &str = "999999999999999999";
    const RATE_ID: &str = "888888888888888888";

    #[derive(Debug, Clone, PartialEq)]
    enum Seen {
        Handshake(String),
        SetActivity { client_id: String, activity: Value, at: Instant },
        Pong(String),
        Closed(String),
    }

    #[derive(Default)]
    struct World {
        /// which pipe index currently "exists"
        available: Mutex<Option<u8>>,
        attempts: Mutex<Vec<u8>>,
        seen: Mutex<Vec<Seen>>,
        reply_error: AtomicBool,
        ping_after_ready: AtomicBool,
        kill: watch::Sender<u64>,
    }

    impl World {
        fn new() -> Arc<Self> {
            let (kill, _) = watch::channel(0);
            Arc::new(Self {
                kill,
                ..Default::default()
            })
        }
        fn set_available(&self, index: Option<u8>) {
            *self.available.lock().unwrap() = index;
        }
        /// drop every server side connection, like discord quitting
        fn kill_connections(&self) {
            self.kill.send_modify(|g| *g += 1);
        }
        fn attempts(&self) -> Vec<u8> {
            self.attempts.lock().unwrap().clone()
        }
        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
        fn activities(&self) -> Vec<(String, Value, Instant)> {
            self.seen()
                .into_iter()
                .filter_map(|s| match s {
                    Seen::SetActivity { client_id, activity, at } => Some((client_id, activity, at)),
                    _ => None,
                })
                .collect()
        }
        fn handshakes(&self) -> Vec<String> {
            self.seen()
                .into_iter()
                .filter_map(|s| match s {
                    Seen::Handshake(id) => Some(id),
                    _ => None,
                })
                .collect()
        }
    }

    struct FakeConnector(Arc<World>);

    impl Connector for FakeConnector {
        type Stream = DuplexStream;

        async fn connect(&self, index: u8) -> io::Result<DuplexStream> {
            self.0.attempts.lock().unwrap().push(index);
            if *self.0.available.lock().unwrap() != Some(index) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such pipe"));
            }
            let (client, server) = tokio::io::duplex(64 * 1024);
            tokio::spawn(serve(self.0.clone(), server));
            Ok(client)
        }
    }

    async fn serve(world: Arc<World>, stream: DuplexStream) {
        let mut kill = world.kill.subscribe();
        let (read_half, mut w) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);

        let first = match reader.read_frame().await {
            Ok(f) => f,
            Err(_) => return,
        };
        assert_eq!(first.opcode, Opcode::Handshake);
        let hello: Value = serde_json::from_str(&first.payload).unwrap();
        assert_eq!(hello["v"], 1);
        let client_id = hello["client_id"].as_str().unwrap().to_owned();
        world.seen.lock().unwrap().push(Seen::Handshake(client_id.clone()));

        let send = |op, body: Value| encode(op, &body.to_string()).unwrap();
        match client_id.as_str() {
            BAD_ID => {
                let _ = w
                    .write_all(&send(Opcode::Close, json!({"code": 4000, "message": "Invalid Client ID"})))
                    .await;
                return;
            }
            RATE_ID => {
                let _ = w
                    .write_all(&send(Opcode::Close, json!({"code": 4002, "message": "Rate limited"})))
                    .await;
                return;
            }
            _ => {}
        }
        let ready = json!({"cmd":"DISPATCH","data":{"v":1,"config":{},"user":{"username":"tester"}},"evt":"READY","nonce":null});
        let _ = w.write_all(&send(Opcode::Frame, ready)).await;
        if world.ping_after_ready.load(Ordering::SeqCst) {
            let _ = w.write_all(&send(Opcode::Ping, json!({"nonce":"ping-1"}))).await;
        }

        loop {
            tokio::select! {
                _ = kill.changed() => return, // dropping both halves closes the pipe
                frame = reader.read_frame() => {
                    let Ok(frame) = frame else {
                        world.seen.lock().unwrap().push(Seen::Closed(client_id.clone()));
                        return;
                    };
                    match frame.opcode {
                        Opcode::Pong => world.seen.lock().unwrap().push(Seen::Pong(frame.payload)),
                        Opcode::Frame => {
                            let v: Value = serde_json::from_str(&frame.payload).unwrap();
                            assert_eq!(v["cmd"], "SET_ACTIVITY");
                            assert!(v["args"]["pid"].is_u64());
                            world.seen.lock().unwrap().push(Seen::SetActivity {
                                client_id: client_id.clone(),
                                activity: v["args"]["activity"].clone(),
                                at: Instant::now(),
                            });
                            let reply = if world.reply_error.load(Ordering::SeqCst) {
                                json!({"cmd":"SET_ACTIVITY","evt":"ERROR","data":{"code":4000,"message":"bad payload"},"nonce":v["nonce"]})
                            } else {
                                json!({"cmd":"SET_ACTIVITY","data":{},"evt":null,"nonce":v["nonce"]})
                            };
                            let _ = w.write_all(&send(Opcode::Frame, reply)).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn setup() -> (Arc<World>, PresenceHandle) {
        let world = World::new();
        let handle = spawn(FakeConnector(world.clone()), ManagerConfig::new(4321));
        (world, handle)
    }

    async fn wait_state(
        handle: &PresenceHandle,
        pred: impl Fn(&ConnectionState) -> bool,
    ) -> ConnectionState {
        let mut rx = handle.subscribe();
        let state = timeout(Duration::from_secs(300), rx.wait_for(|s| pred(s)))
            .await
            .expect("state never reached")
            .expect("manager stopped")
            .clone();
        state
    }

    fn activity(details: &str) -> Activity {
        build_activity(&ActivityInput {
            details: details.into(),
            ..Default::default()
        })
        .unwrap()
    }

    fn is_connected(s: &ConnectionState) -> bool {
        matches!(s, ConnectionState::Connected { .. })
    }

    async fn settle() {
        sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn starts_idle() {
        let (_world, handle) = setup();
        settle().await;
        assert_eq!(handle.state(), ConnectionState::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn rejects_a_malformed_client_id_up_front() {
        let (world, handle) = setup();
        assert!(matches!(
            handle.set("nope", None),
            Err(ProtocolError::BadClientId(_))
        ));
        settle().await;
        assert!(world.attempts().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn walks_pipes_zero_to_nine_in_order_and_uses_the_first_that_works() {
        let (world, handle) = setup();
        world.set_available(Some(3));
        handle.set(GOOD_A, Some(activity("hi"))).unwrap();
        let state = wait_state(&handle, is_connected).await;
        assert_eq!(state, ConnectionState::Connected { username: Some("tester".into()) });
        assert_eq!(world.attempts(), vec![0, 1, 2, 3]);
        assert_eq!(world.handshakes(), vec![GOOD_A.to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn sends_the_activity_once_ready() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("hello"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;
        let sent = world.activities();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, GOOD_A);
        assert_eq!(sent[0].1, json!({"details": "hello"}));
    }

    #[tokio::test(start_paused = true)]
    async fn reports_discord_not_running_and_backs_off() {
        let (world, handle) = setup();
        handle.set(GOOD_A, Some(activity("x"))).unwrap();
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { .. })).await;
        assert_eq!(state, ConnectionState::DiscordNotRunning { retry_in_ms: 1000 });
        assert_eq!(world.attempts().len(), 10, "one pass over pipes 0..9");

        sleep(Duration::from_millis(1100)).await; // first retry after 1s
        assert_eq!(world.attempts().len(), 20);
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { retry_in_ms } if *retry_in_ms == 2000)).await;
        assert_eq!(state, ConnectionState::DiscordNotRunning { retry_in_ms: 2000 });
        sleep(Duration::from_millis(2100)).await; // then after 2s
        assert_eq!(world.attempts().len(), 30);
    }

    #[tokio::test(start_paused = true)]
    async fn backoff_is_capped() {
        let (_world, handle) = setup();
        handle.set(GOOD_A, None).unwrap();
        // 1+2+4+8+16 = 31s of waiting, after which the wait stays at 30s
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { retry_in_ms } if *retry_in_ms == 30_000)).await;
        assert_eq!(state, ConnectionState::DiscordNotRunning { retry_in_ms: 30_000 });
        sleep(Duration::from_secs(31)).await;
        assert_eq!(
            handle.state(),
            ConnectionState::DiscordNotRunning { retry_in_ms: 30_000 }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn connects_when_discord_shows_up_later() {
        let (world, handle) = setup();
        handle.set(GOOD_A, Some(activity("later"))).unwrap();
        wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { .. })).await;
        world.set_available(Some(1));
        wait_state(&handle, is_connected).await;
        settle().await;
        assert_eq!(world.activities().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_now_skips_the_backoff() {
        let (world, handle) = setup();
        handle.set(GOOD_A, Some(activity("x"))).unwrap();
        // let the backoff grow
        sleep(Duration::from_secs(8)).await;
        world.set_available(Some(0));
        let before = Instant::now();
        handle.retry_now();
        wait_state(&handle, is_connected).await;
        assert!(before.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn reconnects_and_resends_after_discord_restarts() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("persist me"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;
        assert_eq!(world.activities().len(), 1);

        // discord quits
        world.set_available(None);
        world.kill_connections();
        wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { .. })).await;

        // ...and comes back later (possibly on another pipe)
        sleep(Duration::from_secs(5)).await;
        world.set_available(Some(2));
        wait_state(&handle, is_connected).await;
        settle().await;

        let sent = world.activities();
        assert_eq!(sent.len(), 2, "re-sent after reconnect");
        assert_eq!(sent[1].1, json!({"details": "persist me"}));
        assert_eq!(world.handshakes(), vec![GOOD_A.to_string(), GOOD_A.to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn bad_app_id_is_reported_and_not_retried() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(BAD_ID, Some(activity("x"))).unwrap();
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::BadAppId { .. })).await;
        assert_eq!(state, ConnectionState::BadAppId { message: "Invalid Client ID".into() });

        let attempts = world.attempts().len();
        sleep(Duration::from_secs(120)).await;
        assert_eq!(world.attempts().len(), attempts, "no reconnect loop");
        assert!(matches!(handle.state(), ConnectionState::BadAppId { .. }));

        // picking a working game recovers
        handle.set(GOOD_A, Some(activity("ok"))).unwrap();
        wait_state(&handle, is_connected).await;
    }

    #[tokio::test(start_paused = true)]
    async fn retry_now_retries_a_bad_app_id() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(BAD_ID, None).unwrap();
        wait_state(&handle, |s| matches!(s, ConnectionState::BadAppId { .. })).await;
        let before = world.handshakes().len();
        handle.retry_now();
        settle().await;
        assert_eq!(world.handshakes().len(), before + 1);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_close_during_handshake() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(RATE_ID, None).unwrap();
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::RateLimited { .. })).await;
        assert_eq!(state, ConnectionState::RateLimited { retry_in_ms: 15_000 });
    }

    #[tokio::test(start_paused = true)]
    async fn switching_games_opens_a_new_connection_with_the_new_id() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("a"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;

        handle.set(GOOD_B, Some(activity("b"))).unwrap();
        settle().await;
        wait_state(&handle, is_connected).await;
        settle().await;

        assert_eq!(world.handshakes(), vec![GOOD_A.to_string(), GOOD_B.to_string()]);
        let sent = world.activities();
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].0.as_str(), &sent[0].1), (GOOD_A, &json!({"details":"a"})));
        assert_eq!((sent[1].0.as_str(), &sent[1].1), (GOOD_B, &json!({"details":"b"})));
        // the first connection was closed, not left open
        assert!(world.seen().contains(&Seen::Closed(GOOD_A.to_string())));
        // switching sends immediately, it doesn't wait out the 15s window
        assert!(sent[1].2.duration_since(sent[0].2) < Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn rapid_edits_send_the_first_immediately_then_only_the_latest() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("v1"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;
        assert_eq!(world.activities().len(), 1);
        let first_at = world.activities()[0].2;

        for text in ["v2", "v3", "v4"] {
            handle.set(GOOD_A, Some(activity(text))).unwrap();
            sleep(Duration::from_millis(200)).await;
        }
        // waiting out the rate limit: nothing sent yet, ui is told
        sleep(Duration::from_secs(1)).await;
        assert_eq!(world.activities().len(), 1);
        match handle.state() {
            ConnectionState::RateLimited { retry_in_ms } => {
                assert!((12_000..=15_000).contains(&retry_in_ms), "{retry_in_ms}");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }

        sleep(Duration::from_secs(15)).await;
        let sent = world.activities();
        assert_eq!(sent.len(), 2, "v2 and v3 were coalesced away");
        assert_eq!(sent[1].1, json!({"details": "v4"}));
        assert!(sent[1].2.duration_since(first_at) >= Duration::from_secs(15));
        assert!(matches!(handle.state(), ConnectionState::Connected { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn identical_updates_are_not_resent() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("same"))).unwrap();
        wait_state(&handle, is_connected).await;
        handle.set(GOOD_A, Some(activity("same"))).unwrap();
        sleep(Duration::from_secs(40)).await;
        assert_eq!(world.activities().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn clear_sends_a_null_activity_and_stays_connected() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("temp"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;

        handle.clear();
        sleep(Duration::from_secs(20)).await;
        let sent = world.activities();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].1, Value::Null);
        assert!(matches!(handle.state(), ConnectionState::Connected { .. }));
        assert_eq!(world.handshakes().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn clear_cancels_a_pending_update() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("one"))).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;
        handle.set(GOOD_A, Some(activity("two"))).unwrap();
        sleep(Duration::from_millis(100)).await;
        handle.clear();
        sleep(Duration::from_secs(20)).await;
        let sent = world.activities();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].1, Value::Null, "the pending 'two' never went out");
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_closes_the_connection_and_goes_idle() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        handle.set(GOOD_A, Some(activity("x"))).unwrap();
        wait_state(&handle, is_connected).await;
        handle.disconnect();
        wait_state(&handle, |s| *s == ConnectionState::Idle).await;
        settle().await;
        assert!(world.seen().contains(&Seen::Closed(GOOD_A.to_string())));
        // and it stays down
        let attempts = world.attempts().len();
        sleep(Duration::from_secs(60)).await;
        assert_eq!(world.attempts().len(), attempts);
    }

    #[tokio::test(start_paused = true)]
    async fn answers_pings_with_the_same_payload() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        world.ping_after_ready.store(true, Ordering::SeqCst);
        handle.set(GOOD_A, None).unwrap();
        wait_state(&handle, is_connected).await;
        settle().await;
        assert!(world
            .seen()
            .contains(&Seen::Pong(r#"{"nonce":"ping-1"}"#.to_string())));
    }

    #[tokio::test(start_paused = true)]
    async fn discord_error_replies_surface_as_an_error_state_then_recover() {
        let (world, handle) = setup();
        world.set_available(Some(0));
        world.reply_error.store(true, Ordering::SeqCst);
        handle.set(GOOD_A, Some(activity("a"))).unwrap();
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::Error { .. })).await;
        assert_eq!(state, ConnectionState::Error { message: "bad payload".into() });
        assert_eq!(world.handshakes().len(), 1, "an error reply doesn't drop the connection");

        world.reply_error.store(false, Ordering::SeqCst);
        handle.set(GOOD_A, Some(activity("b"))).unwrap();
        wait_state(&handle, is_connected).await;
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_timeout_is_an_error_state() {
        // a "discord" that accepts the connection and never answers
        struct Mute;
        impl Connector for Mute {
            type Stream = DuplexStream;
            async fn connect(&self, index: u8) -> io::Result<DuplexStream> {
                if index != 0 {
                    return Err(io::ErrorKind::NotFound.into());
                }
                let (client, server) = tokio::io::duplex(1024);
                std::mem::forget(server); // keep it open, say nothing
                Ok(client)
            }
        }
        let handle = spawn(Mute, ManagerConfig::new(1));
        handle.set(GOOD_A, None).unwrap();
        let state = wait_state(&handle, |s| matches!(s, ConnectionState::Error { .. })).await;
        assert!(matches!(state, ConnectionState::Error { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn switching_while_discord_is_down_uses_the_new_id_when_it_returns() {
        let (world, handle) = setup();
        handle.set(GOOD_A, Some(activity("a"))).unwrap();
        wait_state(&handle, |s| matches!(s, ConnectionState::DiscordNotRunning { .. })).await;
        handle.set(GOOD_B, Some(activity("b"))).unwrap();
        settle().await;
        world.set_available(Some(0));
        handle.retry_now();
        wait_state(&handle, is_connected).await;
        assert_eq!(world.handshakes(), vec![GOOD_B.to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_stops_the_task() {
        let world = World::new();
        let (manager, handle) = Manager::new(FakeConnector(world.clone()), ManagerConfig::new(1));
        let task = tokio::spawn(manager.run());
        world.set_available(Some(0));
        handle.set(GOOD_A, None).unwrap();
        wait_state(&handle, is_connected).await;
        handle.shutdown();
        timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert_eq!(handle.state(), ConnectionState::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_every_handle_stops_the_task() {
        let world = World::new();
        let (manager, handle) = Manager::new(FakeConnector(world), ManagerConfig::new(1));
        let task = tokio::spawn(manager.run());
        drop(handle);
        timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    }

    #[test]
    fn connection_state_json_for_the_ui() {
        assert_eq!(
            serde_json::to_value(ConnectionState::Idle).unwrap(),
            json!({"kind": "idle"})
        );
        assert_eq!(
            serde_json::to_value(ConnectionState::RateLimited { retry_in_ms: 5000 }).unwrap(),
            json!({"kind": "rateLimited", "retryInMs": 5000})
        );
        assert_eq!(
            serde_json::to_value(ConnectionState::DiscordNotRunning { retry_in_ms: 1000 }).unwrap(),
            json!({"kind": "discordNotRunning", "retryInMs": 1000})
        );
        assert_eq!(
            serde_json::to_value(ConnectionState::Connected { username: Some("x".into()) }).unwrap(),
            json!({"kind": "connected", "username": "x"})
        );
        assert_eq!(
            serde_json::to_value(ConnectionState::BadAppId { message: "m".into() }).unwrap(),
            json!({"kind": "badAppId", "message": "m"})
        );
    }
}
