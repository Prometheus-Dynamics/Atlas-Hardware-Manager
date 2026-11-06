//! The arguments of an `update` action.

use std::collections::BTreeMap;

use orion_control_plane::{TypedConfigValue, update_action};

/// The arguments of an `update` action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRequest {
    pub image_url: String,
    pub sha256: String,
    pub size: u64,
    /// On a slots board: install only (`apply = false`), or boot it once
    /// after (the default, as on an A/B board).
    pub apply: bool,
    /// On a slots board: keep the new slot from removal (`pin = true`).
    pub pin: bool,
    /// On a slots board: boot it once as a trial only (`keep_default =
    /// true`): it runs when it is healthy, but the default stays.
    pub keep_default: bool,
}

impl StageRequest {
    pub fn from_args(args: &BTreeMap<String, TypedConfigValue>) -> Result<Self, String> {
        let text = |key: &str| match args.get(key) {
            Some(TypedConfigValue::String(value)) if !value.is_empty() => Ok(value.clone()),
            _ => Err(format!("`{key}` (string) is required")),
        };
        let image_url = text(update_action::ARG_IMAGE_URL)?;
        // file://: a bundle uploaded to the board (HeliOS's
        // /v1/update/uploads, through a robot controller), read in place.
        // The writer keeps it to /data/updates/.
        let upload = image_url
            .strip_prefix("file:///data/updates/")
            .is_some_and(|rest| !rest.is_empty() && !rest.split('/').any(|part| part == ".."));
        if !(image_url.starts_with("http://") || image_url.starts_with("https://") || upload) {
            return Err(
                "`image_url` must be an http:// or https:// URL, or file:///data/updates/<upload>"
                    .into(),
            );
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
        let flag = |key: &str, default: bool| match args.get(key) {
            Some(TypedConfigValue::Bool(value)) => *value,
            _ => default,
        };
        Ok(Self {
            image_url,
            sha256,
            size,
            apply: flag("apply", true),
            pin: flag("pin", false),
            keep_default: flag("keep_default", false),
        })
    }

    /// The writer's restart after the install, if one is asked for.
    pub fn apply_args(&self) -> Option<Vec<&'static str>> {
        self.apply.then(|| {
            let mut args = vec!["apply"];
            if self.keep_default {
                args.push("--keep-default");
            }
            args
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trial_only_asks_the_writer_to_keep_the_default() {
        let mut args = BTreeMap::from([
            (
                update_action::ARG_IMAGE_URL.to_string(),
                TypedConfigValue::String("http://atlas:7700/new.img.xz".into()),
            ),
            (
                update_action::ARG_SHA256.to_string(),
                TypedConfigValue::String("0".repeat(64)),
            ),
            (
                update_action::ARG_SIZE.to_string(),
                TypedConfigValue::UInt(1),
            ),
        ]);
        let request = StageRequest::from_args(&args).unwrap();
        assert_eq!(request.apply_args(), Some(vec!["apply"]));
        args.insert("keep_default".into(), TypedConfigValue::Bool(true));
        let request = StageRequest::from_args(&args).unwrap();
        assert_eq!(request.apply_args(), Some(vec!["apply", "--keep-default"]));
        args.insert("apply".into(), TypedConfigValue::Bool(false));
        assert_eq!(StageRequest::from_args(&args).unwrap().apply_args(), None);
    }
}
