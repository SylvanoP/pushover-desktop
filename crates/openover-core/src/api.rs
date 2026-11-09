use crate::device_name::validate_device_name;
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
    ///
    /// The password is sent once and is not stored by this client.
    pub async fn login(
        &self,
        email: &str,
        password: &str,
        twofa: Option<&str>,
    ) -> Result<LoginResult, ApiError> {
        let mut fields = vec![
            ("email", email.to_string()),
            ("password", password.to_string()),
        ];
        if let Some(code) = twofa.map(str::trim).filter(|code| !code.is_empty()) {
            fields.push(("twofa", code.to_string()));
        }

        let (status, text) = self.post_multipart("users/login.json", &fields).await?;
        if is_two_factor(status, &text) {
            return Err(ApiError::TwoFactorRequired);
        }
        let body: LoginJson = decode_success(status, &text)?;
        if body.status != 1 {
            return Err(ApiError::LoginFailed {
                detail: error_detail(&body.errors),
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
        validate_device_name(name)?;
        let (status, text) = self
            .post_multipart(
                "devices.json",
                &[
                    ("secret", secret.to_string()),
                    ("name", name.to_string()),
                    ("os", OpenClientOs::CODE.to_string()),
                ],
            )
            .await?;
        let body: DeviceJson = decode_success(status, &text)?;
        if body.status != 1 {
            return Err(ApiError::DeviceRegistrationFailed {
                detail: error_detail(&body.errors),
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
        let status = response.status();
        let text = response.text().await?;
        let body: MessagesResponse = decode_success(status, &text)?;
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
        let device_id = path_token(device_id)?;
        let (status, text) = self
            .post_multipart(
                &format!("devices/{device_id}/update_highest_message.json"),
                &[
                    ("secret", secret.to_string()),
                    ("message", highest_message_id.to_string()),
                ],
            )
            .await?;
        let body: ApiEnvelope = decode_success(status, &text)?;
        if body.status != 1 {
            return Err(ApiError::DeleteFailed {
                detail: error_detail(&body.errors),
            });
        }
        Ok(())
    }

    /// `POST /1/receipts/{receipt}/acknowledge.json`
    pub async fn acknowledge_emergency(&self, secret: &str, receipt: &str) -> Result<(), ApiError> {
        let receipt = path_token(receipt)?;
        let (status, text) = self
            .post_multipart(
                &format!("receipts/{receipt}/acknowledge.json"),
                &[("secret", secret.to_string())],
            )
            .await?;
        let body: ApiEnvelope = decode_success(status, &text)?;
        if body.status != 1 {
            return Err(ApiError::AcknowledgeFailed {
                detail: error_detail(&body.errors),
            });
        }
        Ok(())
    }

    /// Multipart form body, matching the Open Client curl examples (`--form-string`).
    async fn post_multipart(
        &self,
        path: &str,
        fields: &[(&str, String)],
    ) -> Result<(reqwest::StatusCode, String), ApiError> {
        let mut form = reqwest::multipart::Form::new();
        for (name, value) in fields {
            form = form.text((*name).to_string(), value.clone());
        }
        let response = self
            .client
            .post(format!("{}/{}", self.base, path.trim_start_matches('/')))
            .multipart(form)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        Ok((status, text))
    }
}

fn path_token(value: &str) -> Result<&str, ApiError> {
    if (1..=80).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(value)
    } else {
        Err(ApiError::InvalidParameter)
    }
}

fn decode_success<T: for<'de> Deserialize<'de>>(
    status: reqwest::StatusCode,
    text: &str,
) -> Result<T, ApiError> {
    if !status.is_success() {
        return Err(ApiError::HttpStatus {
            status: status.as_u16(),
            body: clip(text),
        });
    }
    Ok(serde_json::from_str(text)?)
}

fn is_two_factor(status: reqwest::StatusCode, text: &str) -> bool {
    if status == reqwest::StatusCode::PRECONDITION_FAILED {
        return true;
    }
    serde_json::from_str::<LoginJson>(text)
        .ok()
        .is_some_and(|body| body.status == 0 && looks_like_twofa(&body.errors))
}

fn looks_like_twofa(errors: &Option<serde_json::Value>) -> bool {
    let text = errors
        .as_ref()
        .map(|value| value.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    text.contains("two-factor")
        || text.contains("two factor")
        || text.contains("twofa")
        || text.contains("2fa")
}

fn error_detail(errors: &Option<serde_json::Value>) -> String {
    match errors {
        Some(value) => clip(&value.to_string()),
        None => "status 0".into(),
    }
}

fn clip(text: &str) -> String {
    const MAX: usize = 240;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let mut clipped: String = text.chars().take(MAX).collect();
        clipped.push('…');
        clipped
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
    use crate::types::{MessagesResponse, USER_AGENT};

    #[test]
    fn user_agent_matches_policy() {
        assert_eq!(USER_AGENT, "openover-desktop/0.1.0 (Rust; Open Client)");
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

    #[test]
    fn http_412_is_two_factor() {
        assert!(is_two_factor(reqwest::StatusCode::PRECONDITION_FAILED, ""));
        let body = r#"{"status":0,"errors":["two-factor authentication required"]}"#;
        assert!(is_two_factor(reqwest::StatusCode::OK, body));
        assert!(!is_two_factor(
            reqwest::StatusCode::OK,
            r#"{"status":0,"errors":{"password":["is incorrect"]}}"#
        ));
    }

    #[test]
    fn path_tokens_reject_slashes() {
        assert!(path_token("abc/def").is_err());
        assert!(path_token("zQie8WjzFTWkMz5CcGrUNK2t5rR9zGTsfYQ7HHGs").is_ok());
    }
}
