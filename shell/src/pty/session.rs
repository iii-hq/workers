use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Attached,
    Detached,
    Exited {
        exit_code: Option<u32>,
        signal: Option<String>,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct SessionCredentials {
    pub access_key: String,
    pub reconnect_token: String,
}

#[derive(Debug)]
pub struct SessionControl {
    access_key: String,
    reconnect_token: String,
    owner_worker_id: String,
    output_function_id: String,
    status: SessionStatus,
    last_attach_request_id: Option<String>,
    last_attach_reconnect_token: Option<String>,
}

impl SessionControl {
    pub fn new(owner_worker_id: &str, output_function_id: &str) -> Self {
        let (access_key, reconnect_token) = new_credential_pair(new_credential);
        Self {
            access_key,
            reconnect_token,
            owner_worker_id: owner_worker_id.to_string(),
            output_function_id: output_function_id.to_string(),
            status: SessionStatus::Attached,
            last_attach_request_id: None,
            last_attach_reconnect_token: None,
        }
    }

    pub fn authenticate(&self, access_key: &str, caller_worker_id: &str) -> Result<(), String> {
        if self.access_key == access_key && self.owner_worker_id == caller_worker_id {
            Ok(())
        } else {
            Err("terminal session credentials are invalid".to_string())
        }
    }

    pub fn attach(
        &mut self,
        reconnect_token: &str,
        request_id: Option<&str>,
        owner_worker_id: &str,
        output_function_id: &str,
    ) -> Result<SessionCredentials, String> {
        if self.is_attach_retry(reconnect_token, request_id) {
            self.owner_worker_id = owner_worker_id.to_string();
            self.output_function_id = output_function_id.to_string();
            self.status = SessionStatus::Attached;
            return Ok(SessionCredentials {
                access_key: self.access_key.clone(),
                reconnect_token: self.reconnect_token.clone(),
            });
        }
        if self.reconnect_token != reconnect_token {
            return Err("terminal reconnect token is invalid".to_string());
        }

        let previous_reconnect_token = reconnect_token.to_string();
        let (access_key, reconnect_token) = new_credential_pair(new_credential);
        self.owner_worker_id = owner_worker_id.to_string();
        self.output_function_id = output_function_id.to_string();
        self.access_key = access_key;
        self.reconnect_token = reconnect_token;
        self.status = SessionStatus::Attached;
        self.last_attach_request_id = request_id.map(str::to_string);
        self.last_attach_reconnect_token = Some(previous_reconnect_token);

        Ok(SessionCredentials {
            access_key: self.access_key.clone(),
            reconnect_token: self.reconnect_token.clone(),
        })
    }

    pub fn detach(&mut self, access_key: &str, caller_worker_id: &str) -> Result<(), String> {
        self.authenticate(access_key, caller_worker_id)?;
        self.status = SessionStatus::Detached;
        Ok(())
    }

    pub fn detach_output_target(&mut self, output_function_id: &str, access_key: &str) -> bool {
        if self.status == SessionStatus::Attached
            && self.output_function_id == output_function_id
            && self.access_key == access_key
        {
            self.status = SessionStatus::Detached;
            true
        } else {
            false
        }
    }

    pub fn access_key(&self) -> &str {
        &self.access_key
    }

    pub fn reconnect_token(&self) -> &str {
        &self.reconnect_token
    }

    pub fn can_attach(&self, reconnect_token: &str, request_id: Option<&str>) -> bool {
        self.reconnect_token == reconnect_token || self.is_attach_retry(reconnect_token, request_id)
    }

    pub fn output_function_id(&self) -> &str {
        &self.output_function_id
    }

    pub fn status(&self) -> SessionStatus {
        self.status.clone()
    }

    fn is_attach_retry(&self, reconnect_token: &str, request_id: Option<&str>) -> bool {
        request_id.is_some()
            && self.last_attach_request_id.as_deref() == request_id
            && self.last_attach_reconnect_token.as_deref() == Some(reconnect_token)
    }
}

fn new_credential() -> String {
    Uuid::new_v4().simple().to_string()
}

fn new_credential_pair(mut source: impl FnMut() -> String) -> (String, String) {
    let access_key = source();
    let mut reconnect_token = source();
    while reconnect_token == access_key {
        reconnect_token = source();
    }
    (access_key, reconnect_token)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{new_credential_pair, SessionControl, SessionStatus};

    #[test]
    fn credential_pair_retries_duplicate_values() {
        let mut values = VecDeque::from([
            "duplicate".to_string(),
            "duplicate".to_string(),
            "distinct".to_string(),
        ]);

        let (access_key, reconnect_token) =
            new_credential_pair(|| values.pop_front().expect("test credential"));

        assert_eq!(access_key, "duplicate");
        assert_eq!(reconnect_token, "distinct");
    }

    #[test]
    fn successful_attach_rotates_both_credentials() {
        let mut control = SessionControl::new("owner-1", "output-1");
        let access = control.access_key().to_string();
        let reconnect = control.reconnect_token().to_string();

        let rotated = control
            .attach(&reconnect, None, "owner-2", "output-2")
            .unwrap();

        assert_ne!(rotated.access_key, access);
        assert_ne!(rotated.reconnect_token, reconnect);
        assert!(control
            .attach(&reconnect, None, "owner-3", "output-3")
            .is_err());
    }

    #[test]
    fn failed_attach_leaves_the_reconnect_token_usable() {
        let mut control = SessionControl::new("owner-1", "output-1");
        let reconnect = control.reconnect_token().to_string();

        assert!(control
            .attach("invalid-token", None, "owner-2", "output-2")
            .is_err());
        assert!(control
            .attach(&reconnect, None, "owner-2", "output-2")
            .is_ok());
    }

    #[test]
    fn stale_delivery_failure_does_not_detach_rebound_target() {
        let mut control = SessionControl::new("owner-1", "output-1");
        let old_access = control.access_key().to_string();
        let reconnect = control.reconnect_token().to_string();
        let rotated = control
            .attach(&reconnect, None, "owner-2", "output-1")
            .unwrap();

        assert!(!control.detach_output_target("output-1", &old_access));
        assert_eq!(control.status(), SessionStatus::Attached);
        assert!(control.detach_output_target("output-1", &rotated.access_key));
        assert_eq!(control.status(), SessionStatus::Detached);
    }
}
