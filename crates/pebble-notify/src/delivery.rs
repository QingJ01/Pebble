//! When the agent's own messages may interrupt with a desktop notification
//! (#107). Findings are always recorded in the conversation; this decides
//! which of the pending ones become an OS notification now, which wait,
//! and which stay in the app only.
//!
//! Rules, in order:
//! 1. Notifications off, or Pebble in front → in the app only (the badge
//!    and the conversation are enough).
//! 2. Older than [`DeliveryLimits::stale_after`] → in the app only (a late
//!    "waiting for your reply" is noise; the briefing mentions it).
//! 3. Quiet hours → wait until they end.
//! 4. Daily cap reached → in the app only.
//! 5. Too soon after the last push → wait (urgent ones excepted).
//! 6. Otherwise push, coalescing everything pending into one notification;
//!    a briefing is always its own notification.

use crate::QuietHours;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// Waiting-for-reply nudges, overdue follow-ups.
    Normal,
    /// Ignores the minimum gap (still respects quiet hours and the cap).
    Urgent,
    /// The daily briefing.
    Briefing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingPush {
    pub id: String,
    pub kind: PushKind,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct DeliveryLimits {
    pub max_per_day: u32,
    pub min_gap_seconds: i64,
    pub stale_after: i64,
}

impl Default for DeliveryLimits {
    fn default() -> Self {
        Self {
            max_per_day: 6,
            min_gap_seconds: 45 * 60,
            stale_after: 12 * 60 * 60,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeliveryContext {
    pub now: i64,
    pub local_minute_of_day: u16,
    pub notifications_enabled: bool,
    pub window_focused: bool,
    pub quiet_hours: Option<QuietHours>,
    /// OS notifications already sent today (local day).
    pub pushes_today: u32,
    pub last_push_at: Option<i64>,
}

/// What to do with the pending pushes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeliveryPlan {
    /// Each inner list becomes one OS notification (a summary if > 1).
    pub notify: Vec<Vec<String>>,
    /// Shown in the app only; never pushed.
    pub in_app: Vec<String>,
    /// Kept pending for a later tick.
    pub hold: Vec<String>,
}

pub fn plan_delivery(
    pending: &[PendingPush],
    ctx: &DeliveryContext,
    limits: &DeliveryLimits,
) -> DeliveryPlan {
    let mut plan = DeliveryPlan::default();
    let mut ready: Vec<&PendingPush> = Vec::new();
    for push in pending {
        if !ctx.notifications_enabled
            || ctx.window_focused
            || ctx.now - push.created_at > limits.stale_after
        {
            plan.in_app.push(push.id.clone());
        } else {
            ready.push(push);
        }
    }
    if ready.is_empty() {
        return plan;
    }
    if ctx
        .quiet_hours
        .is_some_and(|quiet| quiet.contains(ctx.local_minute_of_day))
    {
        plan.hold.extend(ready.iter().map(|push| push.id.clone()));
        return plan;
    }
    let mut budget = limits.max_per_day.saturating_sub(ctx.pushes_today);
    let too_soon = ctx
        .last_push_at
        .is_some_and(|last| ctx.now - last < limits.min_gap_seconds);

    // Briefings first, each on its own.
    let (briefings, others): (Vec<&PendingPush>, Vec<&PendingPush>) = ready
        .into_iter()
        .partition(|push| push.kind == PushKind::Briefing);
    for briefing in briefings {
        if budget == 0 {
            plan.in_app.push(briefing.id.clone());
        } else {
            plan.notify.push(vec![briefing.id.clone()]);
            budget -= 1;
        }
    }
    if others.is_empty() {
        return plan;
    }
    let any_urgent = others.iter().any(|push| push.kind == PushKind::Urgent);
    if budget == 0 {
        plan.in_app
            .extend(others.iter().map(|push| push.id.clone()));
    } else if (too_soon || !plan.notify.is_empty()) && !any_urgent {
        plan.hold.extend(others.iter().map(|push| push.id.clone()));
    } else {
        plan.notify
            .push(others.iter().map(|push| push.id.clone()).collect());
    }
    plan
}

/// Whether the daily briefing should be produced now: after the chosen
/// time and within four hours of it, or (if the app wasn't running then)
/// the first time it runs before noon. Never twice a day.
pub fn briefing_due(local_minute_of_day: u16, briefing_minute: u16, already_today: bool) -> bool {
    if already_today || local_minute_of_day < briefing_minute {
        return false;
    }
    local_minute_of_day <= briefing_minute.saturating_add(4 * 60) || local_minute_of_day < 12 * 60
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn push(id: &str, kind: PushKind, age: i64) -> PendingPush {
        PendingPush {
            id: id.into(),
            kind,
            created_at: NOW - age,
        }
    }

    fn ctx() -> DeliveryContext {
        DeliveryContext {
            now: NOW,
            local_minute_of_day: 10 * 60,
            notifications_enabled: true,
            window_focused: false,
            quiet_hours: None,
            pushes_today: 0,
            last_push_at: None,
        }
    }

    #[test]
    fn coalesces_pending_into_one_notification() {
        let plan = plan_delivery(
            &[
                push("a", PushKind::Normal, 10),
                push("b", PushKind::Normal, 20),
            ],
            &ctx(),
            &DeliveryLimits::default(),
        );
        assert_eq!(plan.notify, vec![vec!["a".to_string(), "b".to_string()]]);
        assert!(plan.hold.is_empty() && plan.in_app.is_empty());
    }

    #[test]
    fn focused_window_or_disabled_notifications_stay_in_app() {
        let pending = [push("a", PushKind::Urgent, 10)];
        let focused = DeliveryContext {
            window_focused: true,
            ..ctx()
        };
        assert_eq!(
            plan_delivery(&pending, &focused, &DeliveryLimits::default()).in_app,
            vec!["a"]
        );
        let off = DeliveryContext {
            notifications_enabled: false,
            ..ctx()
        };
        assert_eq!(
            plan_delivery(&pending, &off, &DeliveryLimits::default()).in_app,
            vec!["a"]
        );
    }

    #[test]
    fn quiet_hours_hold_until_they_end_and_stale_items_drop_to_the_app() {
        let quiet = QuietHours {
            start_minute: 22 * 60,
            end_minute: 7 * 60,
            allow_watch: true,
            allow_vip: true,
        };
        let night = DeliveryContext {
            local_minute_of_day: 23 * 60,
            quiet_hours: Some(quiet),
            ..ctx()
        };
        let pending = [
            push("fresh", PushKind::Normal, 60),
            push("old", PushKind::Normal, 13 * 3600),
        ];
        let plan = plan_delivery(&pending, &night, &DeliveryLimits::default());
        assert_eq!(plan.hold, vec!["fresh"]);
        assert_eq!(plan.in_app, vec!["old"]);
        let morning = DeliveryContext {
            local_minute_of_day: 8 * 60,
            ..night
        };
        assert_eq!(
            plan_delivery(&pending[..1], &morning, &DeliveryLimits::default())
                .notify
                .len(),
            1
        );
    }

    #[test]
    fn gap_and_daily_cap() {
        let pending = [push("a", PushKind::Normal, 10)];
        let recent = DeliveryContext {
            last_push_at: Some(NOW - 600),
            ..ctx()
        };
        assert_eq!(
            plan_delivery(&pending, &recent, &DeliveryLimits::default()).hold,
            vec!["a"]
        );
        let urgent = [push("u", PushKind::Urgent, 10)];
        assert_eq!(
            plan_delivery(&urgent, &recent, &DeliveryLimits::default())
                .notify
                .len(),
            1,
            "urgent ignores the gap"
        );
        let capped = DeliveryContext {
            pushes_today: 6,
            ..ctx()
        };
        assert_eq!(
            plan_delivery(&urgent, &capped, &DeliveryLimits::default()).in_app,
            vec!["u"]
        );
    }

    #[test]
    fn briefing_goes_alone_and_others_wait_their_turn() {
        let pending = [
            push("brief", PushKind::Briefing, 5),
            push("a", PushKind::Normal, 5),
        ];
        let plan = plan_delivery(&pending, &ctx(), &DeliveryLimits::default());
        assert_eq!(plan.notify, vec![vec!["brief".to_string()]]);
        assert_eq!(plan.hold, vec!["a"]);
    }

    #[test]
    fn briefing_window() {
        let nine = 9 * 60;
        assert!(!briefing_due(8 * 60, nine, false));
        assert!(briefing_due(9 * 60, nine, false));
        assert!(briefing_due(11 * 60 + 30, nine, false));
        assert!(briefing_due(13 * 60, nine, false), "within four hours");
        assert!(!briefing_due(14 * 60, nine, false));
        assert!(!briefing_due(10 * 60, nine, true));
        let evening = 18 * 60;
        assert!(briefing_due(21 * 60, evening, false));
        assert!(!briefing_due(23 * 60, evening, false));
    }
}
