use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";
const DEFAULT_SKEW_SECONDS: u64 = 120;

#[derive(Parser, Debug)]
#[command(name = "codex-bridge")]
#[command(about = "Bridge Codex CLI OAuth tokens to other local tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print a fresh-ish access token to stdout.
    PrintToken {
        /// Refresh if token has <= this many seconds remaining.
        #[arg(long, default_value_t = DEFAULT_SKEW_SECONDS)]
        skew_seconds: u64,
    },
    /// Show token health metadata (never prints secrets).
    Status {
        #[arg(long, default_value_t = DEFAULT_SKEW_SECONDS)]
        skew_seconds: u64,
    },
    /// Force refresh and persist updated auth.json.
    Refresh,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CodexAuth {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(default)]
    last_refresh: Option<String>,
    #[serde(default)]
    tokens: Option<Tokens>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    exp: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::PrintToken { skew_seconds } => {
            let token = get_token(skew_seconds).await?;
            println!("{}", token);
        }
        Commands::Status { skew_seconds } => {
            let path = auth_path()?;
            let auth = read_auth(&path)?;
            let tokens = auth
                .tokens
                .ok_or_else(|| anyhow!("missing tokens in auth.json"))?;
            let now = now_unix();
            let exp = parse_jwt_exp(&tokens.access_token)?;
            let remaining = exp.saturating_sub(now);
            let state = if remaining <= skew_seconds {
                "expiring"
            } else {
                "valid"
            };
            println!("auth_file={}", path.display());
            println!(
                "auth_mode={}",
                auth.auth_mode.unwrap_or_else(|| "unknown".into())
            );
            println!("expires_at_unix={}", exp);
            println!("seconds_remaining={}", remaining);
            println!("state={}", state);
        }
        Commands::Refresh => {
            let path = auth_path()?;
            let mut auth = read_auth(&path)?;
            let tokens = auth
                .tokens
                .as_ref()
                .ok_or_else(|| anyhow!("missing tokens in auth.json"))?
                .clone();
            let refreshed = refresh_tokens(&tokens).await?;
            auth.tokens = Some(refreshed);
            auth.last_refresh = Some(now_unix().to_string());
            write_auth_atomic(&path, &auth)?;
            eprintln!("refreshed and saved {}", path.display());
        }
    }
    Ok(())
}

async fn get_token(skew_seconds: u64) -> Result<String> {
    let path = auth_path()?;
    let mut auth = read_auth(&path)?;
    let mut tokens = auth
        .tokens
        .clone()
        .ok_or_else(|| anyhow!("missing tokens in auth.json"))?;

    let now = now_unix();
    let exp = parse_jwt_exp(&tokens.access_token)?;
    let remaining = exp.saturating_sub(now);

    if remaining <= skew_seconds {
        match refresh_tokens(&tokens).await {
            Ok(updated) => {
                tokens = updated;
                auth.tokens = Some(tokens.clone());
                auth.last_refresh = Some(now_unix().to_string());
                write_auth_atomic(&path, &auth)?;
            }
            Err(err) => {
                bail!(
                    "token is expiring ({}s left) and refresh failed: {}. Run `codex login` then retry.",
                    remaining,
                    err
                );
            }
        }
    }

    Ok(tokens.access_token)
}

fn auth_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("CODEX_AUTH_FILE") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(home) = env::var("CODEX_HOME") {
        let p = PathBuf::from(home).join("auth.json");
        return Ok(p);
    }
    let home = env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn read_auth(path: &Path) -> Result<CodexAuth> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read auth file: {}", path.display()))?;
    let auth: CodexAuth = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse JSON: {}", path.display()))?;
    Ok(auth)
}

fn write_auth_atomic(path: &Path, auth: &CodexAuth) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(auth)?;
    fs::write(&tmp, payload).with_context(|| format!("failed writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed replacing {}", path.display()))?;
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_jwt_exp(token: &str) -> Result<u64> {
    let mut parts = token.split('.');
    let _header = parts.next().ok_or_else(|| anyhow!("invalid jwt"))?;
    let payload = parts.next().ok_or_else(|| anyhow!("invalid jwt"))?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("failed to decode jwt payload")?;
    let value: Value = serde_json::from_slice(&decoded).context("failed to parse jwt payload")?;
    let claims: JwtClaims = serde_json::from_value(value).context("jwt payload missing exp")?;
    claims.exp.ok_or_else(|| anyhow!("jwt has no exp"))
}

async fn refresh_tokens(tokens: &Tokens) -> Result<Tokens> {
    let client_id = env::var("CODEX_OAUTH_CLIENT_ID")
        .context("CODEX_OAUTH_CLIENT_ID is required for refresh")?;
    let refresh_url =
        env::var("CODEX_OAUTH_TOKEN_URL").unwrap_or_else(|_| DEFAULT_REFRESH_URL.to_string());

    let body = [
        ("grant_type", "refresh_token"),
        ("refresh_token", tokens.refresh_token.as_str()),
        ("client_id", client_id.as_str()),
    ];

    let client = Client::new();
    let resp = client.post(refresh_url).form(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("refresh request failed ({status}): {text}");
    }

    let payload: RefreshResponse = resp.json().await?;
    Ok(Tokens {
        access_token: payload.access_token,
        refresh_token: payload.refresh_token,
        id_token: tokens.id_token.clone(),
        account_id: tokens.account_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jwt_exp() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":2000000000}"#);
        let token = format!("{}.{}.sig", header, payload);
        let exp = parse_jwt_exp(&token).unwrap();
        assert_eq!(exp, 2_000_000_000);
    }

    #[test]
    fn auth_path_prefers_codex_auth_file() {
        unsafe { env::set_var("CODEX_AUTH_FILE", "/tmp/custom-auth.json") };
        let path = auth_path().unwrap();
        assert_eq!(path, PathBuf::from("/tmp/custom-auth.json"));
        unsafe { env::remove_var("CODEX_AUTH_FILE") };
    }
}
