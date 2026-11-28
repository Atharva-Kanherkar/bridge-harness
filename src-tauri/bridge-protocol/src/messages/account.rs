//! The Bridge account: who is signed in to Bridge itself.
//!
//! Separate from provider accounts (Claude, Codex and the rest), which belong
//! to their vendors. Nothing in this domain accepts or returns a token: the
//! host keeps the refresh token in the OS keychain and the access token in
//! memory, and a client only ever learns who is signed in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    #[default]
    SignedOut,
    /// The sign-in page is open in the person's browser and the host is
    /// waiting for it to come back.
    SigningIn,
    SignedIn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountUser {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub state: AccountState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<AccountUser>,
    /// Why the last sign-in or session refresh ended signed out, in words a
    /// person can act on. Never carries a token or a provider response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether this host can sign in at all: it has a client id and a keychain
    /// to keep the session in. False leaves Bridge working as it does signed
    /// out.
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BeginAccountSignIn {
    /// The page to open in the person's own browser.
    pub authorization_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_signed_out_status_carries_no_user_and_no_token_shaped_field() {
        let wire = serde_json::to_value(AccountStatus { available: true, ..Default::default() }).unwrap();
        assert_eq!(wire, json!({ "state": "signed_out", "available": true }));
    }

    #[test]
    fn a_signed_in_status_names_the_user_in_camel_case() {
        let status = AccountStatus {
            state: AccountState::SignedIn,
            user: Some(AccountUser {
                id: "user_1".into(),
                email: "ada@example.com".into(),
                name: "Ada Lovelace".into(),
                avatar_url: Some("https://example.com/a.png".into()),
                organization_id: None,
            }),
            error: None,
            available: true,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({
                "state": "signed_in",
                "user": { "id": "user_1", "email": "ada@example.com", "name": "Ada Lovelace", "avatarUrl": "https://example.com/a.png" },
                "available": true,
            })
        );
    }
}
