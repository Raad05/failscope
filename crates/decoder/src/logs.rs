//! Parses program logs into the failing frame.
//!
//! Log lines this understands:
//! - `Program <id> invoke [<depth>]`
//! - `Program <id> success`
//! - `Program <id> failed: <reason>`
//! - `Program log: AnchorError ...`
//! - `Log truncated`
//!
//! Every frame on the stack logs `failed:` while unwinding, so the first
//! `failed:` line is the innermost failing program. Outer frames may report a
//! different reason (e.g. `Program failed to complete` for CU exhaustion), so
//! only the first one is used. Anything else is ignored; malformed input never
//! panics.

use crate::anchor_log::{parse_anchor_error, AnchorErrorLog};

/// The innermost failing frame found in the logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFailure {
    pub program_id: String,
    /// Invoke depth of the failing frame (top level = 1).
    pub depth: u32,
    /// Text after `failed: `.
    pub reason: String,
    /// The last AnchorError logged directly by the failing frame, if any.
    pub anchor_error: Option<AnchorErrorLog>,
    /// Program of the depth-1 frame the failure happened under.
    pub root_program_id: Option<String>,
    /// False if the `failed:` line didn't match the frame on top of the
    /// stack (e.g. logs started mid-transaction), so the stack can't be trusted.
    pub stack_consistent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogAnalysis {
    pub truncated: bool,
    pub failure: Option<FrameFailure>,
}

#[derive(Debug)]
struct Frame {
    program_id: String,
    depth: u32,
    anchor_error: Option<AnchorErrorLog>,
}

enum Line<'a> {
    Invoke {
        program_id: &'a str,
        depth: u32,
    },
    Success,
    Failed {
        program_id: &'a str,
        reason: &'a str,
    },
    AnchorError(AnchorErrorLog),
    Truncated,
    Other,
}

pub fn analyze_logs<S: AsRef<str>>(logs: &[S]) -> LogAnalysis {
    let mut stack: Vec<Frame> = Vec::new();
    let mut analysis = LogAnalysis::default();

    for line in logs {
        match classify(line.as_ref()) {
            Line::Invoke { program_id, depth } => stack.push(Frame {
                program_id: program_id.to_string(),
                depth,
                anchor_error: None,
            }),
            Line::Success => {
                stack.pop();
            }
            Line::Failed { program_id, reason } => {
                if analysis.failure.is_none() {
                    let top = stack.last();
                    let consistent = top.is_some_and(|f| f.program_id == program_id);
                    analysis.failure = Some(FrameFailure {
                        program_id: program_id.to_string(),
                        depth: match top {
                            Some(f) if consistent => f.depth,
                            _ => u32::try_from(stack.len()).unwrap_or(u32::MAX).max(1),
                        },
                        reason: reason.to_string(),
                        anchor_error: top
                            .filter(|_| consistent)
                            .and_then(|f| f.anchor_error.clone()),
                        root_program_id: stack.first().map(|f| f.program_id.clone()),
                        stack_consistent: consistent,
                    });
                }
                stack.pop();
            }
            Line::AnchorError(err) => {
                if let Some(top) = stack.last_mut() {
                    top.anchor_error = Some(err);
                }
            }
            Line::Truncated => analysis.truncated = true,
            Line::Other => {}
        }
    }
    analysis
}

fn classify(line: &str) -> Line<'_> {
    if line == "Log truncated" {
        return Line::Truncated;
    }
    if let Some(rest) = line.strip_prefix("Program log: ") {
        return parse_anchor_error(rest).map_or(Line::Other, Line::AnchorError);
    }
    let Some(rest) = line.strip_prefix("Program ") else {
        return Line::Other;
    };
    let Some((program_id, rest)) = rest.split_once(' ') else {
        return Line::Other;
    };
    if !looks_like_pubkey(program_id) {
        return Line::Other;
    }
    if rest == "success" {
        return Line::Success;
    }
    if let Some(reason) = rest.strip_prefix("failed: ") {
        return Line::Failed { program_id, reason };
    }
    if let Some(depth) = rest
        .strip_prefix("invoke [")
        .and_then(|d| d.strip_suffix(']'))
        .and_then(|d| d.parse().ok())
    {
        return Line::Invoke { program_id, depth };
    }
    Line::Other
}

/// Base58, 32–44 chars. Rules out `log:`, `data:`, `return:` and friends.
fn looks_like_pubkey(s: &str) -> bool {
    (32..=44).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() && !matches!(b, b'0' | b'O' | b'I' | b'l'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn logs(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn innermost_failure_wins_over_outer_reports() {
        let a = analyze_logs(&logs(&[
            &format!("Program {A} invoke [1]"),
            &format!("Program {B} invoke [2]"),
            &format!("Program {B} failed: custom program error: 0x1"),
            &format!("Program {A} failed: custom program error: 0x1"),
        ]));
        let f = a.failure.unwrap();
        assert_eq!(f.program_id, B);
        assert_eq!(f.depth, 2);
        assert_eq!(f.root_program_id.as_deref(), Some(A));
        assert!(f.stack_consistent);
        assert!(!a.truncated);
    }

    #[test]
    fn earlier_successful_cpis_are_popped() {
        let a = analyze_logs(&logs(&[
            &format!("Program {A} invoke [1]"),
            &format!("Program {B} invoke [2]"),
            &format!("Program {B} success"),
            &format!("Program {A} failed: custom program error: 0x1771"),
        ]));
        let f = a.failure.unwrap();
        assert_eq!((f.program_id.as_str(), f.depth), (A, 1));
    }

    #[test]
    fn anchor_error_is_attached_only_to_the_frame_that_logged_it() {
        let a = analyze_logs(&logs(&[
            &format!("Program {A} invoke [1]"),
            "Program log: AnchorError occurred. Error Code: Outer. Error Number: 6000. Error Message: outer.",
            &format!("Program {B} invoke [2]"),
            &format!("Program {B} failed: custom program error: 0x1"),
        ]));
        assert_eq!(a.failure.unwrap().anchor_error, None);
    }

    #[test]
    fn truncation_is_flagged_and_failure_may_be_missing() {
        let a = analyze_logs(&logs(&[
            &format!("Program {A} invoke [1]"),
            "Log truncated",
        ]));
        assert!(a.truncated);
        assert_eq!(a.failure, None);
    }

    #[test]
    fn failed_line_without_matching_frame_is_marked_inconsistent() {
        let a = analyze_logs(&logs(&[&format!("Program {B} failed: boom")]));
        let f = a.failure.unwrap();
        assert!(!f.stack_consistent);
        assert_eq!(f.depth, 1);
        assert_eq!(f.root_program_id, None);
    }

    #[test]
    fn program_log_lines_are_not_mistaken_for_frames() {
        let a = analyze_logs(&logs(&[
            &format!("Program {A} invoke [1]"),
            "Program log: failed: not a real failure",
            "Program data: AAAA",
            &format!("Program {A} success"),
        ]));
        assert_eq!(a.failure, None);
    }
}
