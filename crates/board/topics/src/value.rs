//! How a reading is typed and what it holds.
//!
//! A reading's kind is what the node's description says it is (Lemnos `describe 1`: `reading`, one
//! of number, range, switch, text, colour_array or three_axes). A sample's value is decoded from
//! its topic field's type. The two are separate: a description may say nothing about a reading, and
//! a sample may not fit it.

use orion::topics::FieldType;

/// A fact the node's description may or may not give. A missing one is [`Reported::NotReported`],
/// never an empty string and never a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reported<T> {
    /// The description gives it.
    Value(T),
    /// The description does not give it (yet).
    NotReported,
}

impl<T> Reported<T> {
    /// The value, when it is given.
    #[must_use]
    pub const fn get(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::NotReported => None,
        }
    }
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

/// What a reading is, as its description says (`describe 1`'s `reading` word). A word this crate
/// does not know is not a kind: the reading then reads as not reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingKind {
    /// A number.
    Number,
    /// A number with a range.
    Range,
    /// On or off.
    Switch,
    /// Text.
    Text,
    /// An array of colour components.
    ColourArray,
    /// One axis of a three-axis vector.
    ThreeAxes,
}

impl ReadingKind {
    /// The kind a description's word names, or `None` for a word this crate does not know.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "number" => Self::Number,
            "range" => Self::Range,
            "switch" => Self::Switch,
            "text" => Self::Text,
            "colour_array" => Self::ColourArray,
            "three_axes" => Self::ThreeAxes,
            _ => return None,
        })
    }
}

/// What an item is in its description: a value read, a control that is set, or an action run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A value the device reports.
    Value,
    /// A control: its value is set with `lemnos.<device>.set`.
    Control,
    /// An action: run, with no value.
    Action,
}

impl ItemKind {
    /// The kind a description's word names, or `None` for a word this crate does not know.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "value" => Self::Value,
            "control" => Self::Control,
            "action" => Self::Action,
            _ => return None,
        })
    }
}

/// An item's unit state (`describe 1`'s `unit_state`). Three states, and each is a fact the
/// description gives: `Unknown` is not the same as `Dimensionless`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    /// The unit is the string the description gives.
    Known,
    /// No unit applies: a count, a switch, a text, an action.
    Dimensionless,
    /// The device cannot say what its unit is.
    Unknown,
}

impl UnitState {
    /// The state a description's word names, or `None` for a word this crate does not know.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "known" => Self::Known,
            "dimensionless" => Self::Dimensionless,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

/// A value read from a sample.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    /// A number (a float, or an integer read as one).
    Number(f64),
    /// On or off.
    Switch(bool),
    /// Text.
    Text(String),
    /// An array of numbers.
    Numbers(Vec<f64>),
}

/// Decodes a sample's payload as a value of a field of type `ty`. `None` when the payload does not
/// fit the type, or the type is not shown (bytes, structured values, frame buffers).
#[must_use]
pub fn decode(ty: &FieldType, payload: &[u8]) -> Option<Scalar> {
    match ty {
        FieldType::Float => Some(Scalar::Number(f64::from_le_bytes(payload.try_into().ok()?))),
        FieldType::Int => Some(Scalar::Number(
            i64::from_le_bytes(payload.try_into().ok()?) as f64
        )),
        FieldType::Bool => match payload {
            [byte] => Some(Scalar::Switch(*byte != 0)),
            _ => None,
        },
        FieldType::Text => String::from_utf8(payload.to_vec()).ok().map(Scalar::Text),
        FieldType::FloatArray => {
            let (chunks, rest) = payload.as_chunks::<8>();
            if !rest.is_empty() {
                return None;
            }
            Some(Scalar::Numbers(
                chunks
                    .iter()
                    .map(|chunk| f64::from_le_bytes(*chunk))
                    .collect(),
            ))
        }
        _ => None,
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
            decode(&FieldType::Float, &bytes),
            Some(Scalar::Number(21.5))
        );
        assert_eq!(decode(&FieldType::Float, &bytes[..4]), None);
    }

    #[test]
    fn a_switch_is_one_byte_text_must_be_utf8_and_an_array_is_whole_floats() {
        assert_eq!(decode(&FieldType::Bool, &[1]), Some(Scalar::Switch(true)));
        assert_eq!(decode(&FieldType::Bool, &[0, 0]), None);
        assert_eq!(decode(&FieldType::Text, &[0xff]), None);
        let mut colour = Vec::new();
        for component in [0.1_f64, 0.2, 0.3] {
            colour.extend_from_slice(&component.to_le_bytes());
        }
        assert_eq!(
            decode(&FieldType::FloatArray, &colour),
            Some(Scalar::Numbers(vec![0.1, 0.2, 0.3]))
        );
        assert_eq!(decode(&FieldType::FloatArray, &colour[..7]), None);
    }

    #[test]
    fn a_description_word_names_its_kind_and_an_unknown_word_names_none() {
        assert_eq!(
            ReadingKind::from_word("three_axes"),
            Some(ReadingKind::ThreeAxes)
        );
        assert_eq!(
            ReadingKind::from_word("colour_array"),
            Some(ReadingKind::ColourArray)
        );
        assert_eq!(ReadingKind::from_word("dial"), None);
        assert_eq!(UnitState::from_word("maybe"), None);
        assert_eq!(
            UnitState::from_word("dimensionless"),
            Some(UnitState::Dimensionless)
        );
        assert_eq!(ItemKind::from_word("control"), Some(ItemKind::Control));
    }
}
