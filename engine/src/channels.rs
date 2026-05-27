//! Channel Adapters — unified messaging interface for WhatsApp, Telegram, Slack.
//!
//! Each channel adapter normalizes incoming messages into a common ChannelMessage
//! format and sends outgoing responses via the platform's API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Supported channel types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    WhatsApp,
    Telegram,
    Slack,
    Discord,
    Webhook,
    Rest,
}

/// Content type of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Audio,
    Image,
    File,
}

/// Normalized incoming message from any channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub channel_type: ChannelType,
    pub sender_id: String,
    pub content_type: ContentType,
    pub content: String,
    pub timestamp: u64,
    #[serde(default)]
    pub metadata: Value,
}

/// Outgoing response to send via a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResponse {
    pub channel_type: ChannelType,
    pub recipient_id: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Value>,
}

/// Channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub channel_type: ChannelType,
    pub config: Value,
    pub universe_id: String,
}

/// Channel adapter trait — implemented by each platform.
#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn channel_type(&self) -> ChannelType;
    async fn send(&self, response: &ChannelResponse) -> Result<(), ChannelError>;
}

/// Channel errors.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel send failed: {0}")]
    SendFailed(String),
    #[error("channel not configured: {0}")]
    NotConfigured(String),
    #[error("invalid message format: {0}")]
    InvalidFormat(String),
}

// ---------------------------------------------------------------------------
// Webhook adapter (built-in, generic)
// ---------------------------------------------------------------------------

/// Generic webhook adapter — sends responses to a configured URL.
pub struct WebhookAdapter {
    pub url: String,
    pub client: reqwest::Client,
}

impl WebhookAdapter {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for WebhookAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Webhook
    }

    async fn send(&self, response: &ChannelResponse) -> Result<(), ChannelError> {
        let body = serde_json::to_value(response)
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        self.client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_message_serde() {
        let msg = ChannelMessage {
            channel_type: ChannelType::WhatsApp,
            sender_id: "+1234567890".into(),
            content_type: ContentType::Text,
            content: "Hello!".into(),
            timestamp: 1234567890,
            metadata: Value::Null,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChannelMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channel_type, ChannelType::WhatsApp);
        assert_eq!(back.content, "Hello!");
    }

    #[test]
    fn channel_response_serde() {
        let resp = ChannelResponse {
            channel_type: ChannelType::Telegram,
            recipient_id: "12345".into(),
            text: "Response".into(),
            attachments: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("telegram"));
    }

    #[test]
    fn channel_types_all_variants() {
        let types = vec![
            ChannelType::WhatsApp,
            ChannelType::Telegram,
            ChannelType::Slack,
            ChannelType::Discord,
            ChannelType::Webhook,
            ChannelType::Rest,
        ];
        for ct in types {
            let json = serde_json::to_string(&ct).unwrap();
            let back: ChannelType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ct);
        }
    }

    #[test]
    fn webhook_adapter_creation() {
        let adapter = WebhookAdapter::new("https://example.com/hook");
        assert_eq!(adapter.channel_type(), ChannelType::Webhook);
    }
}
