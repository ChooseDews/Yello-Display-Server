//! Home Assistant REST client.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

lazy_static::lazy_static! {
    static ref ENTITY_RE: Regex = Regex::new(r"^[a-z0-9_]+\.[a-z0-9_]+$").unwrap();
}

#[derive(Error, Debug, Clone)]
pub enum HaError {
    #[error("Home Assistant error: {0}")]
    Error(String),
}

pub type HomeAssistantError = HaError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaState {
    pub entity_id: String,
    pub state: String,
    pub attributes: HashMap<String, Value>,
    pub last_changed: Option<String>,
}

/// Home Assistant credentials come in two shapes: a long-lived access
/// token the user pastes in, or OAuth2 (HA's own sign-in flow), where the
/// server keeps a refresh token plus the client_id (the Studio origin the
/// user authorized from) and mints short-lived access tokens as needed.
#[derive(Clone)]
pub struct HaClient {
    url: String,
    token: String,
    refresh_token: String,
    client_id: String,
    access: std::sync::Arc<tokio::sync::RwLock<Option<(String, std::time::Instant)>>>,
    client: reqwest::Client,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Token-endpoint client: Home Assistant answers /auth/token directly, so
/// redirects are never followed. A redirect there means a proxy login page
/// (e.g. Authelia) is intercepting the address, and following it lands on an
/// HTML page that then fails JSON parsing with a cryptic message.
fn auth_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// Shown when the configured address answers with a web page instead of
/// Home Assistant's API (the classic public-hostname-behind-Authelia setup).
const PROXY_PAGE_MESSAGE: &str = "that address answered with a web page instead of Home Assistant's API \u{2014} it is probably behind a proxy login (such as Authelia). Use Home Assistant's direct address instead, e.g. http://192.168.1.247:8123";

/// Parses a Home Assistant JSON response, turning the "proxy login page
/// instead of JSON" failure into an explanation instead of serde's
/// "expected value at line 1".
async fn json_body(resp: reqwest::Response, what: &str) -> Result<Value, HaError> {
    let is_html = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/html"))
        .unwrap_or(false);
    let bytes = resp.bytes().await.map_err(|e| HaError::Error(e.to_string()))?;
    if is_html || bytes.first() == Some(&b'<') {
        return Err(HaError::Error(format!("Could not {what}: {PROXY_PAGE_MESSAGE}")));
    }
    serde_json::from_slice(&bytes).map_err(|e| HaError::Error(format!("Could not {what}: {e}")))
}

impl HaClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            refresh_token: String::new(),
            client_id: String::new(),
            access: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            client: http_client(),
        }
    }

    pub fn new_oauth(url: &str, refresh_token: &str, client_id: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token: String::new(),
            refresh_token: refresh_token.to_string(),
            client_id: client_id.to_string(),
            access: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            client: http_client(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn auth_method(&self) -> Option<&'static str> {
        if !self.refresh_token.is_empty() {
            Some("oauth")
        } else if !self.token.is_empty() {
            Some("token")
        } else {
            None
        }
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Server-side only: the stored long-lived token, so a settings save
    /// with a blank token field can keep it. Never sent to the browser.
    pub fn token_for_save(&self) -> String {
        self.token.clone()
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn configured(&self) -> bool {
        !self.url.is_empty() && self.auth_method().is_some()
    }

    /// Exchange an OAuth authorization code at the HA token endpoint.
    /// Returns (refresh_token, access_token, expires_in_seconds).
    pub async fn exchange_code(
        url: &str,
        code: &str,
        client_id: &str,
    ) -> Result<(String, String, u64), HaError> {
        let base = url.trim_end_matches('/');
        let resp = auth_client()
            .post(format!("{base}/auth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", client_id),
            ])
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;
        if !resp.status().is_success() {
            if resp.status().is_redirection() {
                return Err(HaError::Error(format!(
                    "Could not complete the sign-in: {PROXY_PAGE_MESSAGE}"
                )));
            }
            return Err(HaError::Error(format!(
                "Home Assistant rejected the sign-in code (HTTP {})",
                resp.status()
            )));
        }
        let val: Value = json_body(resp, "complete the sign-in").await?;
        let refresh = val
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HaError::Error("Home Assistant did not return a refresh token".into()))?
            .to_string();
        let access = val
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let expires = val.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(1800);
        Ok((refresh, access, expires))
    }

    /// Seed the access-token cache right after an exchange so the first
    /// API call does not immediately spend a refresh.
    pub async fn prime_access(&self, access_token: &str, expires_in: u64) {
        if access_token.is_empty() {
            return;
        }
        let expiry = std::time::Instant::now() + Duration::from_secs(expires_in.max(30));
        *self.access.write().await = Some((access_token.to_string(), expiry));
    }

    /// The bearer token for API calls: the pasted long-lived token, or a
    /// cached/refreshed OAuth access token.
    async fn bearer(&self) -> Result<String, HaError> {
        if !self.token.is_empty() {
            return Ok(self.token.clone());
        }
        if self.refresh_token.is_empty() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }
        {
            let cached = self.access.read().await;
            if let Some((token, expiry)) = cached.as_ref() {
                if *expiry > std::time::Instant::now() + Duration::from_secs(30) {
                    return Ok(token.clone());
                }
            }
        }
        let resp = self
            .client
            .post(format!("{}/auth/token", self.url))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", self.refresh_token.as_str()),
                ("client_id", self.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;
        if !resp.status().is_success() {
            if resp.status().is_redirection() {
                return Err(HaError::Error(format!(
                    "Could not refresh the Home Assistant session: {PROXY_PAGE_MESSAGE}"
                )));
            }
            return Err(HaError::Error(format!(
                "Home Assistant token refresh failed (HTTP {})",
                resp.status()
            )));
        }
        let val: Value = json_body(resp, "refresh the Home Assistant session").await?;
        let access = val
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HaError::Error("Home Assistant did not return an access token".into()))?
            .to_string();
        let expires = val.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(1800);
        let expiry = std::time::Instant::now() + Duration::from_secs(expires.max(30));
        *self.access.write().await = Some((access.clone(), expiry));
        Ok(access)
    }

    pub fn is_valid_entity_id(entity_id: &str) -> bool {
        ENTITY_RE.is_match(entity_id)
    }

    pub async fn get_state(&self, entity_id: &str) -> Result<Value, HaError> {
        if !Self::is_valid_entity_id(entity_id) {
            return Err(HaError::Error(format!("invalid entity ID: {entity_id}")));
        }
        if !self.configured() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }

        let token = self.bearer().await?;
        let url = format!("{}/api/states/{}", self.url, entity_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(HaError::Error(format!("HTTP {}", resp.status())));
        }

        json_body(resp, "read the entity state").await
    }

    pub async fn list_states(&self) -> Result<Vec<HaState>, HaError> {
        if !self.configured() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }
        let token = self.bearer().await?;
        let url = format!("{}/api/states", self.url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(HaError::Error(format!("HTTP {}", resp.status())));
        }
        let val = json_body(resp, "read Home Assistant states").await?;
        serde_json::from_value(val).map_err(|e| HaError::Error(e.to_string()))
    }

    /// Call an arbitrary Home Assistant service for one entity:
    /// POST /api/services/<domain>/<service> with {entity_id}.
    pub async fn call_service(&self, domain: &str, service: &str, entity_id: &str) -> Result<(), HaError> {
        if !Self::is_valid_entity_id(entity_id) {
            return Err(HaError::Error(format!("invalid entity ID: {entity_id}")));
        }
        if !ENTITY_RE.is_match(&format!("{domain}.{service}")) {
            return Err(HaError::Error(format!("invalid service: {domain}.{service}")));
        }
        if !self.configured() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }

        let url = format!("{}/api/services/{}/{}", self.url, domain, service);
        let body = json!({ "entity_id": entity_id });
        let body_str = serde_json::to_string(&body).unwrap_or_default();

        let token = self.bearer().await?;
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(HaError::Error(format!("HTTP {}", resp.status())));
        }

        Ok(())
    }

    pub async fn toggle(&self, entity_id: &str) -> Result<(), HaError> {
        if !Self::is_valid_entity_id(entity_id) {
            return Err(HaError::Error(format!("invalid entity ID: {entity_id}")));
        }
        let domain = entity_id.split('.').next().unwrap_or("homeassistant");
        let service_domain = match domain {
            "light" | "switch" | "fan" | "input_boolean" => domain,
            _ => "homeassistant",
        };
        self.call_service(service_domain, "toggle", entity_id).await
    }
}

