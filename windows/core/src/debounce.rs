//! Coalescing + rate limiting of presence updates.
//!
//! Two rules, both measured on tokio's clock so tests can run with paused time:
//!
//! 1. *quiet period*: an edit isn't sent until it has been left alone for
//!    `quiet`, so typing a sentence is one update, not forty.
//! 2. *minimum interval*: two sends are never closer than `min_interval`
//!    (discord drops updates faster than about one per 15 seconds).
//!
//! Only the most recent value is kept; a newer value replaces a pending one.
//! This type is a plain state machine, it never sleeps by itself: ask it for a
//! [`deadline`](Debouncer::deadline), wait until then, and call
//! [`take_due`](Debouncer::take_due).

use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebounceConfig {
    pub quiet: Duration,
    pub min_interval: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(750),
            min_interval: Duration::from_secs(15),
        }
    }
}

#[derive(Debug)]
pub struct Debouncer<T> {
    cfg: DebounceConfig,
    /// the newest value and the earliest moment the quiet rule allows sending it
    pending: Option<(T, Instant)>,
    last_sent: Option<Instant>,
}

impl<T> Debouncer<T> {
    pub fn new(cfg: DebounceConfig) -> Self {
        Self {
            cfg,
            pending: None,
            last_sent: None,
        }
    }

    pub fn config(&self) -> DebounceConfig {
        self.cfg
    }

    /// Queue `value`, replacing whatever was pending. It will be sent after the
    /// quiet period (and not before the minimum interval has passed).
    pub fn submit(&mut self, now: Instant, value: T) {
        self.pending = Some((value, now + self.cfg.quiet));
    }

