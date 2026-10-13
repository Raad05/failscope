//! Features → points → class.

use serde::{Deserialize, Serialize};

/// Thresholds and weights. Every field has a default, so a config file only
/// needs the values it changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScoreConfig {
    /// Sliding window per signer, in slots (1500 ≈ 10 minutes at 400 ms).
    pub window_slots: u64,
    /// Failures in the window that count as a high rate.
    pub high_rate_failures: u32,
    /// Failures in the window that count as a very high rate.
    pub very_high_rate_failures: u32,
    /// Same program + error repeated at least this many times...
    pub retry_run_min: u32,
    /// ...with at most this many slots between consecutive attempts.
    pub retry_gap_slots: u64,
    /// A price at or above this percentile of the program's recent prices
    /// is a fee outlier.
    pub fee_outlier_percentile: f64,
    /// Prices needed for a program before percentiles mean anything.
    pub fee_min_samples: u32,
    /// Recent prices kept per program.
    pub fee_sample_size: u32,
    /// At most this many failures in the window counts as low activity.
    pub low_activity_max_failures: u32,
    pub weights: Weights,
    /// Score at or above which a signer is a likely bot.
    pub bot_threshold: u8,
    /// Score below which a signer is likely human.
    pub human_below: u8,
}

impl Default for ScoreConfig {
    fn default() -> Self {
        Self {
            window_slots: 1_500,
            high_rate_failures: 20,
            very_high_rate_failures: 100,
            retry_run_min: 5,
            retry_gap_slots: 3,
            fee_outlier_percentile: 0.95,
            fee_min_samples: 50,
            fee_sample_size: 1_000,
            low_activity_max_failures: 3,
            weights: Weights::default(),
            bot_threshold: 60,
            human_below: 30,
        }
    }
}

/// Points per signal. `low_activity` is negative: it pulls quiet signers
/// toward "human".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Weights {
    pub high_rate: i32,
    pub very_high_rate: i32,
    pub retry_loop: i32,
    pub jito_tip: i32,
    pub fee_outlier: i32,
    pub silent_program: i32,
    pub low_activity: i32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            high_rate: 35,
            very_high_rate: 20,
            retry_loop: 25,
            jito_tip: 20,
            fee_outlier: 10,
            silent_program: 10,
            low_activity: -20,
        }
    }
}

/// What the tracker measured for one failure. Stored as-is, so the scoring
/// can be redone or replaced by a fitted model without re-ingesting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Features {
    /// The signer's failures in the window, this one included.
    pub window_failures: u32,
    pub window_slots: u64,
    /// Longest run of the same program + error with small slot gaps.
    pub retry_run: u32,
    /// The signer tipped Jito in any failure in the window.
    pub tipped: bool,
    pub cu_price: u64,
    /// Share of the program's recent prices below this one (0..=1); `None`
    /// until the program has enough samples, or when the price is 0.
    pub fee_percentile: Option<f64>,
    /// The failing program has no IDL entry and logged no Anchor error, so
    /// the error is unnamed (typical of private trading programs).
    pub silent_program: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    HighRate,
    VeryHighRate,
    RetryLoop,
    JitoTip,
    FeeOutlier,
    SilentProgram,
    LowActivity,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::HighRate => "high_rate",
            Reason::VeryHighRate => "very_high_rate",
            Reason::RetryLoop => "retry_loop",
            Reason::JitoTip => "jito_tip",
            Reason::FeeOutlier => "fee_outlier",
            Reason::SilentProgram => "silent_program",
            Reason::LowActivity => "low_activity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotClass {
    LikelyBot,
    Unclear,
    LikelyHuman,
}

