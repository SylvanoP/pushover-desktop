use crate::error::ApiError;
use crate::types::{
    API_BASE, ApiEnvelope, DeviceRegistration, LoginResult, Message, MessagesResponse,
    OpenClientOs, USER_AGENT,
};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PushoverClient {
    client: Client,
    base: String,
}

#[derive(Debug, Deserialize)]
struct LoginJson {
    status: i32,
    id: Option<String>,
    secret: Option<String>,
    errors: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DeviceJson {
    status: i32,
    id: Option<String>,
    errors: Option<serde_json::Value>,
}

impl PushoverClient {
    pub fn new() -> Result<Self, ApiError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            base: API_BASE.to_string(),
        })
    }

    /// `POST /1/users/login.json`
    pub async fn login(
        &self,
        email: &str,
        password: &str,
        twofa: Option<&str>,
    ) -> Result<LoginResult, ApiError> {
        let mut form = vec![
            ("email", email.to_string()),
            ("password", password.to_string()),
        ];
        if let Some(code) = twofa {
            form.push(("twofa", code.to_string()));
        }

        let response = self
            .client
            .post(format!("{}/users/login.json", self.base))
            .form(&form)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiError::TwoFactorRequired);
        }

        let body: LoginJson = response.error_for_status()?.json().await?;
        if body.status != 1 {
            return Err(ApiError::LoginFailed {
                detail: format!("{:?}", body.errors),
            });
        }
        let user_key = body.id.ok_or(ApiError::LoginFailed {
            detail: "missing user id".into(),
        })?;
        let secret = body.secret.ok_or(ApiError::LoginFailed {
            detail: "missing session secret".into(),
        })?;
        Ok(LoginResult { user_key, secret })
    }

    /// `POST /1/devices.json` with `os=O`.
    pub async fn register_device(
        &self,
        secret: &str,
        name: &str,
    ) -> Result<DeviceRegistration, ApiError> {
        let response = self
            .client
            .post(format!("{}/devices.json", self.base))
            .form(&[
                ("secret", secret.to_string()),
                ("name", name.to_string()),
                ("os", OpenClientOs::CODE.to_string()),
            ])
            .send()
            .await?;

        let body: DeviceJson = response.error_for_status()?.json().await?;
        if body.status != 1 {
            return Err(ApiError::DeviceRegistrationFailed {
                detail: format!("{:?}", body.errors),
            });
        }
        let device_id = body.id.ok_or(ApiError::DeviceRegistrationFailed {
            detail: "missing device id".into(),
        })?;
        Ok(DeviceRegistration { device_id })
    }

    /// `GET /1/messages.json`
    pub async fn fetch_messages(
        &self,
        secret: &str,
        device_id: &str,
    ) -> Result<Vec<Message>, ApiError> {
        let response = self
            .client
            .get(format!("{}/messages.json", self.base))
            .query(&[("secret", secret), ("device_id", device_id)])
            .send()
            .await?;

        let body: MessagesResponse = response.error_for_status()?.json().await?;
        if body.status != 1 {
            return Err(ApiError::MessagesFailed {
                detail: "status 0".into(),
            });
        }
        Ok(body.messages)
    }

    /// `POST /1/devices/{id}/update_highest_message.json`
    pub async fn delete_through(
        &self,
        secret: &str,
        device_id: &str,
        highest_message_id: u64,
    ) -> Result<(), ApiError> {
        let response = self
            .client
            .post(format!(
                "{}/devices/{}/update_highest_message.json",
                self.base, device_id
            ))
            .form(&[
                ("secret", secret.to_string()),
                ("message", highest_message_id.to_string()),
            ])
            .send()
            .await?;

        let body: ApiEnvelope = response.error_for_status()?.json().await?;
        if body.status != 1 {
            return Err(ApiError::DeleteFailed {
                detail: format!("{:?}", body.errors),
            });
        }
        Ok(())
    }
}

impl Default for PushoverClient {
    fn default() -> Self {
        Self::new().expect("HTTP client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_matches_policy() {
        assert!(USER_AGENT.contains("openover-desktop"));
        assert!(USER_AGENT.contains("Open Client"));
    }

    #[test]
    fn parses_messages_fixture() {
        let json = r#"{
            "status": 1,
            "messages": [
                {"id": 1, "message": "hello", "priority": 0, "title": "T"}
            ]
        }"#;
        let parsed: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].message, "hello");
    }
}
