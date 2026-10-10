//! Spike detection. Pure: per-minute counts in, a decision out.
//!
//! A program spikes when its failures in the last `window` minutes reach
//!
//! ```text
//! threshold = max(min_failures, factor × max(baseline_per_window, 1))
//! ```
//!
//! where `baseline_per_window` is its average count per window over the
//! `baseline` minutes before that. Comparing a program with its own recent
//! past means a busy program's normal failure rate doesn't alert, and a
//! quiet one's sudden burst does.
//!
//! A firing alert resolves only once the count falls below
//! `resolve_ratio × threshold`, so a count hovering at the threshold doesn't
//! flap between firing and resolved.

use failscope_store::AlertState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub window_minutes: u32,
    pub baseline_minutes: u32,
    pub factor: f64,
    pub min_failures: u64,
    pub resolve_ratio: f64,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            window_minutes: 5,
            baseline_minutes: 60,
            factor: 3.0,
            min_failures: 10,
            resolve_ratio: 0.75,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evaluation {
    pub current: u64,
    pub baseline_per_window: f64,
    pub threshold: f64,
}

/// What to do about one program this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Fire,
    Resolve,
    None,
}

impl Rule {
    /// Minutes of history `evaluate` needs.
    pub fn span_minutes(&self) -> u32 {
        self.window_minutes + self.baseline_minutes
    }

    /// `per_minute[0]` is the current minute, `[1]` the one before, ...
    /// Missing minutes count as zero.
    pub fn evaluate(&self, per_minute: &[u64]) -> Evaluation {
        let window = self.window_minutes as usize;
        let at = |i: usize| per_minute.get(i).copied().unwrap_or(0);
        let current: u64 = (0..window).map(at).sum();
        let baseline_total: u64 = (window..window + self.baseline_minutes as usize)
            .map(at)
            .sum();
        let baseline_per_window = baseline_total as f64 * f64::from(self.window_minutes)
            / f64::from(self.baseline_minutes.max(1));
        let threshold = (self.min_failures as f64).max(self.factor * baseline_per_window.max(1.0));
        Evaluation {
            current,
            baseline_per_window,
            threshold,
        }
    }

    /// State transition given the previous state (`None` = never alerted).
    pub fn decide(&self, previous: Option<AlertState>, e: &Evaluation) -> Action {
        let current = e.current as f64;
        match previous {
            Some(AlertState::Firing) if current < self.resolve_ratio * e.threshold => {
                Action::Resolve
            }
            Some(AlertState::Firing) => Action::None,
            _ if current >= e.threshold => Action::Fire,
            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `window` minutes at `now` per minute, after `baseline` minutes at `before`.
    fn series(rule: &Rule, now: u64, before: u64) -> Vec<u64> {
        let mut v = vec![now; rule.window_minutes as usize];
        v.extend(vec![before; rule.baseline_minutes as usize]);
        v
    }

    #[test]
    fn steady_busy_program_does_not_fire() {
        let rule = Rule::default();
        // 20/min, every minute: 100 per window, baseline 100 per window.
        let e = rule.evaluate(&series(&rule, 20, 20));
        assert_eq!(e.current, 100);
        assert!((e.baseline_per_window - 100.0).abs() < 1e-9);
        assert_eq!(rule.decide(None, &e), Action::None);
    }

    #[test]
    fn spike_against_own_baseline_fires() {
        let rule = Rule::default();
        // Baseline 2/min (10 per window); now 8/min (40) >= 3 × 10.
        let e = rule.evaluate(&series(&rule, 8, 2));
        assert_eq!(e.threshold, 30.0);
        assert_eq!(rule.decide(None, &e), Action::Fire);
        assert_eq!(rule.decide(Some(AlertState::Resolved), &e), Action::Fire);
    }

    #[test]
    fn quiet_program_needs_min_failures() {
        let rule = Rule::default();
        // No history: threshold is max(10, 3 × 1) = 10.
        let small = rule.evaluate(&[3, 2, 1]);
        assert_eq!(small.current, 6);
        assert_eq!(rule.decide(None, &small), Action::None);
        let burst = rule.evaluate(&[6, 4]);
        assert_eq!(rule.decide(None, &burst), Action::Fire);
    }

    #[test]
    fn hysteresis_prevents_flapping() {
        let rule = Rule::default();
        let threshold = 30.0;
        let at = |current| Evaluation {
            current,
            baseline_per_window: 10.0,
            threshold,
        };
        // Firing, then dips just under threshold: stays firing.
        assert_eq!(rule.decide(Some(AlertState::Firing), &at(25)), Action::None);
        // Below 75% of threshold (22.5): resolves.
        assert_eq!(
            rule.decide(Some(AlertState::Firing), &at(22)),
            Action::Resolve
        );
        // Resolved and still low: nothing.
        assert_eq!(
            rule.decide(Some(AlertState::Resolved), &at(22)),
            Action::None
        );
    }

    #[test]
    fn short_history_counts_missing_minutes_as_zero() {
        let rule = Rule::default();
        let e = rule.evaluate(&[]);
        assert_eq!(e.current, 0);
        assert_eq!(e.baseline_per_window, 0.0);
        assert_eq!(rule.decide(None, &e), Action::None);
    }
}