/// Map a button / touch-zone service preset (see
/// `layout_model::HA_ACTION_SERVICES`) plus a target entity to the
/// concrete (domain, service) pair to call. None for an unknown preset.
pub fn ha_service_call(entity_id: &str, preset: &str) -> Option<(String, String)> {
    let domain = entity_id.split('.').next().unwrap_or("");
    let pair = match preset {
        "toggle" => {
            let service_domain = match domain {
                "light" | "switch" | "fan" | "input_boolean" => domain,
                _ => "homeassistant",
            };
            (service_domain, "toggle")
        }
        "turn_on" => ("homeassistant", "turn_on"),
        "turn_off" => ("homeassistant", "turn_off"),
        // button.press / input_button.press live on the entity's domain.
        "press" => (domain, "press"),
        "trigger" => ("automation", "trigger"),
        "scene" => ("scene", "turn_on"),
        _ => return None,
    };
    Some((pair.0.to_string(), pair.1.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_client_is_configured_without_a_pasted_token() {
        let client = HaClient::new_oauth("http://homeassistant.local:8123/", "refresh-123", "http://yello.local:8884");
        assert!(client.configured());
        assert_eq!(client.auth_method(), Some("oauth"));
        assert_eq!(client.url(), "http://homeassistant.local:8123");
        assert_eq!(client.client_id(), "http://yello.local:8884");
    }

    #[test]
    fn service_presets_map_to_domain_service_pairs() {
        assert_eq!(
            ha_service_call("light.kitchen", "toggle"),
            Some(("light".to_string(), "toggle".to_string()))
        );
        assert_eq!(
            ha_service_call("cover.garage", "toggle"),
            Some(("homeassistant".to_string(), "toggle".to_string()))
        );
        assert_eq!(
            ha_service_call("switch.porch", "turn_on"),
            Some(("homeassistant".to_string(), "turn_on".to_string()))
        );
        assert_eq!(
            ha_service_call("switch.porch", "turn_off"),
            Some(("homeassistant".to_string(), "turn_off".to_string()))
        );
        assert_eq!(
            ha_service_call("input_button.doorbell", "press"),
            Some(("input_button".to_string(), "press".to_string()))
        );
        assert_eq!(
            ha_service_call("automation.movie_night", "trigger"),
            Some(("automation".to_string(), "trigger".to_string()))
        );
        assert_eq!(
            ha_service_call("scene.evening", "scene"),
            Some(("scene".to_string(), "turn_on".to_string()))
        );
        assert_eq!(ha_service_call("light.kitchen", "explode"), None);
    }

    #[test]
    fn token_client_reports_token_method() {
        let client = HaClient::new("http://ha:8123", "llat");
        assert!(client.configured());
        assert_eq!(client.auth_method(), Some("token"));
        let empty = HaClient::new("", "");
        assert!(!empty.configured());
        assert_eq!(empty.auth_method(), None);
    }
}
