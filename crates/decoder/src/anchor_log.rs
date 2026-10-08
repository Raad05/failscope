//! Parses Anchor's error log line (the text after `Program log: `). Shapes:
//!
//! - `AnchorError thrown in <file>:<line>. Error Code: <Name>. Error Number: <n>. Error Message: <msg>.`
//! - `AnchorError caused by account: <acct>. Error Code: ...`
//! - `AnchorError occurred. Error Code: ...`

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorErrorLog {
    pub name: String,
    pub number: u32,
    pub message: String,
    /// Account named by `caused by account: <acct>`.
    pub account: Option<String>,
    /// `<file>:<line>` from `thrown in`.
    pub origin: Option<String>,
}

pub fn parse_anchor_error(line: &str) -> Option<AnchorErrorLog> {
    let rest = line.strip_prefix("AnchorError ")?;
    let (head, tail) = rest.split_once(". Error Code: ")?;
    let (name, tail) = tail.split_once(". Error Number: ")?;
    let (number, message) = tail.split_once(". Error Message: ")?;
    let number = number.trim().parse().ok()?;
    let message = message.strip_suffix('.').unwrap_or(message);

    let account = head.strip_prefix("caused by account: ").map(str::to_string);
    let origin = head.strip_prefix("thrown in ").map(str::to_string);

    Some(AnchorErrorLog {
        name: name.to_string(),
        number,
        message: message.to_string(),
        account,
        origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thrown_in() {
        let e = parse_anchor_error("AnchorError thrown in programs/x/src/lib.rs:21. Error Code: AlwaysFails. Error Number: 6000. Error Message: This instruction always fails on purpose.").unwrap();
        assert_eq!(e.name, "AlwaysFails");
        assert_eq!(e.number, 6000);
        assert_eq!(e.message, "This instruction always fails on purpose");
        assert_eq!(e.origin.as_deref(), Some("programs/x/src/lib.rs:21"));
        assert_eq!(e.account, None);
    }

    #[test]
    fn caused_by_account() {
        let e = parse_anchor_error("AnchorError caused by account: counter. Error Code: ConstraintHasOne. Error Number: 2001. Error Message: A has one constraint was violated.").unwrap();
        assert_eq!(e.account.as_deref(), Some("counter"));
        assert_eq!(e.number, 2001);
    }

    #[test]
    fn occurred_and_message_with_periods() {
        let e = parse_anchor_error(
            "AnchorError occurred. Error Code: X. Error Number: 6001. Error Message: a. b. c.",
        )
        .unwrap();
        assert_eq!(e.message, "a. b. c");
        assert_eq!(e.origin, None);
    }

    #[test]
    fn rejects_non_anchor_and_malformed() {
        assert_eq!(parse_anchor_error("Instruction: Route"), None);
        assert_eq!(
            parse_anchor_error(
                "AnchorError occurred. Error Code: X. Error Number: nope. Error Message: m."
            ),
            None
        );
        assert_eq!(
            parse_anchor_error("AnchorError occurred. Error Code: X"),
            None
        );
    }
}
