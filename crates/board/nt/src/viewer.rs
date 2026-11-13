use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::time::Duration;

use orion_nt4::{
    Client, ClientConfig, ClientEvent, ClientHandle, Publisher, SubscribeOptions, Subscription,
    TopicEvent, Value, type_id_for_name,
};

use crate::error::Error;
use crate::topic::{Sample, Topic, Update, merge_properties};

/// The longest wait between two reconnect attempts. A server that is gone is tried again at
/// least this often, until the viewer is dropped.
pub const RECONNECT_MAX: Duration = Duration::from_secs(5);

/// Where a viewer connects, and the name it asks for (`/nt/<name>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerConfig {
    /// The server's host name or address.
    pub host: String,
    /// The server's port (5810 by default).
    pub port: u16,
    /// The client name the server sees.
    pub name: String,
}

impl ViewerConfig {
    /// A config for `host` on the default NT4 port, under the name `atlas`.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: crate::DEFAULT_PORT,
            name: "atlas".to_owned(),
        }
    }
}

/// A connection to one NT4 server, with its topics.
///
/// The viewer subscribes to every topic under `/`. Its table follows the server's announces,
/// property changes and values; [`Viewer::edit`] publishes a value on an announced topic.
/// Read it with [`Viewer::next`], which waits for the next change.
pub struct Viewer {
    client: Client,
    handle: ClientHandle,
    topics: Subscription,
    table: BTreeMap<String, Topic>,
    /// One publication per topic the viewer has written, with the type it was made with.
    publishers: HashMap<String, (String, Publisher)>,
    pending: VecDeque<Update>,
}

impl fmt::Debug for Viewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Viewer")
            .field("topics", &self.table.len())
            .field("publications", &self.publishers.len())
            .finish_non_exhaustive()
    }
}

impl Viewer {
    /// Starts connecting to the server on the current Tokio runtime. Connecting, and every
    /// reconnect after a failure, happens in the background.
    pub async fn connect(config: ViewerConfig) -> Result<Self, Error> {
        let mut client_config = ClientConfig::new(config.host, config.name);
        client_config.port = config.port;
        client_config.reconnect_max = RECONNECT_MAX;
        let client = Client::start(client_config);
        let handle = client.handle();
        let topics = handle.subscribe(
            &["/"],
            SubscribeOptions {
                prefix: true,
                ..SubscribeOptions::default()
            },
        )?;
        Ok(Self {
            client,
            handle,
            topics,
            table: BTreeMap::new(),
            publishers: HashMap::new(),
            pending: VecDeque::new(),
        })
    }

    /// Waits for the next change and returns it. Returns `None` only if the connection
    /// machinery has stopped, which it does not do while the viewer lives.
    pub async fn next(&mut self) -> Option<Update> {
        loop {
            if let Some(update) = self.pending.pop_front() {
                return Some(update);
            }
            tokio::select! {
                event = self.client.next_event() => {
                    self.on_connection(event?);
                }
                event = self.topics.next() => {
                    self.on_topic(event?);
                }
            }
        }
    }

    /// The topics the viewer holds, by name.
    pub fn topics(&self) -> impl Iterator<Item = &Topic> {
        self.table.values()
    }

    /// One topic by name.
    pub fn topic(&self, name: &str) -> Option<&Topic> {
        self.table.get(name)
    }

    /// Writes `value` to the announced topic `name`, as its announced type.
    ///
    /// Nothing is sent when the topic is not announced, or the value's type is not the topic's
    /// type. The write goes out at once when the connection is up and is kept until it is, so a
    /// write during a reconnect is not lost. The viewer does not change its own table: the
    /// server's value comes back as an update.
    pub async fn edit(&mut self, name: &str, value: Value) -> Result<(), Error> {
        let Some(topic) = self.table.get(name) else {
            return Err(Error::UnknownTopic(name.to_owned()));
        };
        let type_name = topic.type_name.clone();
        self.write(name, &type_name, value)
    }

    /// Writes `value` to `name` with the NT4 type `type_name`, for a topic the server has not
    /// announced yet. An NT4 server that accepts a topic from clients announces it only once a
    /// client has published it, so this is how a viewer first writes one. If the server already
    /// announces the topic, `type_name` must be its type.
    pub async fn publish_value(
        &mut self,
        name: &str,
        type_name: &str,
        value: Value,
    ) -> Result<(), Error> {
        if let Some(topic) = self.table.get(name)
            && topic.type_name != type_name
        {
            return Err(Error::WrongType {
                topic: name.to_owned(),
                announced: topic.type_name.clone(),
                asked: type_name.to_owned(),
            });
        }
        self.write(name, type_name, value)
    }

    fn write(&mut self, name: &str, type_name: &str, value: Value) -> Result<(), Error> {
        if type_id_for_name(type_name).is_none() {
            return Err(Error::UnsupportedType(type_name.to_owned()));
        }
        if !value.fits_type(type_name) {
            return Err(Error::TypeMismatch {
                topic: name.to_owned(),
                expected: type_name.to_owned(),
                got: value.default_type_name(),
            });
        }
        let current = self
            .publishers
            .get(name)
            .is_some_and(|(made, _)| made == type_name);
        if !current {
            let publisher = self.handle.publish(name, type_name)?;
            self.publishers
                .insert(name.to_owned(), (type_name.to_owned(), publisher));
        }
        if let Some((_, publisher)) = self.publishers.get(name) {
            publisher.set(value)?;
        }
        Ok(())
    }

    fn on_connection(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::Connected => self.pending.push_back(Update::Connection {
                connected: true,
                reason: None,
            }),
            ClientEvent::Disconnected { reason } => {
                let gone: Vec<String> = self.table.keys().cloned().collect();
                self.table.clear();
                for name in gone {
                    self.pending.push_back(Update::Unannounced(name));
                }
                self.pending.push_back(Update::Connection {
                    connected: false,
                    reason: Some(reason),
                });
            }
            ClientEvent::ProtocolWarning(message) => {
                self.pending.push_back(Update::Warning(message));
            }
            ClientEvent::TimeSync { .. } => {}
        }
    }

    fn on_topic(&mut self, event: TopicEvent) {
        let update = match event {
            TopicEvent::Announced(info) => {
                let topic = Topic {
                    name: info.name,
                    type_name: info.type_name,
                    properties: info.properties,
                    sample: None,
                };
                self.table.insert(topic.name.clone(), topic.clone());
                Update::Announced(topic)
            }
            TopicEvent::Unannounced { name, .. } => {
                self.table.remove(&name);
                Update::Unannounced(name)
            }
            TopicEvent::PropertiesChanged { name, update } => {
                let Some(topic) = self.table.get_mut(&name) else {
                    return;
                };
                merge_properties(&mut topic.properties, &update);
                Update::Properties { name, update }
            }
            TopicEvent::Value {
                name,
                timestamp_us,
                value,
                ..
            } => {
                let Some(topic) = self.table.get_mut(&name) else {
                    return;
                };
                let sample = Sample {
                    t_us: timestamp_us,
                    value,
                };
                topic.sample = Some(sample.clone());
                Update::Value { name, sample }
            }
        };
        self.pending.push_back(update);
    }
}
