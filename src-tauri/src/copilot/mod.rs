//! GitHub Copilot as a summary and chat provider (#227), reached through the
//! user's Copilot subscription: Humla signs in with its own OAuth app and
//! drives the Copilot CLI through `github-copilot-sdk`.

pub mod auth;
pub mod cli;
pub mod session;

use std::path::{Path, PathBuf};

use github_copilot_sdk::Client;

use crate::keychain;

pub use session::Runtime;

/// The Keychain account holding [`auth::Credentials`] as JSON.
pub const CREDENTIALS_ACCOUNT: &str = "copilot_credentials";

/// Used when no model is chosen: Copilot picks one the account may use.
pub const DEFAULT_MODEL: &str = "auto";

pub const SIGNED_OUT: &str = "GitHub Copilot is signed out. Sign in in Settings.";
pub const NO_CLI: &str = "The Copilot CLI isn't installed. Download it in Settings.";

pub fn read_credentials(cache: &keychain::Cache) -> Result<Option<auth::Credentials>, String> {
    Ok(keychain::read(cache, CREDENTIALS_ACCOUNT)?.and_then(|json| serde_json::from_str(&json).ok()))
}

pub fn write_credentials(cache: &keychain::Cache, creds: Option<&auth::Credentials>) -> Result<(), String> {
    let json = creds.map(|c| serde_json::to_string(c).expect("credentials serialize"));
    keychain::write(cache, CREDENTIALS_ACCOUNT, json.as_deref())
}

impl Runtime {
    /// A GitHub token good for the next call, refreshed and stored when needed.
    pub async fn token(&self, cache: &keychain::Cache) -> anyhow::Result<String> {
        let _refreshing = self.refreshing.lock().await;
        let before = read_credentials(cache).map_err(anyhow::Error::msg)?;
        let mut creds = before.clone();
        let result = auth::fresh_token(&auth::GitHub, &mut creds, auth::now_ms()).await;
        if creds != before {
            write_credentials(cache, creds.as_ref()).map_err(anyhow::Error::msg)?;
        }
        match result {
            Ok(token) => Ok(token),
            Err(auth::TokenError::SignedOut) => anyhow::bail!(SIGNED_OUT),
            Err(auth::TokenError::Unreachable(e)) => anyhow::bail!("{e}"),
        }
    }

    /// The CLI's path, looked for once per launch and again after a miss.
    pub async fn cli(&self, app_data: &Path) -> Option<PathBuf> {
        let mut known = self.cli_path.lock().await;
        if let Some(p) = known.as_ref().filter(|p| p.is_file()) {
            return Some(p.clone());
        }
        *known = cli::locate(app_data).await;
        known.clone()
    }

    pub async fn forget_cli(&self) {
        *self.cli_path.lock().await = None;
    }

    /// A running client signed in as the user.
    pub async fn ready_client(&self, cache: &keychain::Cache, app_data: &Path) -> anyhow::Result<Client> {
        let token = self.token(cache).await?;
        let cli = self.cli(app_data).await.ok_or_else(|| anyhow::anyhow!(NO_CLI))?;
        self.client(&cli, &token).await
    }
}
