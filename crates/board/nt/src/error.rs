/// What the viewer and the local server can refuse or fail at.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server has not announced a topic of this name (or has withdrawn it).
    #[error("no topic named {0}: the server has not announced it")]
    UnknownTopic(String),
    /// The value's type does not fit the topic's type. Nothing was sent.
    #[error("topic {topic} has type {expected}, got a {got} value")]
    TypeMismatch {
        /// The topic.
        topic: String,
        /// The topic's NT4 type string.
        expected: String,
        /// The value's default NT4 type string.
        got: &'static str,
    },
    /// The server announces the topic with another type than the one asked for.
    #[error("topic {topic} is announced as {announced}, not {asked}")]
    WrongType {
        /// The topic.
        topic: String,
        /// The type the server announces.
        announced: String,
        /// The type asked for.
        asked: String,
    },
    /// The topic's type is one NT4 does not model, so no value can be checked against it.
    #[error("topic type {0} is not one NT4 models")]
    UnsupportedType(String),
    /// The NT4 layer failed (the listener could not bind, a client task has stopped, ...).
    #[error(transparent)]
    Nt(#[from] orion_nt4::Error),
}