impl BotClass {
    pub fn as_str(self) -> &'static str {
        match self {
            BotClass::LikelyBot => "likely_bot",
            BotClass::Unclear => "unclear",
            BotClass::LikelyHuman => "likely_human",
        }
    }

    pub fn from_score(score: u8, config: &ScoreConfig) -> Self {
        if score >= config.bot_threshold {
            BotClass::LikelyBot
        } else if score < config.human_below {
            BotClass::LikelyHuman
        } else {
            BotClass::Unclear
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotScore {
    /// 0..=100.
    pub score: u8,
    pub class: BotClass,
    /// Signals that fired, in a fixed order.
    pub reasons: Vec<Reason>,
    pub features: Features,
}

/// Adds the weight of every signal that fires, clamps to 0..=100.
pub fn score(features: Features, config: &ScoreConfig) -> BotScore {
    let w = &config.weights;
    let fee_outlier = features
        .fee_percentile
        .is_some_and(|p| p >= config.fee_outlier_percentile);
    let low_activity = features.window_failures <= config.low_activity_max_failures
        && !features.tipped
        && !fee_outlier;

    let signals = [
        (
            features.window_failures >= config.high_rate_failures,
            Reason::HighRate,
            w.high_rate,
        ),
        (
            features.window_failures >= config.very_high_rate_failures,
            Reason::VeryHighRate,
            w.very_high_rate,
        ),
        (
            features.retry_run >= config.retry_run_min,
            Reason::RetryLoop,
            w.retry_loop,
        ),
        (features.tipped, Reason::JitoTip, w.jito_tip),
        (fee_outlier, Reason::FeeOutlier, w.fee_outlier),
        (
            features.silent_program,
            Reason::SilentProgram,
            w.silent_program,
        ),
        (low_activity, Reason::LowActivity, w.low_activity),
    ];

    let mut total: i32 = 0;
    let mut reasons = Vec::new();
    for (fired, reason, weight) in signals {
        if fired {
            total = total.saturating_add(weight);
            reasons.push(reason);
        }
    }
    let score = u8::try_from(total.clamp(0, 100)).unwrap_or(0);
    BotScore {
        score,
        class: BotClass::from_score(score, config),
        reasons,
        features,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> Features {
        Features {
            window_failures: 1,
            window_slots: 1_500,
            retry_run: 1,
            tipped: false,
            cu_price: 0,
            fee_percentile: None,
            silent_program: false,
        }
    }

    #[test]
    fn a_single_ordinary_failure_is_likely_human() {
        let s = score(quiet(), &ScoreConfig::default());
        assert_eq!(s.score, 0);
        assert_eq!(s.class, BotClass::LikelyHuman);
        assert_eq!(s.reasons, vec![Reason::LowActivity]);
    }

    #[test]
    fn a_fast_tipping_retry_loop_is_likely_bot() {
        let f = Features {
            window_failures: 140,
            retry_run: 30,
            tipped: true,
            fee_percentile: Some(0.99),
            ..quiet()
        };
        let s = score(f, &ScoreConfig::default());
        assert_eq!(s.score, 100, "35+20+25+20+10 clamps to 100");
        assert_eq!(s.class, BotClass::LikelyBot);
        assert_eq!(
            s.reasons,
            vec![
                Reason::HighRate,
                Reason::VeryHighRate,
                Reason::RetryLoop,
                Reason::JitoTip,
                Reason::FeeOutlier
            ]
        );
    }

    #[test]
    fn one_tipped_failure_alone_is_unclear() {
        // A single transaction can't tell a bot from a careful trader.
        let f = Features {
            tipped: true,
            silent_program: true,
            ..quiet()
        };
        let s = score(f, &ScoreConfig::default());
        assert_eq!(s.score, 30);
        assert_eq!(s.class, BotClass::Unclear);
    }

    #[test]
    fn thresholds_are_inclusive_where_documented() {
        let c = ScoreConfig::default();
        assert_eq!(BotClass::from_score(60, &c), BotClass::LikelyBot);
        assert_eq!(BotClass::from_score(59, &c), BotClass::Unclear);
        assert_eq!(BotClass::from_score(30, &c), BotClass::Unclear);
        assert_eq!(BotClass::from_score(29, &c), BotClass::LikelyHuman);
    }

    #[test]
    fn partial_config_keeps_defaults() {
        let c: ScoreConfig =
            serde_json::from_str(r#"{"high_rate_failures": 5, "weights": {"jito_tip": 40}}"#)
                .unwrap();
        assert_eq!(c.high_rate_failures, 5);
        assert_eq!(c.weights.jito_tip, 40);
        assert_eq!(c.weights.high_rate, 35);
        assert_eq!(c.window_slots, 1_500);
        assert!(serde_json::from_str::<ScoreConfig>(r#"{"typo": 1}"#).is_err());
    }
}
