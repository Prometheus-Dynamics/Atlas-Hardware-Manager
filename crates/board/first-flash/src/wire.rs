//! What goes to the board and what comes back, as Lemnos's bridge and `system.flash/1` define them (Lemnos next
//! 4b5be3a5, `docs/slots.md` "First flash"). No I/O here.
//!
//! - The image goes to `lemnos/<device>/<name>`, its transport manifest first to `<name>.json`.
//! - The committed image's answer is fields: `raw_bytes=<n> outcome=<code>`.
//! - `verify`'s result is the byte count checked, a `u64` little-endian, which the bridge passes as the run's output.

use atlas_flash_flow::{ImagePlan, Stream};

/// The bridge's file prefix for a device that takes files.
pub const SINK_PREFIX: &str = "lemnos/";
/// The stream name of an xz image, as the flasher's `LEMNOS_ORION_STREAMS` gives it.
pub const XZ_NAME: &str = "image.img.xz";
/// The stream name of a raw image.
pub const RAW_NAME: &str = "image.img";
/// The manifest's format word for a whole-disk image.
pub const FORMAT_DISK: &str = "disk";
/// The record's length in hex digits (108 bytes).
pub const RECORD_HEX: usize = 216;

/// The two destinations of one image on device `device`: the manifest's, then the image's.
#[must_use]
pub fn destinations(device: &str, stream: Stream) -> (String, String) {
    let name = match stream {
        Stream::Xz => XZ_NAME,
        Stream::Raw => RAW_NAME,
    };
    let image = format!("{SINK_PREFIX}{device}/{name}");
    (format!("{image}.json"), image)
}

/// The transport manifest, schema 1. `board`, `image.size` and `image.sha256` are the raw image's; `signature` is
/// the record, left out when the image is unsigned.
///
/// # Errors
///
/// When the plan's hash is not 64 lowercase hex digits or its record is not 216: the board would refuse the
/// manifest, so it is not sent.
pub fn manifest(image: &ImagePlan) -> Result<Vec<u8>, String> {
    if image.sha256.len() != 64 || !is_lower_hex(&image.sha256) {
        return Err(format!(
            "the image's SHA-256 {:?} is not 64 lowercase hex digits",
            image.sha256
        ));
    }
    let mut json = serde_json::json!({
        "schema": 1,
        "board": image.board,
        "format": FORMAT_DISK,
        "image": { "size": image.raw_size, "sha256": image.sha256 },
    });
    if let Some(record) = &image.signature {
        if record.len() != RECORD_HEX || !is_lower_hex(record) {
            return Err(format!(
                "the image's signature record is {} characters, not {RECORD_HEX} lowercase hex digits",
                record.len()
            ));
        }
        json["signature"] = serde_json::Value::String(record.clone());
    }
    Ok(json.to_string().into_bytes())
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What the board said about the image's signature (`system.flash/1`'s outcome code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `0`: signed by a key in the board's list, and the signature verifies.
    Verified,
    /// `1`: not checked, the image is unsigned.
    Unsigned,
    /// `2`: not checked, signed by a key the board does not know.
    UnknownKey,
    /// `none`: the job reported no outcome.
    NotReported,
}

impl Outcome {
    /// The board's words for it, as Lemnos's bridge said them before the fields.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Verified => "signature verified",
            Self::Unsigned => "not checked: unsigned",
            Self::UnknownKey => "not checked: signed by an unknown key",
            Self::NotReported => "not reported",
        }
    }
}

/// A committed image's answer, read from its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// The raw bytes the board wrote.
    pub raw_bytes: u64,
    /// The signature's outcome.
    pub outcome: Outcome,
}

/// Reads a committed image's answer: `raw_bytes=<n> outcome=<code>`, space-separated. Fields this build does not
/// know are skipped (Lemnos adds new ones only after these).
///
/// # Errors
///
/// When `raw_bytes` or `outcome` is missing, given twice, or not a value the board gives.
pub fn committed(detail: &str) -> Result<Committed, String> {
    let mut raw_bytes = None;
    let mut outcome = None;
    for field in detail.split(' ').filter(|field| !field.is_empty()) {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key {
            "raw_bytes" => {
                let n = value
                    .parse::<u64>()
                    .map_err(|_| format!("the board's raw_bytes {value:?} is not a byte count"))?;
                if raw_bytes.replace(n).is_some() {
                    return Err("the board gave raw_bytes twice".to_owned());
                }
            }
            "outcome" => {
                let code = match value {
                    "0" => Outcome::Verified,
                    "1" => Outcome::Unsigned,
                    "2" => Outcome::UnknownKey,
                    "none" => Outcome::NotReported,
                    other => {
                        return Err(format!(
                            "the board's outcome {other:?} is not one Atlas knows"
                        ));
                    }
                };
                if outcome.replace(code).is_some() {
                    return Err("the board gave outcome twice".to_owned());
                }
            }
            _ => {}
        }
    }
    match (raw_bytes, outcome) {
        (Some(raw_bytes), Some(outcome)) => Ok(Committed { raw_bytes, outcome }),
        _ => Err(format!(
            "the board committed the image without raw_bytes and outcome (its answer: {detail:?}); its Lemnos is older than FLASH-BCM2712"
        )),
    }
}

