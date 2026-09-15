use std::time::Duration;

/// Tunable timing constants for the supervisor's backoff and shutdown behaviour.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub uptime_reset: Duration,
    pub shutdown_grace: Duration,
    /// How long to wait for a burst of config file events to go quiet before reloading.
    pub config_debounce: Duration,
    /// How often a worker re-checks the paused flag while it is paused. Only spent while
    /// paused: an unpaused worker never polls.
    pub pause_poll: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            uptime_reset: Duration::from_secs(60),
            shutdown_grace: Duration::from_secs(5),
            config_debounce: Duration::from_millis(300),
            pause_poll: Duration::from_secs(2),
        }
    }
}
