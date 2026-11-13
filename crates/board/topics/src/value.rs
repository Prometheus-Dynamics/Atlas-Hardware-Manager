//! How a reading is typed and what it holds. A reading's type is its topic field's type; its
//! unit is the unit its field declares. Nothing else about it is assumed.

use orion::topics::FieldType;

/// A fact the node's description may or may not give. A missing one is [`Reported::NotReported`],
/// never an empty string and never a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reported<T> {
    /// The description gives it.
    Value(T),
    /// The description does not give it.
    NotReported,
}

impl Reported<String> {
    /// The text as given, or [`Reported::NotReported`] when it is empty.
    #[must_use]
    pub fn text(text: &str) -> Self {
        if text.is_empty() {
            Self::NotReported
        } else {
            Self::Value(text.to_owned())
        }
    }
}

/// How a reading is shown: its type, from the field it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingKind {
    /// A 64-bit float.
    Number,
    /// A 64-bit signed integer.
    Integer,
    /// A flag, on or off.
    Switch,
    /// UTF-8 text.
    Text,
    /// A type this crate does not show yet (bytes, arrays, structured values, frame buffers).
    Unsupported,
}

impl ReadingKind {
    /// The kind a field of type `ty` reads as.
    #[must_use]
    pub fn of(ty: &FieldType) -> Self {
        match *ty {
            FieldType::Float => Self::Number,
            FieldType::Int => Self::Integer,
            FieldType::Bool => Self::Switch,
            FieldType::Text => Self::Text,
            _ => Self::Unsupported,
        }
    }
}

/// A value read from a sample.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    /// A float.
    Number(f64),
    /// An integer.
    Integer(i64),
    /// A flag.
    Switch(bool),
    /// Text.
    Text(String),
}

/// Reads a sample's payload as a reading of `kind`. `None` when the payload does not fit the kind
/// (a short or badly coded sample), or the kind is not shown.
#[must_use]
pub fn decode(kind: ReadingKind, payload: &[u8]) -> Option<Scalar> {
    match kind {
        ReadingKind::Number => Some(Scalar::Number(f64::from_le_bytes(payload.try_into().ok()?))),
        ReadingKind::Integer => Some(Scalar::Integer(i64::from_le_bytes(
            payload.try_into().ok()?,
        ))),
        ReadingKind::Switch => match payload {
            [byte] => Some(Scalar::Switch(*byte != 0)),
            _ => None,
        },
        ReadingKind::Text => String::from_utf8(payload.to_vec()).ok().map(Scalar::Text),
        ReadingKind::Unsupported => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_text_is_not_reported() {
        assert_eq!(Reported::text(""), Reported::NotReported);
        assert_eq!(Reported::text("ok"), Reported::Value("ok".to_owned()));
    }

    #[test]
    fn a_float_sample_decodes_and_a_short_one_does_not() {
        let bytes = 21.5_f64.to_le_bytes();
        assert_eq!(
            decode(ReadingKind::Number, &bytes),
            Some(Scalar::Number(21.5))
        );
        assert_eq!(decode(ReadingKind::Number, &bytes[..4]), None);
    }

    #[test]
    fn a_switch_is_one_byte_and_text_must_be_utf8() {
        assert_eq!(
            decode(ReadingKind::Switch, &[1]),
            Some(Scalar::Switch(true))
        );
        assert_eq!(decode(ReadingKind::Switch, &[0, 0]), None);
        assert_eq!(decode(ReadingKind::Text, &[0xff]), None);
    }

    #[test]
    fn a_field_type_reads_as_its_kind() {
        assert_eq!(ReadingKind::of(&FieldType::Float), ReadingKind::Number);
        assert_eq!(ReadingKind::of(&FieldType::Bool), ReadingKind::Switch);
        assert_eq!(ReadingKind::of(&FieldType::Bytes), ReadingKind::Unsupported);
    }
}
