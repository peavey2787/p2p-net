//! Background credential refresh for one provider: fetches run as detached
//! tasks and the runtime collects their results on its own schedule, so a
//! slow provider never blocks the swarm loop.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use web_time::Instant;

use super::credentials::TurnCredentials;
use super::provider::TurnProviderConfig;

const FIRST_RETRY: Duration = Duration::from_secs(30);
const MAX_RETRY: Duration = Duration::from_secs(600);

type FetchSlot = Arc<Mutex<Option<Result<TurnCredentials, String>>>>;

#[derive(Debug)]
pub(crate) struct ProviderRefresher {
    provider: TurnProviderConfig,
    ttl_secs: u64,
    slot: FetchSlot,
    in_flight: bool,
    next_due: Instant,
    failures: u32,
}

impl ProviderRefresher {
    pub(crate) fn new(provider: TurnProviderConfig, ttl_secs: u64) -> Self {
        Self {
            provider,
            ttl_secs,
            slot: FetchSlot::default(),
            in_flight: false,
            next_due: Instant::now(),
            failures: 0,
        }
    }

    pub(crate) fn provider_name(&self) -> &'static str {
        self.provider.name()
    }

    /// Start a fetch when one is due and none is running.
    pub(crate) fn start_if_due(&mut self, now: Instant) -> bool {
        if self.in_flight || now < self.next_due {
            return false;
        }
        self.in_flight = true;
        let provider = self.provider.clone();
        let ttl_secs = self.ttl_secs;
        let slot = Arc::clone(&self.slot);
        let fetch = async move {
            let result = provider.issue(ttl_secs).await;
            *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        };
        drop(crate::runtime::spawn(fetch));
        true
    }

    /// Collect a finished fetch and schedule the next one: at three quarters
    /// of the credential lifetime on success, with exponential backoff on failure.
    pub(crate) fn take_result(&mut self, now: Instant) -> Option<Result<TurnCredentials, String>> {
        let result = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()?;
        self.in_flight = false;
        self.next_due = now + self.delay_after(&result);
        Some(result)
    }

    fn delay_after(&mut self, result: &Result<TurnCredentials, String>) -> Duration {
        match result {
            Ok(credentials) => {
                self.failures = 0;
                Duration::from_secs(credentials.ttl_secs()) * 3 / 4
            }
            Err(_) => {
                let delay = FIRST_RETRY.saturating_mul(1 << self.failures.min(5));
                self.failures = self.failures.saturating_add(1);
                delay.min(MAX_RETRY)
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::connectivity::turn::{SecretRef, SharedSecretProviderConfig};

    fn refresher(secret: &str) -> ProviderRefresher {
        ProviderRefresher::new(
            TurnProviderConfig::SharedSecret(SharedSecretProviderConfig {
                urls: vec!["turn:turn.example.org:3478".to_string()],
                secret: SecretRef::Value(secret.to_string()),
            }),
            600,
        )
    }

    async fn wait_for_result(refresher: &mut ProviderRefresher) -> Result<TurnCredentials, String> {
        for _ in 0..200 {
            if let Some(result) = refresher.take_result(Instant::now()) {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("refresh did not finish");
    }

    #[tokio::test]
    async fn successful_fetch_is_collected_and_rescheduled() {
        let mut refresher = refresher("secret");
        let now = Instant::now();
        assert!(refresher.start_if_due(now));
        assert!(!refresher.start_if_due(now), "one fetch at a time");
        let credentials = wait_for_result(&mut refresher).await.expect("minted");
        assert_eq!(credentials.ttl, Some(600));
        assert!(!refresher.start_if_due(Instant::now() + Duration::from_secs(400)));
        assert!(refresher.start_if_due(Instant::now() + Duration::from_secs(451)));
    }

    #[tokio::test]
    async fn failures_back_off_exponentially() {
        let mut refresher = ProviderRefresher::new(
            TurnProviderConfig::SharedSecret(SharedSecretProviderConfig {
                urls: vec!["turn:turn.example.org:3478".to_string()],
                secret: SecretRef::Env("P2P_NET_TEST_SECRET_THAT_IS_NEVER_SET".to_string()),
            }),
            600,
        );
        assert!(refresher.start_if_due(Instant::now()));
        assert!(wait_for_result(&mut refresher).await.is_err());
        assert!(!refresher.start_if_due(Instant::now() + Duration::from_secs(29)));
        assert!(refresher.start_if_due(Instant::now() + FIRST_RETRY));
        assert!(wait_for_result(&mut refresher).await.is_err());
        assert!(!refresher.start_if_due(Instant::now() + FIRST_RETRY));
        assert!(refresher.start_if_due(Instant::now() + FIRST_RETRY * 2));
    }
}
