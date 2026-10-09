//! The arguments of an `update` action.

use std::collections::BTreeMap;

use orion_control_plane::{TypedConfigValue, update_action};

/// The arguments of an `update` action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRequest {
    pub image_url: String,
    pub sha256: String,
    pub size: u64,
}

impl StageRequest {
    pub fn from_args(args: &BTreeMap<String, TypedConfigValue>) -> Result<Self, String> {
        let text = |key: &str| match args.get(key) {
            Some(TypedConfigValue::String(value)) if !value.is_empty() => Ok(value.clone()),
            _ => Err(format!("`{key}` (string) is required")),
        };
        let image_url = text(update_action::ARG_IMAGE_URL)?;
        if !(image_url.starts_with("http://") || image_url.starts_with("https://")) {
            return Err("`image_url` must be an http:// or https:// URL".into());
        }
        let sha256 = text(update_action::ARG_SHA256)?.to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("`sha256` must be 64 hex digits".into());
        }
        let size = match args.get(update_action::ARG_SIZE) {
            Some(TypedConfigValue::UInt(size)) if *size > 0 => *size,
            Some(TypedConfigValue::Int(size)) if *size > 0 => size.unsigned_abs(),
            _ => return Err("`size` (uint, bytes) is required".into()),
        };
        Ok(Self {
            image_url,
            sha256,
            size,
        })
    }
}
