#[derive(Debug, Clone, Copy)]
pub struct RealtimePollPolicy {
    pub foreground_recent_secs: u64,
    pub foreground_idle_secs: u64,
    pub background_secs: u64,
    pub max_backoff_secs: u64,
}

impl Default for RealtimePollPolicy {
    fn default() -> Self {
        Self {
            foreground_recent_secs: 10,
            foreground_idle_secs: 30,
            background_secs: 120,
            max_backoff_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RealtimeContext {
    pub app_foreground: bool,
    pub recent_activity: bool,
    pub consecutive_failures: u32,
}

impl RealtimePollPolicy {
    pub fn from_foreground_interval_secs(foreground_recent_secs: u64) -> Self {
        Self {
            foreground_recent_secs: foreground_recent_secs.max(1),
            ..Self::default()
        }
    }

    pub fn next_delay(&self, ctx: RealtimeContext) -> std::time::Duration {
        if ctx.consecutive_failures > 0 {
            let delay = self
                .foreground_recent_secs
                .saturating_mul(2_u64.saturating_pow(ctx.consecutive_failures));
            return std::time::Duration::from_secs(delay.min(self.max_backoff_secs));
        }

        if ctx.app_foreground && ctx.recent_activity {
            return std::time::Duration::from_secs(self.foreground_recent_secs);
        }
        if ctx.app_foreground {
            return std::time::Duration::from_secs(self.foreground_idle_secs);
        }
        std::time::Duration::from_secs(self.background_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_recent_activity_uses_low_latency_polling() {
        let policy = RealtimePollPolicy::default();
        assert_eq!(
            policy.next_delay(RealtimeContext {
                app_foreground: true,
                recent_activity: true,
                consecutive_failures: 0,
            }),
            std::time::Duration::from_secs(10)
        );
    }

    #[test]
    fn background_stable_mode_uses_slower_polling() {
        let policy = RealtimePollPolicy::default();
        assert_eq!(
            policy.next_delay(RealtimeContext {
                app_foreground: false,
                recent_activity: false,
                consecutive_failures: 0,
            }),
            std::time::Duration::from_secs(120)
        );
    }

    #[test]
    fn failures_back_off_polling() {
        let policy = RealtimePollPolicy::default();
        assert_eq!(
            policy.next_delay(RealtimeContext {
                app_foreground: true,
                recent_activity: false,
                consecutive_failures: 3,
            }),
            std::time::Duration::from_secs(80)
        );
    }

    #[test]
    fn configured_foreground_interval_supports_battery_saver() {
        let policy = RealtimePollPolicy::from_foreground_interval_secs(120);

        assert_eq!(
            policy.next_delay(RealtimeContext {
                app_foreground: true,
                recent_activity: true,
                consecutive_failures: 0,
            }),
            std::time::Duration::from_secs(120)
        );
    }
}
