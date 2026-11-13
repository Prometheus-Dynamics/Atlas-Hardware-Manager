use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use orion_nt4::{ClientInfo, Properties, Server, ServerConfig, ServerEvent, ServerHandle, Value};
use tokio::sync::broadcast::{Receiver, error::RecvError};

use crate::error::Error;
use crate::topic::{Sample, Topic};

/// Where the local server listens, and where its persistent topics are kept.
#[derive(Clone, Debug)]
pub struct LocalServerConfig {
    /// The listen address. Port 0 takes a free port (see [`LocalServer::local_addr`]).
    pub bind: SocketAddr,
    /// A JSON file for the `persistent` topics, read on start and written on each change.
    /// `None` keeps nothing across restarts.
    pub persist_path: Option<PathBuf>,
}

impl Default for LocalServerConfig {
    /// `0.0.0.0:5810`, nothing persisted.
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], crate::DEFAULT_PORT)),
            persist_path: None,
        }
    }
}

/// What a [`LocalServer`] reports, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerUpdate {
    /// A client wrote a value. The server's table already holds it.
    Written {
        /// The client's connection id (see [`LocalServer::clients`]).
        client: u64,
        /// The topic.
        name: String,
        /// The value.
        value: Value,
        /// The server's clock when it was stored, in microseconds.
        t_us: i64,
    },
    /// Topics or clients changed, or events were missed: read [`LocalServer::topics`] and
    /// [`LocalServer::clients`] again.
    Changed,
    /// A client's message was rejected, or the persistence file could not be written.
    Warning(String),
}

/// The NT4 server Atlas runs itself, for bench use: a robot program's stand-in whose topics
/// the host creates and edits. Clients subscribe to them and write to them like to a robot.
///
/// Dropping it stops the listener and closes every connection.
pub struct LocalServer {
    server: Server,
    handle: ServerHandle,
    events: Receiver<ServerEvent>,
}

impl fmt::Debug for LocalServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalServer")
            .field("addr", &self.local_addr())
            .finish_non_exhaustive()
    }
}

impl LocalServer {
    /// Binds the listener and starts serving on the current Tokio runtime.
    pub async fn start(config: LocalServerConfig) -> Result<Self, Error> {
        let server = Server::start(ServerConfig {
            bind: config.bind,
            persist_path: config.persist_path,
            ..ServerConfig::default()
        })
        .await?;
        let handle = server.handle();
        let events = handle.events();
        Ok(Self {
            server,
            handle,
            events,
        })
    }

    /// The address the server listens on (the real port when the config asked for port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.server.local_addr()
    }

    /// Creates a topic with no value yet, or changes its type (the value is then reset).
    /// `persistent` keeps it across restarts when the config has a persistence file.
    pub async fn publish(
        &self,
        name: &str,
        type_name: &str,
        persistent: bool,
    ) -> Result<(), Error> {
        let mut properties = Properties::new();
        if persistent {
            properties.insert("persistent".to_owned(), serde_json::Value::Bool(true));
        }
        self.handle
            .publish(name, type_name, properties)
            .map_err(Error::from)
    }

    /// Sets a topic's value. A missing topic is created with the value's type. A value that
    /// does not fit the topic's type is refused and nothing changes.
    pub async fn set(&self, name: &str, value: Value) -> Result<(), Error> {
        self.handle.set_value(name, value).map_err(Error::from)
    }

    /// Deletes a topic and its value.
    pub async fn delete(&self, name: &str) -> Result<(), Error> {
        self.handle.delete(name).map_err(Error::from)
    }

    /// One topic by name.
    pub fn topic(&self, name: &str) -> Option<Topic> {
        self.handle.topic(name).map(|snapshot| Topic {
            name: snapshot.name,
            type_name: snapshot.type_name,
            properties: snapshot.properties,
            sample: snapshot
                .value
                .zip(snapshot.timestamp_us)
                .map(|(value, t_us)| Sample { t_us, value }),
        })
    }

    /// Every topic the server holds.
    pub fn topics(&self) -> Vec<Topic> {
        let mut names: Vec<String> = self.handle.topics().into_iter().map(|t| t.name).collect();
        names.sort();
        names.iter().filter_map(|name| self.topic(name)).collect()
    }

    /// The connected clients.
    pub fn clients(&self) -> Vec<ClientInfo> {
        self.handle.clients()
    }

    /// Waits for the next client write or change. Returns `None` only once the server has
    /// stopped. The host's own writes are not reported: the host knows them.
    pub async fn next_event(&mut self) -> Option<ServerUpdate> {
        loop {
            match self.events.recv().await {
                Ok(ServerEvent::ValueChanged {
                    name,
                    value,
                    timestamp_us,
                    client: Some(client),
                }) => {
                    return Some(ServerUpdate::Written {
                        client,
                        name,
                        value,
                        t_us: timestamp_us,
                    });
                }
                Ok(ServerEvent::ValueChanged { client: None, .. }) => {}
                Ok(
                    ServerEvent::ProtocolWarning { message, .. }
                    | ServerEvent::PersistFailed { message },
                ) => return Some(ServerUpdate::Warning(message)),
                Ok(_) | Err(RecvError::Lagged(_)) => return Some(ServerUpdate::Changed),
                Err(RecvError::Closed) => return None,
            }
        }
    }
}
