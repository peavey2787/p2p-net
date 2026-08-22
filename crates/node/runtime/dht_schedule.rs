use std::time::Duration;

use tokio::time::Instant as TokioInstant;

const DHT_STARTUP_BACKOFF_SECS: [u64; 4] = [5, 15, 30, 60];
const DHT_EVENT_REFRESH_MIN_GAP_SECS: u64 = 5;

#[derive(Debug)]
pub(super) struct DhtRefreshSchedule {
    startup_index: usize,
    steady_interval: Duration,
    last_refresh: TokioInstant,
    next_due: TokioInstant,
    event_refresh_pending: bool,
}

impl DhtRefreshSchedule {
    pub(super) fn new(steady_interval_secs: u64) -> Self {
        let now = TokioInstant::now();
        let first_delay = Duration::from_secs(DHT_STARTUP_BACKOFF_SECS[0]);
        Self {
            startup_index: 0,
            steady_interval: Duration::from_secs(steady_interval_secs.max(1)),
            last_refresh: now,
            next_due: now + first_delay,
            event_refresh_pending: false,
        }
    }

    pub(super) fn next_due(&self) -> TokioInstant {
        self.next_due
    }

    pub(super) fn current_interval_secs(&self) -> u64 {
        if self.event_refresh_pending {
            return DHT_EVENT_REFRESH_MIN_GAP_SECS;
        }
        if self.startup_index < DHT_STARTUP_BACKOFF_SECS.len() {
            DHT_STARTUP_BACKOFF_SECS[self.startup_index]
        } else {
            self.steady_interval.as_secs().max(1)
        }
    }

    pub(super) fn record_refresh(&mut self) {
        let now = TokioInstant::now();
        self.last_refresh = now;
        self.event_refresh_pending = false;
        if self.startup_index < DHT_STARTUP_BACKOFF_SECS.len() {
            self.startup_index += 1;
        }
        let next_delay = if self.startup_index < DHT_STARTUP_BACKOFF_SECS.len() {
            Duration::from_secs(DHT_STARTUP_BACKOFF_SECS[self.startup_index])
        } else {
            self.steady_interval
        };
        self.next_due = now + next_delay;
    }

    /// Accelerate discovery only when the node regains connectivity after
    /// having no connected peers. This avoids a DHT query -> connection ->
    /// refresh feedback loop while retaining fast recovery after isolation.
    pub(super) fn request_connectivity_recovery_refresh(&mut self) -> bool {
        self.request_event_refresh()
    }

    pub(super) fn request_event_refresh(&mut self) -> bool {
        let now = TokioInstant::now();
        let earliest = self.last_refresh + Duration::from_secs(DHT_EVENT_REFRESH_MIN_GAP_SECS);
        let requested = now.max(earliest);
        if requested < self.next_due {
            self.next_due = requested;
            self.event_refresh_pending = true;
            true
        } else {
            false
        }
    }
}
