//! Names from the image are data, not paths: a slot id and every path inside
//! the image are checked component by component before anything is opened or
//! written, so no name can reach outside the scratch directory.

use crate::error::ImageError;

/// A slot id: 1 to 64 of A-Z, a-z, 0-9, '.', '_' or '-', not starting with '.'.
/// The same characters as a bundle's slot id, minus the dot names.
pub(crate) fn check_id(id: &str) -> Result<(), ImageError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(ImageError::InvalidName(format!(
            "slot id {id:?} must be 1 to 64 of A-Z, a-z, 0-9, '.', '_' or '-', not starting with '.'"
        )))
    }
}

/// One name inside the image: not empty, not `.` or `..`, no separators, no
/// control characters, at most 255 bytes.
pub(crate) fn check_component(name: &str) -> Result<(), ImageError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name
            .bytes()
            .any(|b| b == b'/' || b == b'\\' || b < 0x20 || b == 0x7f);
    if bad {
        Err(ImageError::InvalidName(format!(
            "{name:?} is not a plain file or directory name"
        )))
    } else {
        Ok(())
    }
}

/// A relative path inside the image, split into checked components.
pub(crate) fn split_rel(path: &str) -> Result<Vec<&str>, ImageError> {
    if path.is_empty() || path.starts_with('/') {
        return Err(ImageError::InvalidName(format!(
            "{path:?} must be a relative path"
        )));
    }
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts {
        check_component(part)?;
    }
    Ok(parts)
}

/// A configured pattern with its `{id}` filled in. The result is checked by
/// [`split_rel`] wherever it is used.
pub(crate) fn expand(pattern: &str, id: &str) -> String {
    pattern.replace("{id}", id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_ids_are_plain_names() {
        assert!(check_id("alpha").is_ok());
        assert!(check_id("a.b-c_9").is_ok());
        for bad in [
            "",
            ".",
            "..",
            ".hidden",
            "a/b",
            "a\\b",
            "a b",
            "é",
            &"x".repeat(65),
        ] {
            assert!(check_id(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn paths_inside_the_image_cannot_climb_out() {
        assert_eq!(split_rel("slots/alpha").unwrap(), vec!["slots", "alpha"]);
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "./a", "a/\0b", "a/b\n"] {
            assert!(split_rel(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(check_component("..").is_err());
        assert!(check_component("a\\b").is_err());
    }
}
