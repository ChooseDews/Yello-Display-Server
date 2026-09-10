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

#[derive(Clone)]
pub struct HaClient {
    url: String,
    token: String,
    client: reqwest::Client,
}

impl HaClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn configured(&self) -> bool {
        !self.url.is_empty() && !self.token.is_empty()
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

        let url = format!("{}/api/states/{}", self.url, entity_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(HaError::Error(format!("HTTP {}", resp.status())));
        }

        let bytes = resp.bytes().await.map_err(|e| HaError::Error(e.to_string()))?;
        let val: Value = serde_json::from_slice(&bytes).map_err(|e| HaError::Error(e.to_string()))?;
        Ok(val)
    }

    pub async fn list_states(&self) -> Result<Vec<HaState>, HaError> {
        if !self.configured() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }
        let url = format!("{}/api/states", self.url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| HaError::Error(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(HaError::Error(format!("HTTP {}", resp.status())));
        }
        let bytes = resp.bytes().await.map_err(|e| HaError::Error(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| HaError::Error(e.to_string()))
    }

    pub async fn toggle(&self, entity_id: &str) -> Result<(), HaError> {
        if !Self::is_valid_entity_id(entity_id) {
            return Err(HaError::Error(format!("invalid entity ID: {entity_id}")));
        }
        if !self.configured() {
            return Err(HaError::Error("Home Assistant is not configured".into()));
        }

        let domain = entity_id.split('.').next().unwrap_or("homeassistant");
        let service_domain = match domain {
            "light" | "switch" | "fan" | "input_boolean" => domain,
            _ => "homeassistant",
        };

        let url = format!("{}/api/services/{}/toggle", self.url, service_domain);
        let body = json!({ "entity_id": entity_id });
        let body_str = serde_json::to_string(&body).unwrap_or_default();

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
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
}
