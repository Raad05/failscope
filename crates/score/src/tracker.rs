//! Per-signer sliding windows and per-program price samples.

use std::collections::{HashMap, VecDeque};

use failscope_decoder::{DecodeSource, DecodedFailure};

use crate::rules::{score, BotScore, Features, ScoreConfig};

/// The parts of a failure the tracker needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub signature: String,
    pub signer: String,
    pub slot: u64,
    pub program: Option<String>,
    /// Identifies "the same error" for retry detection.
    pub error_key: String,
    pub cu_price: u64,
    pub jito_tip: bool,
    pub silent_program: bool,
}

impl Observation {
    /// `None` when the failure has no signer (never the case for landed
    /// transactions, but the field is optional).
    pub fn from_failure(f: &DecodedFailure) -> Option<Self> {
        Some(Self {
            signature: f.signature.clone(),
            signer: f.signer.clone()?,
            slot: f.slot,
            program: f.failing_program_id.clone(),
            error_key: error_key(
                &f.error_kind,
                f.error_code.map(i64::from),
                f.error_name.as_deref(),
            ),
            cu_price: f.cu_price,
            jito_tip: f.jito_tip,
            silent_program: f.decode_source == DecodeSource::Unknown,
        })
    }
}

/// Stable key for "same error": the name if known, else kind and code.
pub fn error_key(kind: &str, code: Option<i64>, name: Option<&str>) -> String {
    match (name, code) {
        (Some(n), _) => n.to_string(),
        (None, Some(c)) => format!("{kind}:{c}"),
        (None, None) => kind.to_string(),
    }
}

#[derive(Debug, Clone)]
struct Entry {
    signature: String,
    slot: u64,
    program: Option<String>,
    error_key: String,
    tipped: bool,
}

/// Remembers recent failures per signer and recent prices per program.
/// Memory stays bounded: entries older than the window are dropped, each
/// signer keeps at most [`MAX_ENTRIES_PER_SIGNER`], and idle signers are
/// swept periodically.
#[derive(Debug)]
pub struct SignerTracker {
    config: ScoreConfig,
    signers: HashMap<String, VecDeque<Entry>>,
    prices: HashMap<String, VecDeque<u64>>,
    max_slot: u64,
    since_sweep: u32,
}

/// Enough to saturate every rate rule.
pub const MAX_ENTRIES_PER_SIGNER: usize = 1_000;
const SWEEP_EVERY: u32 = 10_000;

impl SignerTracker {
    pub fn new(config: ScoreConfig) -> Self {
        Self {
            config,
            signers: HashMap::new(),
            prices: HashMap::new(),
            max_slot: 0,
            since_sweep: 0,
        }
    }

    pub fn config(&self) -> &ScoreConfig {
        &self.config
    }

    /// Signers currently tracked.
    pub fn signers(&self) -> usize {
        self.signers.len()
    }

    /// Records the failure and scores its signer. Returns `None` for a
    /// signature already in the signer's window (replayed duplicates must
    /// not inflate the rate).
    pub fn observe(&mut self, obs: &Observation) -> Option<BotScore> {
        let window = self.config.window_slots;
        self.max_slot = self.max_slot.max(obs.slot);
        let horizon = self.max_slot.saturating_sub(window);

        let entries = self.signers.entry(obs.signer.clone()).or_default();
        if entries.iter().any(|e| e.signature == obs.signature) {
            return None;
        }
        entries.retain(|e| e.slot > horizon);
        if entries.len() >= MAX_ENTRIES_PER_SIGNER {
            entries.pop_front();
        }
        entries.push_back(Entry {
            signature: obs.signature.clone(),
            slot: obs.slot,
            program: obs.program.clone(),
            error_key: obs.error_key.clone(),
            tipped: obs.jito_tip,
        });

        // Window around this failure (replay can deliver slots slightly out
        // of order, so later entries count too).
        let since = obs.slot.saturating_sub(window);
        let in_window: Vec<&Entry> = entries.iter().filter(|e| e.slot > since).collect();
        let window_failures = u32::try_from(in_window.len()).unwrap_or(u32::MAX);
        let tipped = in_window.iter().any(|e| e.tipped);
        let mut same: Vec<u64> = in_window
            .iter()
            .filter(|e| e.program == obs.program && e.error_key == obs.error_key)
            .map(|e| e.slot)
            .collect();
        same.sort_unstable();
        let retry_run = longest_run(&same, self.config.retry_gap_slots);

        let fee_percentile = obs
            .program
            .as_ref()
            .and_then(|p| self.price_percentile(p, obs.cu_price));
        if let Some(program) = &obs.program {
            let samples = self.prices.entry(program.clone()).or_default();
            if samples.len() >= self.config.fee_sample_size as usize {
                samples.pop_front();
            }
            samples.push_back(obs.cu_price);
        }

        self.since_sweep += 1;
        if self.since_sweep >= SWEEP_EVERY {
            self.sweep();
        }

        Some(score(
            Features {
                window_failures,
                window_slots: window,
                retry_run,
                tipped,
                cu_price: obs.cu_price,
                fee_percentile,
                silent_program: obs.silent_program,
            },
            &self.config,
        ))
    }

    /// Share of the program's recent prices strictly below `price`.
    fn price_percentile(&self, program: &str, price: u64) -> Option<f64> {
        if price == 0 {
            return None;
        }
        let samples = self.prices.get(program)?;
        if samples.len() < self.config.fee_min_samples as usize {
            return None;
        }
        let below = samples.iter().filter(|&&p| p < price).count();
        Some(below as f64 / samples.len() as f64)
    }

