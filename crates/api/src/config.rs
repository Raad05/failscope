//! Runtime configuration, read from the environment (and `.env` if present).

use std::env;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("environment variable {0} is not valid unicode")]
    NotUnicode(&'static str),
}

/// Settings shared by all failscope services. Fields are optional until the
/// milestone that needs them makes them required.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub database_url: Option<String>,
    pub rpc_url: Option<String>,
    pub yellowstone_endpoint: Option<String>,
    pub yellowstone_x_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: var("DATABASE_URL")?,
            rpc_url: var("RPC_URL")?,
            yellowstone_endpoint: var("YELLOWSTONE_ENDPOINT")?,
            yellowstone_x_token: var("YELLOWSTONE_X_TOKEN")?,
        })
    }
}

fn var(key: &'static str) -> Result<Option<String>, ConfigError> {
    match env::var(key) {
        Ok(v) if v.is_empty() => Ok(None),
        Ok(v) => Ok(Some(v)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::NotUnicode(key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_empty_vars_are_none() {
        assert_eq!(var("FAILSCOPE_TEST_SURELY_UNSET").unwrap(), None);
    }
}
