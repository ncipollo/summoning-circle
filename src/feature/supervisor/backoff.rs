use std::time::Duration;

use super::policy::Policy;

/// Doubling relaunch delay that resets once a process has stayed up long enough.
pub struct Backoff {
    current: Duration,
    initial: Duration,
    max: Duration,
    reset_after: Duration,
}

impl Backoff {
    pub fn new(policy: &Policy) -> Self {
        Self {
            current: policy.initial_backoff,
            initial: policy.initial_backoff,
            max: policy.max_backoff,
            reset_after: policy.uptime_reset,
        }
    }

    /// Delay before the next relaunch, given how long the process just stayed up.
    pub fn after_exit(&mut self, uptime: Duration) -> Duration {
        if uptime >= self.reset_after {
            self.current = self.initial;
        }

        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        delay
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Backoff, Policy};

    fn policy() -> Policy {
        Policy {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            uptime_reset: Duration::from_secs(60),
            ..Policy::default()
        }
    }

    #[test]
    fn doubles_and_caps_on_repeated_failures() {
        let mut backoff = Backoff::new(&policy());
        let short_uptime = Duration::from_secs(0);

        let delays: Vec<_> = (0..7)
            .map(|_| backoff.after_exit(short_uptime))
            .map(|delay| delay.as_secs())
            .collect();

        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn resets_after_long_uptime() {
        let mut backoff = Backoff::new(&policy());
        backoff.after_exit(Duration::from_secs(0));
        backoff.after_exit(Duration::from_secs(0));

        let delay = backoff.after_exit(Duration::from_secs(120));

        assert_eq!(delay.as_secs(), 1);
    }
}
