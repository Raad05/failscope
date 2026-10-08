//! Where account bytes come from: the RPC in production, a fixed map in tests.

use std::collections::HashMap;
use std::future::Future;

use base64::Engine;
use serde::Deserialize;

use crate::IdlError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountData {
    pub owner: String,
    pub data: Vec<u8>,
}

pub trait AccountSource: Send + Sync {
    /// Accounts in the same order as `addresses`; `None` where none exists.
    fn get_accounts(
        &self,
        addresses: &[String],
    ) -> impl Future<Output = Result<Vec<Option<AccountData>>, IdlError>> + Send;
}

/// JSON-RPC `getMultipleAccounts` (base64, `confirmed`).
#[derive(Debug, Clone)]
pub struct RpcAccountSource {
    client: reqwest::Client,
    url: String,
}

impl RpcAccountSource {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_client(reqwest::Client::new(), url)
    }

    pub fn with_client(client: reqwest::Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }
}

#[derive(Deserialize)]
struct RpcResponse {
    result: Option<RpcResult>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RpcResult {
    value: Vec<Option<RpcAccount>>,
}

/// Account as `getAccountInfo` / `getMultipleAccounts` return it with
/// `encoding: "base64"`. Also the shape of fixtures/idl_accounts `value`.
#[derive(Debug, Deserialize)]
pub struct RpcAccount {
    pub owner: String,
    /// `[base64, "base64"]`
    pub data: (String, String),
}

impl RpcAccount {
    pub fn decode(&self) -> Result<AccountData, IdlError> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&self.data.0)
            .map_err(|_| IdlError::Malformed("account data is not base64"))?;
        Ok(AccountData {
            owner: self.owner.clone(),
            data,
        })
    }
}

impl AccountSource for RpcAccountSource {
    async fn get_accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<AccountData>>, IdlError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getMultipleAccounts",
            "params": [addresses, {"encoding": "base64", "commitment": "confirmed"}],
        });
        let response: RpcResponse = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(error) = response.error {
            return Err(IdlError::Rpc(error.to_string()));
        }
        let accounts = response
            .result
            .ok_or_else(|| IdlError::Rpc("response has neither result nor error".to_string()))?
            .value;
        if accounts.len() != addresses.len() {
            return Err(IdlError::Rpc(format!(
                "asked for {} accounts, got {}",
                addresses.len(),
                accounts.len()
            )));
        }
        accounts
            .into_iter()
            .map(|a| a.map(|a| a.decode()).transpose())
            .collect()
    }
}

/// Fixed accounts, for tests and offline use. Unknown addresses don't exist.
#[derive(Debug, Clone, Default)]
pub struct StaticAccounts {
    pub accounts: HashMap<String, AccountData>,
}

impl AccountSource for StaticAccounts {
    async fn get_accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<AccountData>>, IdlError> {
        Ok(addresses
            .iter()
            .map(|a| self.accounts.get(a).cloned())
            .collect())
    }
}
