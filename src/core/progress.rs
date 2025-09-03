use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tower_lsp::Client;

/// Simple delayed message reporter using lsp_info macro
pub struct DelayedInfoReporter {
    started: Arc<Mutex<bool>>,
    start_time: Instant,
    delay_threshold: Duration,
}

impl DelayedInfoReporter {
    pub fn new(_client: Arc<Client>) -> Self {
        Self::with_threshold(_client, Duration::from_millis(2000))
    }

    pub fn with_threshold(_client: Arc<Client>, delay_threshold: Duration) -> Self {
        Self {
            started: Arc::new(Mutex::new(false)),
            start_time: Instant::now(),
            delay_threshold,
        }
    }

    pub fn delay_threshold(&self) -> Duration {
        self.delay_threshold
    }

    /// Check if we should send info message and do so if needed
    pub async fn maybe_send_info(&self, message: &str) {
        if self.start_time.elapsed() >= self.delay_threshold {
            let mut started = self.started.lock().await;
            if !*started {
                crate::lsp_info!("{}", message);
                *started = true;
            }
        }
    }
}

