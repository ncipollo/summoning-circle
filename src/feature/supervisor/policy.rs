use std::time::Duration;

/// Tunable timing constants for the supervisor's backoff and shutdown behaviour.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub uptime_reset: Duration,
    pub shutdown_grace: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            uptime_reset: Duration::from_secs(60),
            shutdown_grace: Duration::from_secs(5),
        }
    }
}
