//! What Atlas says when a board's Orion speaks another control protocol:
//! Orion takes only its own version, so one of the two needs updating.

use atlas_driver::DriverError;
use orion_client::remote::RemoteError;

/// Which side to update, from the two versions.
fn message(atlas: u16, board: u16) -> String {
    // Orion's protocol started over at 1 before its first release, so a
    // board reporting a higher number may be newer or a pre-release image.
    let update = if board < atlas {
        "update HeliOS on the board"
    } else {
        "update Atlas, or HeliOS on the board if its image predates Orion protocol 1"
    };
    format!("this board speaks Orion protocol {board}, Atlas speaks protocol {atlas}: {update}")
}

/// The error for `error`, when it is a protocol mismatch.
pub(crate) fn mismatch(error: &RemoteError) -> Option<DriverError> {
    match error {
        RemoteError::ProtocolMismatch { local, remote } => {
            Some(DriverError::Incompatible(message(*local, *remote)))
        }
        _ => None,
    }
}

/// `error` as Atlas reports an Orion that can't be reached at `url`.
pub(crate) fn unreachable(url: &str, error: &RemoteError) -> DriverError {
    mismatch(error).unwrap_or_else(|| DriverError::Unreachable(format!("Orion at {url}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_older_side_is_the_one_to_update() {
        let older_board = RemoteError::ProtocolMismatch {
            local: 2,
            remote: 1,
        };
        let Some(DriverError::Incompatible(text)) = mismatch(&older_board) else {
            panic!("a mismatch is incompatible");
        };
        assert_eq!(
            text,
            "this board speaks Orion protocol 1, Atlas speaks protocol 2: update HeliOS on the board"
        );
        let older_atlas = RemoteError::ProtocolMismatch {
            local: 1,
            remote: 2,
        };
        assert!(
            unreachable("orion+tcp://b:1", &older_atlas)
                .to_string()
                .contains(
                    "speaks Orion protocol 2, Atlas speaks protocol 1: update Atlas, or HeliOS"
                )
        );
    }
}
