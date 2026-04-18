use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Startup,
    Manual,
    Timer,
    NetworkOnline,
    WindowFocus,
    WindowBlur,
    ProviderPush,
}

impl SyncTrigger {
    pub fn from_reason(reason: &str) -> Self {
        match reason {
            "network_online" => Self::NetworkOnline,
            "window_focus" => Self::WindowFocus,
            "window_blur" => Self::WindowBlur,
            "provider_push" => Self::ProviderPush,
            "startup" => Self::Startup,
            "timer" => Self::Timer,
            _ => Self::Manual,
        }
    }

    pub fn should_sync_now(self) -> bool {
        matches!(
            self,
            Self::Manual | Self::NetworkOnline | Self::WindowFocus | Self::ProviderPush
        )
    }
}

#[derive(Debug, Clone)]
pub struct RealtimeRuntimeState {
    app_foreground: bool,
    recent_activity_window: Duration,
    last_activity_at: Option<Instant>,
}

impl RealtimeRuntimeState {
    pub fn new(recent_activity_window: Duration, now: Instant) -> Self {
        Self {
            app_foreground: true,
            recent_activity_window,
            last_activity_at: Some(now),
        }
    }

    pub fn record_trigger(&mut self, trigger: SyncTrigger, now: Instant) {
        match trigger {
            SyncTrigger::WindowBlur => {
                self.app_foreground = false;
                self.last_activity_at = None;
            }
            SyncTrigger::WindowFocus => {
                self.app_foreground = true;
                self.last_activity_at = Some(now);
            }
            SyncTrigger::Manual
            | SyncTrigger::NetworkOnline
            | SyncTrigger::ProviderPush
            | SyncTrigger::Startup => {
                self.last_activity_at = Some(now);
            }
            SyncTrigger::Timer => {}
        }
    }

    pub fn context(&self, consecutive_failures: u32, now: Instant) -> RealtimeContext {
        RealtimeContext {
            app_foreground: self.app_foreground,
            recent_activity: self.last_activity_at.is_some_and(|last_activity| {
                now.duration_since(last_activity) <= self.recent_activity_window
            }),
            consecutive_failures,
        }
    }
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
            return Duration::from_secs(delay.min(self.max_backoff_secs));
        }

        if ctx.app_foreground && ctx.recent_activity {
            return Duration::from_secs(self.foreground_recent_secs);
        }
        if ctx.app_foreground {
            return Duration::from_secs(self.foreground_idle_secs);
        }
        Duration::from_secs(self.background_secs)
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

    #[test]
    fn runtime_context_tracks_focus_blur_and_recent_activity() {
        let started = std::time::Instant::now();
        let mut runtime = RealtimeRuntimeState::new(std::time::Duration::from_secs(60), started);

        runtime.record_trigger(SyncTrigger::WindowBlur, started);
        let background = runtime.context(0, started + std::time::Duration::from_secs(1));
        assert!(!background.app_foreground);
        assert!(!background.recent_activity);

        runtime.record_trigger(SyncTrigger::NetworkOnline, started + std::time::Duration::from_secs(1));
        let background_recent = runtime.context(0, started + std::time::Duration::from_secs(2));
        assert!(!background_recent.app_foreground);
        assert!(background_recent.recent_activity);

        runtime.record_trigger(SyncTrigger::WindowFocus, started + std::time::Duration::from_secs(2));
        let recent = runtime.context(0, started + std::time::Duration::from_secs(30));
        assert!(recent.app_foreground);
        assert!(recent.recent_activity);

        let idle = runtime.context(0, started + std::time::Duration::from_secs(90));
        assert!(idle.app_foreground);
        assert!(!idle.recent_activity);
    }

    #[test]
    fn sync_trigger_from_reason_supports_window_blur() {
        assert_eq!(SyncTrigger::from_reason("window_blur"), SyncTrigger::WindowBlur);
        assert!(!SyncTrigger::WindowBlur.should_sync_now());
        assert!(SyncTrigger::WindowFocus.should_sync_now());
    }
}