    /// Like [`submit`](Self::submit) but skips the quiet period. The minimum
    /// interval still applies. Used for the first update on a fresh connection.
    pub fn submit_now(&mut self, now: Instant, value: T) {
        self.pending = Some((value, now));
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// When the pending value may be sent, or `None` if nothing is pending.
    pub fn deadline(&self) -> Option<Instant> {
        let (_, ready_at) = self.pending.as_ref()?;
        Some(match self.last_sent {
            Some(sent) => (*ready_at).max(sent + self.cfg.min_interval),
            None => *ready_at,
        })
    }

    /// True when the pending value is only waiting because of the minimum
    /// interval (rate limiting), as opposed to the short quiet period.
    pub fn is_throttled(&self) -> bool {
        match (&self.pending, self.last_sent) {
            (Some((_, ready_at)), Some(sent)) => sent + self.cfg.min_interval > *ready_at,
            _ => false,
        }
    }

    /// If the pending value is due at `now`, take it and record the send.
    pub fn take_due(&mut self, now: Instant) -> Option<T> {
        if self.deadline()? <= now {
            self.last_sent = Some(now);
            self.pending.take().map(|(value, _)| value)
        } else {
            None
        }
    }

    /// Discord told us to slow down: pretend we just sent, so the next update
    /// waits a full interval.
    pub fn penalize(&mut self, now: Instant) {
        self.last_sent = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{advance, sleep};

    const QUIET: Duration = Duration::from_millis(750);
    const INTERVAL: Duration = Duration::from_secs(15);

    fn deb() -> Debouncer<&'static str> {
        Debouncer::new(DebounceConfig::default())
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_pending_means_no_deadline() {
        let mut d = deb();
        assert_eq!(d.deadline(), None);
        assert_eq!(d.take_due(Instant::now()), None);
        assert!(!d.is_throttled());
    }

    #[tokio::test(start_paused = true)]
    async fn first_update_waits_only_for_the_quiet_period() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit(t0, "a");
        assert_eq!(d.deadline(), Some(t0 + QUIET));
        assert_eq!(d.take_due(t0), None);
        advance(QUIET).await;
        assert_eq!(d.take_due(Instant::now()), Some("a"));
        assert!(!d.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn rapid_edits_collapse_into_the_latest() {
        let mut d = deb();
        for text in ["h", "he", "hel", "hell", "hello"] {
            d.submit(Instant::now(), text);
            advance(Duration::from_millis(100)).await;
        }
        // 100ms after the last edit: still inside the quiet period
        assert_eq!(d.take_due(Instant::now()), None);
        advance(QUIET).await;
        assert_eq!(d.take_due(Instant::now()), Some("hello"));
        assert_eq!(d.take_due(Instant::now()), None, "only one send");
    }

    #[tokio::test(start_paused = true)]
    async fn each_edit_restarts_the_quiet_period() {
        let mut d = deb();
        d.submit(Instant::now(), "a");
        advance(Duration::from_millis(700)).await;
        d.submit(Instant::now(), "b");
        advance(Duration::from_millis(700)).await;
        assert_eq!(d.take_due(Instant::now()), None);
        advance(Duration::from_millis(50)).await;
        assert_eq!(d.take_due(Instant::now()), Some("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn second_send_waits_for_the_minimum_interval() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit_now(t0, "first");
        assert_eq!(d.take_due(t0), Some("first"));

        d.submit(t0, "second");
        assert_eq!(d.deadline(), Some(t0 + INTERVAL));
        assert!(d.is_throttled());

        advance(INTERVAL - Duration::from_millis(1)).await;
        assert_eq!(d.take_due(Instant::now()), None);
        advance(Duration::from_millis(1)).await;
        assert_eq!(d.take_due(Instant::now()), Some("second"));
    }

    #[tokio::test(start_paused = true)]
    async fn edits_during_the_wait_replace_the_pending_value() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit_now(t0, "first");
        d.take_due(t0);

        for text in ["x", "y", "latest"] {
            advance(Duration::from_secs(3)).await;
            d.submit(Instant::now(), text);
        }
        // 9s since the first send, still limited
        assert_eq!(d.take_due(Instant::now()), None);
        advance(Duration::from_secs(6)).await;
        assert_eq!(d.take_due(Instant::now()), Some("latest"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_old_send_does_not_delay_a_new_edit_beyond_the_quiet_period() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit_now(t0, "first");
        d.take_due(t0);
        sleep(Duration::from_secs(60)).await;

        let t1 = Instant::now();
        d.submit(t1, "later");
        assert_eq!(d.deadline(), Some(t1 + QUIET));
        assert!(!d.is_throttled());
    }

    #[tokio::test(start_paused = true)]
    async fn throttled_only_reports_interval_waits() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit_now(t0, "a");
        d.take_due(t0);
        advance(Duration::from_secs(14)).await;
        // sent 14s ago: the quiet period (0.75s) is shorter than the 1s left
        d.submit(Instant::now(), "b");
        assert!(d.is_throttled());
        advance(Duration::from_millis(1000)).await;
        assert_eq!(d.take_due(Instant::now()), Some("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn submit_now_skips_quiet_but_not_the_interval() {
        let mut d = deb();
        let t0 = Instant::now();
        d.submit_now(t0, "a");
        assert_eq!(d.take_due(t0), Some("a"));
        d.submit_now(t0, "b");
        assert_eq!(d.deadline(), Some(t0 + INTERVAL));
    }

    #[tokio::test(start_paused = true)]
    async fn penalize_pushes_the_next_send_out_a_full_interval() {
        let mut d = deb();
        let t0 = Instant::now();
        d.penalize(t0);
        d.submit(t0, "a");
        assert_eq!(d.deadline(), Some(t0 + INTERVAL));
    }

    #[tokio::test(start_paused = true)]
    async fn works_with_a_real_timer() {
        let mut d = deb();
        d.submit_now(Instant::now(), "a");
        assert_eq!(d.take_due(Instant::now()), Some("a"));
        d.submit(Instant::now(), "b");
        let started = Instant::now();
        let deadline = d.deadline().unwrap();
        tokio::time::sleep_until(deadline).await;
        assert_eq!(d.take_due(Instant::now()), Some("b"));
        assert_eq!(started.elapsed(), INTERVAL);
    }
}
