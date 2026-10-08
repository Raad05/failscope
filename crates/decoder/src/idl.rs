//! Error tables taken from Anchor IDLs. Fetching IDLs is the `idl` crate's
//! job; the decoder only gets the parsed JSON.
//!
//! Both IDL formats (legacy and the 0.30+ spec) use the same shape for
//! errors: `"errors": [{"code": 6000, "name": "...", "msg": "..."}]`, with
//! `msg` optional.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdlError {
    pub name: String,
    pub message: Option<String>,
}

/// What the decoder needs from IDLs. Keyed by (program id, code), because the
/// same code means different things in different programs.
pub trait ErrorLookup {
    fn idl_error(&self, program_id: &str, code: u32) -> Option<IdlError>;
    /// True if an IDL is known for this program, i.e. it is an Anchor program
    /// and Anchor's framework error codes apply to it.
    fn has_idl(&self, program_id: &str) -> bool;
}

/// In-memory [`ErrorLookup`] built from IDL JSON documents.
#[derive(Debug, Clone, Default)]
pub struct IdlErrorTable {
    programs: HashMap<String, HashMap<u32, IdlError>>,
}

#[derive(Deserialize)]
struct IdlErrors {
    #[serde(default)]
    errors: Vec<RawError>,
}

#[derive(Deserialize)]
struct RawError {
    code: u32,
    name: String,
    msg: Option<String>,
}

impl IdlErrorTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the errors of one IDL. Returns how many were read.
    pub fn add_idl(
        &mut self,
        program_id: &str,
        idl: &serde_json::Value,
    ) -> Result<usize, serde_json::Error> {
        let parsed = IdlErrors::deserialize(idl)?;
        let errors: HashMap<u32, IdlError> = parsed
            .errors
            .into_iter()
            .map(|e| {
                (
                    e.code,
                    IdlError {
                        name: e.name,
                        message: e.msg,
                    },
                )
            })
            .collect();
        let n = errors.len();
        self.programs.insert(program_id.to_string(), errors);
        Ok(n)
    }
}

impl ErrorLookup for IdlErrorTable {
    fn idl_error(&self, program_id: &str, code: u32) -> Option<IdlError> {
        self.programs.get(program_id)?.get(&code).cloned()
    }

    fn has_idl(&self, program_id: &str) -> bool {
        self.programs.contains_key(program_id)
    }
}

/// An [`ErrorLookup`] that knows no IDLs.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoIdls;

impl ErrorLookup for NoIdls {
    fn idl_error(&self, _: &str, _: u32) -> Option<IdlError> {
        None
    }

    fn has_idl(&self, _: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_errors_and_keys_by_program() {
        let mut t = IdlErrorTable::new();
        let n = t
            .add_idl(
                "P1",
                &json!({"errors": [{"code": 6000, "name": "A", "msg": "a"}, {"code": 6001, "name": "B"}]}),
            )
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(t.idl_error("P1", 6001).unwrap().message, None);
        assert_eq!(t.idl_error("P2", 6000), None);
        assert!(t.has_idl("P1"));
    }

    #[test]
    fn idl_without_errors_is_still_known() {
        let mut t = IdlErrorTable::new();
        assert_eq!(t.add_idl("P", &json!({"instructions": []})).unwrap(), 0);
        assert!(t.has_idl("P"));
    }
}
