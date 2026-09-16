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
    /// How often a daemon-kind process's `status` command is polled once it's running.
    pub status_poll: Duration,
    /// How long a freshly started daemon has to report itself alive via `status` before it's
    /// treated as a failed launch. Bounds a slow or misconfigured `status` command so it can't
    /// make every launch look like instant death.
    pub daemon_start_grace: Duration,
    /// How long a single daemon `stop`/`status` command is given to finish before it's
    /// treated as a timeout.
    pub daemon_command_timeout: Duration,
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
            status_poll: Duration::from_secs(2),
            daemon_start_grace: Duration::from_secs(10),
            daemon_command_timeout: Duration::from_secs(10),
        }
    }
}
