//! The real transport: Orion's remote operator client (signed orion+tcp,
//! `orion_client::remote`). Atlas connects as an operator, never a cluster
//! member. The node it connects to forwards actions and status queries to
//! the node that owns them.
//!
//! The connection is made on first use and dropped on a transport error,
//! so a node that restarts is picked up again on the next call. The node's
//! key is trusted on first use and then pinned (`pin_file`).

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use atlas_driver::DriverError;
use orion_client::remote::{NodeTrust, OperatorIdentity, RemoteError, RemoteOperator};
use orion_control_plane::{ActionRequest, ActionResult, NodeRecord, StatusEntry, StatusQuery};
use serde::Serialize;

use crate::transport::OrionTransport;

/// What the UI shows about the Orion connection.
#[derive(Clone, Debug, Default, Serialize)]
pub struct OrionConnection {
    pub url: Option<String>,
    /// `operator:<name>`: what an administrator enrolls.
    pub operator_id: String,
    /// `sha256:<32 hex>`, compared with what `orionctl operators list` shows.
    pub fingerprint: String,
    /// The command an administrator runs on a node to let Atlas in.
    pub enroll_command: String,
    pub connected: bool,
    pub enrolled: bool,
    pub node_id: Option<String>,
    pub node_fingerprint: Option<String>,
    pub error: Option<String>,
}

pub struct RemoteTransport {
    identity: OperatorIdentity,
    pin_file: Option<PathBuf>,
    url: Mutex<Option<String>>,
    session: tokio::sync::Mutex<Option<RemoteOperator>>,
    state: Mutex<OrionConnection>,
}

/// The actions Atlas runs through Orion; the enroll command grants these.
const ACTIONS: [&str; 3] = ["update", "reboot", "locate"];

impl RemoteTransport {
    pub fn new(identity: OperatorIdentity, url: Option<String>, pin_file: Option<PathBuf>) -> Self {
        let operator_id = identity.operator_id().to_string();
        let fingerprint = identity.fingerprint();
        let enroll_command = format!(
            "orionctl operators enroll {operator_id} --fingerprint {fingerprint} {}",
            ACTIONS
                .map(|action| format!("--action '{action}'"))
                .join(" ")
        );
        Self {
            identity,
            pin_file,
            state: Mutex::new(OrionConnection {
                url: url.clone(),
                operator_id,
                fingerprint,
                enroll_command,
                ..OrionConnection::default()
            }),
            url: Mutex::new(url),
            session: tokio::sync::Mutex::new(None),
        }
    }

    /// Points Atlas at another node (or none); the next call reconnects.
    pub async fn set_url(&self, url: Option<String>) {
        *self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = url.clone();
        *self.session.lock().await = None;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.url = url;
        state.connected = false;
        state.enrolled = false;
        state.node_id = None;
        state.node_fingerprint = None;
        state.error = None;
        // A new node has its own key: trust it on first use again.
        if let Some(pin) = &self.pin_file {
            let _ = std::fs::remove_file(pin);
        }
    }

    pub fn connection(&self) -> OrionConnection {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Shared-key enrollment, for nodes built with an enrollment key.
    pub async fn enroll_with_key(&self, key: &str) -> Result<(), DriverError> {
        let session = self.session().await?;
        let result = session.enroll_with_key(key.as_bytes()).await;
        self.record(&result);
        result.map(|_| ()).map_err(|error| self.error(&error))
    }

    fn trust(&self) -> NodeTrust {
        self.pin_file
            .as_ref()
            .and_then(|pin| std::fs::read(pin).ok())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .map_or(NodeTrust::FirstUse, NodeTrust::Key)
    }

    async fn session(&self) -> Result<RemoteOperator, DriverError> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            return Ok(session.clone());
        }
        let url = self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| DriverError::Unsupported("Orion isn't set up in Atlas".into()))?;
        let connected = RemoteOperator::connect(&url, self.identity.clone(), self.trust()).await;
        let session = match connected {
            Ok(session) => session,
            Err(error) => {
                let message = error.to_string();
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.connected = false;
                state.error = Some(message.clone());
                return Err(DriverError::Unreachable(format!(
                    "Orion at {url}: {message}"
                )));
            }
        };
        if let Some(pin) = &self.pin_file
            && !pin.exists()
        {
            let _ = std::fs::write(pin, session.node_public_key());
        }
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.connected = true;
            state.enrolled = session.is_enrolled();
            state.node_id = Some(session.node_id().to_string());
            state.node_fingerprint = Some(session.node_fingerprint());
            state.error = None;
        }
        *guard = Some(session.clone());
        Ok(session)
    }

    fn record<T>(&self, result: &Result<T, RemoteError>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok(_) => {
                state.enrolled = true;
                state.error = None;
            }
            Err(error) if error.is_not_enrolled() => {
                state.enrolled = false;
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    fn error(&self, error: &RemoteError) -> DriverError {
        if error.is_not_enrolled() {
            let command = self.connection().enroll_command;
            return DriverError::Unsupported(format!(
                "Atlas isn't enrolled with Orion yet; on the device run: {command}"
            ));
        }
        DriverError::Unreachable(format!("Orion: {error}"))
    }

    /// Runs one call, dropping the session after a failure so the next call
    /// reconnects (the node may have restarted).
    async fn call<T, F, Fut>(&self, work: F) -> Result<T, DriverError>
    where
        F: FnOnce(RemoteOperator) -> Fut,
        Fut: std::future::Future<Output = Result<T, RemoteError>>,
    {
        let session = self.session().await?;
        let result = work(session).await;
        self.record(&result);
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if !error.is_not_enrolled() {
                    *self.session.lock().await = None;
                    self.state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .connected = false;
                }
                Err(self.error(&error))
            }
        }
    }
}

#[async_trait]
impl OrionTransport for RemoteTransport {
    fn configured(&self) -> bool {
        self.url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError> {
        self.call(|session| async move { session.nodes().await })
            .await
    }

    async fn status(&self, query: StatusQuery) -> Result<Vec<StatusEntry>, DriverError> {
        self.call(|session| async move { session.status(query).await })
            .await
    }

    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError> {
        self.call(|session| async move { session.run_action(request).await })
            .await
    }

    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError> {
        let id = action_id.to_string();
        self.call(|session| async move { session.query_action(&id).await })
            .await
    }
}
