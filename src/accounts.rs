use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "SettingsInput")]
pub struct Settings {
    pub accounts: Vec<AccountConfig>,
    pub timezone: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    accounts: Vec<AccountConfig>,
    #[serde(default = "utc")]
    timezone: String,
}
fn utc() -> String {
    "UTC".into()
}
impl TryFrom<SettingsInput> for Settings {
    type Error = Error;
    fn try_from(raw: SettingsInput) -> Result<Self> {
        if raw.accounts.is_empty() {
            return Err(Error::input("At least one account is required"));
        }
        let mut ids = std::collections::HashSet::new();
        if raw.accounts.iter().any(|a| !ids.insert(&a.id)) {
            return Err(Error::input("Duplicate account ID"));
        }
        raw.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| Error::input("Unknown timezone"))?;
        Ok(Self {
            accounts: raw.accounts,
            timezone: raw.timezone,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "AccountInput")]
pub struct AccountConfig {
    pub id: String,
    pub url: Url,
    pub username: Option<String>,
    pub password_env: Option<String>,
    pub bearer_token_env: Option<String>,
    pub calendars: Vec<Url>,
    pub sharing_url: Option<Url>,
    pub default_calendar: Option<Url>,
    pub timeout_seconds: f64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountInput {
    id: String,
    url: String,
    username: Option<String>,
    password_env: Option<String>,
    bearer_token_env: Option<String>,
    #[serde(default)]
    calendars: Vec<String>,
    sharing_url: Option<String>,
    default_calendar: Option<String>,
    #[serde(default = "timeout")]
    timeout_seconds: f64,
}
fn timeout() -> f64 {
    30.0
}
pub fn resource_url(value: &str) -> Result<Url> {
    let u = Url::parse(value).map_err(|_| Error::input("Expected absolute HTTP(S) URL"))?;
    if !["http", "https"].contains(&u.scheme())
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(Error::input(
            "Expected credential-free HTTP(S) resource URL",
        ));
    }
    Ok(u)
}
impl TryFrom<AccountInput> for AccountConfig {
    type Error = Error;
    fn try_from(a: AccountInput) -> Result<Self> {
        if a.id.is_empty()
            || !a
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::input("Invalid account ID"));
        }
        if a.bearer_token_env.is_some() && (a.username.is_some() || a.password_env.is_some())
            || a.password_env.is_some() && a.username.is_none()
        {
            return Err(Error::input("Choose basic or bearer authentication"));
        }
        if !a.timeout_seconds.is_finite() || a.timeout_seconds <= 0.0 || a.timeout_seconds > 300.0 {
            return Err(Error::input("Timeout must be in (0, 300] seconds"));
        }
        let url = resource_url(&a.url)?;
        let same = |s: &str| -> Result<Url> {
            let u = resource_url(s)?;
            if u.origin() != url.origin() {
                return Err(Error::input("Account URLs must have the same origin"));
            }
            Ok(u)
        };
        let calendars = a.calendars.iter().map(|s| same(s)).collect::<Result<_>>()?;
        let sharing_url = a.sharing_url.as_deref().map(same).transpose()?;
        let default_calendar = a.default_calendar.as_deref().map(same).transpose()?;
        Ok(Self {
            id: a.id,
            url,
            username: a.username,
            password_env: a.password_env,
            bearer_token_env: a.bearer_token_env,
            calendars,
            sharing_url,
            default_calendar,
            timeout_seconds: a.timeout_seconds,
        })
    }
}