    /// Drops signers with nothing left in the window.
    pub fn sweep(&mut self) {
        let horizon = self.max_slot.saturating_sub(self.config.window_slots);
        self.signers
            .retain(|_, entries| entries.back().is_some_and(|e| e.slot > horizon));
        self.since_sweep = 0;
    }
}

/// Longest run in sorted `slots` where consecutive values differ by at most
/// `max_gap`.
fn longest_run(slots: &[u64], max_gap: u64) -> u32 {
    let mut best = 0u32;
    let mut run = 0u32;
    let mut prev: Option<u64> = None;
    for &s in slots {
        run = match prev {
            Some(p) if s - p <= max_gap => run + 1,
            _ => 1,
        };
        best = best.max(run);
        prev = Some(s);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{BotClass, Reason};

    fn obs(sig: u64, signer: &str, slot: u64) -> Observation {
        Observation {
            signature: format!("sig{sig}"),
            signer: signer.to_string(),
            slot,
            program: Some("JUP".to_string()),
            error_key: "SlippageToleranceExceeded".to_string(),
            cu_price: 1_000,
            jito_tip: false,
            silent_program: false,
        }
    }

    #[test]
    fn longest_run_respects_gaps() {
        assert_eq!(longest_run(&[], 3), 0);
        assert_eq!(longest_run(&[10], 3), 1);
        assert_eq!(longest_run(&[1, 2, 5, 9, 10, 11, 12], 3), 4);
        assert_eq!(longest_run(&[1, 2, 5, 9, 10, 11, 12], 1), 4);
        assert_eq!(longest_run(&[1, 2, 5, 9, 10, 11, 12], 4), 7);
    }

    #[test]
    fn a_bot_hammering_every_slot_becomes_likely_bot() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        let mut last = None;
        for i in 0..120 {
            last = t.observe(&Observation {
                jito_tip: true,
                ..obs(i, "bot", 1_000 + i)
            });
        }
        let s = last.unwrap();
        assert_eq!(s.features.window_failures, 120);
        assert_eq!(s.features.retry_run, 120);
        assert_eq!(s.class, BotClass::LikelyBot);
        assert!(s.reasons.contains(&Reason::VeryHighRate));
        assert!(s.reasons.contains(&Reason::RetryLoop));
    }

    #[test]
    fn early_failures_of_a_bot_score_lower_than_later_ones() {
        // Documented lag: the rate signal needs failures to accumulate.
        let mut t = SignerTracker::new(ScoreConfig::default());
        let first = t.observe(&obs(0, "bot", 1_000)).unwrap();
        for i in 1..30 {
            t.observe(&obs(i, "bot", 1_000 + i));
        }
        let later = t.observe(&obs(30, "bot", 1_030)).unwrap();
        assert_eq!(first.class, BotClass::LikelyHuman);
        assert_eq!(later.class, BotClass::LikelyBot);
    }

    #[test]
    fn a_person_retrying_a_few_times_stays_likely_human() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        // Three attempts about 10-20 seconds apart.
        for (i, slot) in [1_000, 1_030, 1_070].into_iter().enumerate() {
            let s = t.observe(&obs(i as u64, "alice", slot)).unwrap();
            assert_eq!(s.class, BotClass::LikelyHuman, "attempt {i}");
        }
    }

    #[test]
    fn replayed_duplicates_are_ignored() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        assert!(t.observe(&obs(1, "a", 10)).is_some());
        assert!(t.observe(&obs(1, "a", 10)).is_none());
        assert_eq!(
            t.observe(&obs(2, "a", 11))
                .unwrap()
                .features
                .window_failures,
            2
        );
    }

    #[test]
    fn old_failures_leave_the_window() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        for i in 0..25 {
            t.observe(&obs(i, "a", 100 + i));
        }
        let later = t.observe(&obs(99, "a", 100 + 1_600)).unwrap();
        assert_eq!(later.features.window_failures, 1);
        t.sweep();
        assert_eq!(t.signers(), 1);
        t.observe(&obs(100, "b", 10_000));
        t.sweep();
        assert_eq!(t.signers(), 1, "idle signer a swept");
    }

    #[test]
    fn fee_outliers_need_enough_samples() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        for i in 0..49 {
            t.observe(&obs(i, &format!("s{i}"), 100));
        }
        let early = t
            .observe(&Observation {
                cu_price: 1_000_000,
                ..obs(49, "rich", 100)
            })
            .unwrap();
        assert_eq!(early.features.fee_percentile, None, "only 49 samples");
        let late = t
            .observe(&Observation {
                cu_price: 1_000_000,
                ..obs(50, "rich2", 100)
            })
            .unwrap();
        assert!(late.features.fee_percentile.unwrap() >= 0.95);
        assert!(late.reasons.contains(&Reason::FeeOutlier));
    }

    #[test]
    fn signers_do_not_share_windows() {
        let mut t = SignerTracker::new(ScoreConfig::default());
        for i in 0..50 {
            t.observe(&obs(i, "bot", 1_000 + i));
        }
        let other = t.observe(&obs(999, "alice", 1_050)).unwrap();
        assert_eq!(other.features.window_failures, 1);
        assert_eq!(other.class, BotClass::LikelyHuman);
    }
}