/// Reads `verify`'s result: the byte count checked, a `u64` little-endian. The bridge passes a job's result as the
/// run's output as it is when it is UTF-8 and as hex otherwise, so eight bytes are the count and sixteen hex digits
/// are its hex.
///
/// # Errors
///
/// When the output is neither.
pub fn verified(output: &[u8]) -> Result<u64, String> {
    let bytes: [u8; 8] = match output.len() {
        8 => output.try_into().map_err(|_| "unreachable".to_owned())?,
        16 if output.iter().all(u8::is_ascii_hexdigit) => {
            let mut bytes = [0u8; 8];
            for (i, byte) in bytes.iter_mut().enumerate() {
                let pair =
                    std::str::from_utf8(&output[2 * i..2 * i + 2]).map_err(|e| e.to_string())?;
                *byte = u8::from_str_radix(pair, 16).map_err(|e| e.to_string())?;
            }
            bytes
        }
        n => {
            return Err(format!(
                "the board's verify answered {n} bytes, not a byte count"
            ));
        }
    };
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(signature: Option<String>) -> ImagePlan {
        ImagePlan {
            file: "image.img.xz".into(),
            board: "reference".into(),
            slot: "slot".into(),
            raw_size: 8_388_608,
            sha256: "ab".repeat(32),
            stream: Stream::Xz,
            signature,
        }
    }

    #[test]
    fn the_manifest_goes_first_beside_the_image_on_the_device() {
        assert_eq!(
            destinations("flash", Stream::Xz),
            (
                "lemnos/flash/image.img.xz.json".into(),
                "lemnos/flash/image.img.xz".into()
            )
        );
        assert_eq!(
            destinations("flash", Stream::Raw),
            (
                "lemnos/flash/image.img.json".into(),
                "lemnos/flash/image.img".into()
            )
        );
    }

    #[test]
    fn the_manifest_is_schema_one_with_the_record_only_when_signed() {
        let unsigned: serde_json::Value =
            serde_json::from_slice(&manifest(&plan(None)).unwrap()).unwrap();
        assert_eq!(
            unsigned,
            serde_json::json!({"schema":1,"board":"reference","format":"disk",
                "image":{"size":8_388_608,"sha256":"ab".repeat(32)}})
        );
        let record = "0f".repeat(108);
        let signed: serde_json::Value =
            serde_json::from_slice(&manifest(&plan(Some(record.clone()))).unwrap()).unwrap();
        assert_eq!(signed["signature"], serde_json::Value::String(record));
    }

    #[test]
    fn a_manifest_the_board_would_refuse_is_not_made() {
        assert!(manifest(&plan(Some("0f".into()))).is_err());
        assert!(manifest(&plan(Some("0F".repeat(108)))).is_err());
        let mut upper = plan(None);
        upper.sha256 = "AB".repeat(32);
        assert!(manifest(&upper).is_err());
    }

    #[test]
    fn every_outcome_is_read_from_the_fields() {
        for (code, outcome) in [
            ("0", Outcome::Verified),
            ("1", Outcome::Unsigned),
            ("2", Outcome::UnknownKey),
            ("none", Outcome::NotReported),
        ] {
            assert_eq!(
                committed(&format!("raw_bytes=8388608 outcome={code}")),
                Ok(Committed {
                    raw_bytes: 8_388_608,
                    outcome
                })
            );
        }
    }

    #[test]
    fn an_unknown_field_is_skipped_and_a_missing_one_refused() {
        assert_eq!(
            committed("raw_bytes=5 outcome=0 written_ms=12").map(|c| c.raw_bytes),
            Ok(5)
        );
        assert!(
            committed("").is_err(),
            "the stub's empty answer has no fields"
        );
        assert!(committed("8388608 bytes written; signature verified").is_err());
        assert!(committed("raw_bytes=5").is_err());
        assert!(committed("raw_bytes=5 outcome=3").is_err());
        assert!(committed("raw_bytes=5 raw_bytes=6 outcome=0").is_err());
        assert!(committed("raw_bytes=-1 outcome=0").is_err());
    }

    #[test]
    fn verify_s_count_reads_as_bytes_or_as_the_bridge_s_hex() {
        let n: u64 = 8_388_608;
        assert_eq!(verified(&n.to_le_bytes()), Ok(n));
        let hex: String = n.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(verified(hex.as_bytes()), Ok(n));
        assert!(verified(b"").is_err());
        assert!(verified(b"zzzzzzzzzzzzzzzz").is_err());
    }
}
