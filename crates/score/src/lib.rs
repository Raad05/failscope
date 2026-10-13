//! Bot-likelihood scoring for failing signers. Pure, no I/O.
//!
//! Many failed transactions on Solana come from bots whose failures are
//! expected (arbitrage attempts that weren't profitable). To estimate how
//! many *people* a program's failures affect, each failure is tagged with a
//! score for its signer:
//!
//! 1. [`SignerTracker`] keeps each signer's recent failures over a sliding
//!    window measured in slots, and recent compute-unit prices per program.
//! 2. From those it builds [`Features`] for the new failure.
//! 3. [`score`] adds fixed points per signal (see [`ScoreConfig`]) and maps
//!    the total to a [`BotClass`], keeping the reasons.
//!
//! The points are a starting point, not ground truth; the raw features are
//! stored with every score so a model can be fitted on labelled data later.

mod rules;
mod tracker;

pub use rules::{score, BotClass, BotScore, Features, Reason, ScoreConfig, Weights};
pub use tracker::{error_key, Observation, SignerTracker, MAX_ENTRIES_PER_SIGNER};
