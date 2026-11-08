//! A board's Orion that knows this computer's operator under another key:
//! the board enrolled a key made by another data folder or an earlier
//! install. Atlas never enrolls or removes on a board by itself; the fix is
//! two commands run on the board, which [`OtherKey::message`] names.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use atlas_driver::DriverError;
use orion_client::remote::{OperatorIdentity, RemoteError};
use serde::Serialize;

use crate::remote::ACTIONS;

/// This computer's operator, refused by a board under another key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OtherKey {
    /// `operator:<name>`, as the board enrolled it.
    pub operator_id: String,
    /// This computer's key, `sha256:<32 hex>`.
    pub fingerprint: String,
    /// Run on the board: removes the enrolment (the other key).
    pub remove_command: String,
    /// Run on the board: enrolls this computer's key.
    pub enroll_command: String,
}

impl OtherKey {
    /// The refusal this computer's `identity` gets from a board that enrolled
    /// another key for its name.
    pub fn for_identity(identity: &OperatorIdentity) -> Self {
        let operator_id = identity.operator_id().to_string();
        let fingerprint = identity.fingerprint();
        Self {
            remove_command: format!("orionctl operators remove {operator_id}"),
            enroll_command: enroll_command(&operator_id, &fingerprint),
            operator_id,
            fingerprint,
        }
    }

    /// What Atlas tells the user: the key, the name the board enrolled, and
    /// the two commands for the board.
    pub fn message(&self) -> String {
        let name = self
            .operator_id
            .strip_prefix("operator:")
            .unwrap_or(&self.operator_id);
        format!(
            "This computer's Orion key ({}) isn't the one the board enrolled for {name}. On the board, run `{}`, then `{}`.",
            self.fingerprint, self.remove_command, self.enroll_command
        )
    }
}

/// The command an administrator runs on a board to enroll `operator_id`
/// under `fingerprint`, with the actions Atlas uses.
pub(crate) fn enroll_command(operator_id: &str, fingerprint: &str) -> String {
    format!(
        "orionctl operators enroll {operator_id} --fingerprint {fingerprint} {}",
        ACTIONS
            .map(|action| format!("--action '{action}'"))
            .join(" ")
    )
}

/// The typed prefix Orion puts on a refusal (its `OperatorRejection`).
const OTHER_KEY_PREFIX: &str = "[operator.other_key]";
const REMOVED_PREFIX: &str = "[operator.removed]";

/// Whether a board refused this computer's key because it has another key
/// enrolled for the operator name.
// TODO: switch to `orion_client::remote::OperatorRejection` (via
// `RemoteError::operator_rejection()`) on the next Orion bump. Atlas's
// Cargo.lock pins an older Orion, so the prefix and text matches stay until then.
pub(crate) fn is_other_key(error: &RemoteError) -> bool {
    matches!(error, RemoteError::Rejected(message)
        if message.starts_with(OTHER_KEY_PREFIX) || message.contains("not its enrolled key"))
}

/// Whether the board removed this computer's operator (the enrolment is gone,
/// not replaced by another key).
pub(crate) fn is_removed(error: &RemoteError) -> bool {
    matches!(error, RemoteError::Rejected(message) if message.starts_with(REMOVED_PREFIX))
}

/// What Atlas says when the board removed this computer's operator: the fix
/// is to enroll it again, with the command to run on the board.
pub(crate) fn removed_message(identity: &OperatorIdentity) -> String {
    let operator_id = identity.operator_id().to_string();
    format!(
        "This computer's operator was removed on the board. On the board, run `{}` to enroll it again.",
        enroll_command(&operator_id, &identity.fingerprint())
    )
}

/// Logs each refusal once per board, operator and key: a retry that's
/// refused again says nothing new.
pub struct PairingLog {
    seen: Mutex<HashSet<String>>,
    sink: Box<dyn Fn(&str) + Send + Sync>,
}

impl PairingLog {
    pub fn new(sink: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            seen: Mutex::new(HashSet::new()),
            sink: Box::new(sink),
        }
    }

    /// Logs that the board at `board` refused `key`, unless it already has.
    pub fn refused(&self, board: &str, key: &OtherKey) {
        let first = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(format!("{board} {} {}", key.operator_id, key.fingerprint));
        if first {
            (self.sink)(&format!(
                "board {board} refuses {} with key {}: it has another key enrolled for that name (an older data folder or install). Atlas doesn't change the board. On the board, run `{}`, then `{}`.",
                key.operator_id, key.fingerprint, key.remove_command, key.enroll_command
            ));
        }
    }
}

/// The boards that refused this computer's key, by URL, with the fix for
/// each. A board that accepts the key again comes off the list.
#[derive(Default)]
pub(crate) struct Refusals {
    by_board: Mutex<BTreeMap<String, OtherKey>>,
    log: Option<Arc<PairingLog>>,
}

impl Refusals {
    pub(crate) fn new(log: Option<Arc<PairingLog>>) -> Self {
        Self {
            by_board: Mutex::default(),
            log,
        }
    }

    /// The board at `board` refused `identity`: logged once, kept as the fix.
    pub(crate) fn refuse(&self, board: &str, identity: &OperatorIdentity) -> OtherKey {
        let key = OtherKey::for_identity(identity);
        if let Some(log) = &self.log {
            log.refused(board, &key);
        }
        self.lock().insert(board.to_owned(), key.clone());
        key
    }

    /// The error for a request to the board at `board` that failed with
    /// `error`, when the board refused this computer's operator: a refused key
    /// is logged and kept as the fix; a removed operator gets its own message.
    /// `None` for any other failure.
    pub(crate) fn rejection(
        &self,
        board: &str,
        identity: &OperatorIdentity,
        error: &RemoteError,
    ) -> Option<DriverError> {
        if is_other_key(error) {
            return Some(DriverError::Unsupported(
                self.refuse(board, identity).message(),
            ));
        }
        is_removed(error).then(|| DriverError::Unsupported(removed_message(identity)))
    }

    /// The board at `board` accepted this computer's key.
    pub(crate) fn accept(&self, board: &str) {
        self.lock().remove(board);
    }

    /// The fix for the board at `board`, when it refused this computer's key.
    pub(crate) fn get(&self, board: &str) -> Option<OtherKey> {
        self.lock().get(board).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, OtherKey>> {
        self.by_board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> OtherKey {
        OtherKey {
            operator_id: "operator:atlas-mathias-pc".into(),
            fingerprint: "sha256:0123456789abcdef0123456789abcdef".into(),
            remove_command: "orionctl operators remove operator:atlas-mathias-pc".into(),
            enroll_command: enroll_command(
                "operator:atlas-mathias-pc",
                "sha256:0123456789abcdef0123456789abcdef",
            ),
        }
    }

    #[test]
    fn the_other_key_refusal_is_recognised_and_the_rest_are_not() {
        let other = RemoteError::Rejected(
            "operator operator:atlas-mathias-pc signed with key sha256:ff, not its enrolled key"
                .into(),
        );
        assert!(is_other_key(&other));
        let unknown = RemoteError::Rejected(
            "operator operator:atlas-mathias-pc is not enrolled and may not perform x".into(),
        );
        assert!(!is_other_key(&unknown));
        assert!(!is_other_key(&RemoteError::Transport("reset".into())));
    }

    #[test]
    fn the_typed_prefix_is_recognised_and_removed_is_distinct() {
        let typed = RemoteError::Rejected(
            "[operator.other_key] operator operator:atlas-mathias-pc signed with key sha256:ff"
                .into(),
        );
        assert!(is_other_key(&typed));
        assert!(!is_removed(&typed));
        let removed = RemoteError::Rejected("[operator.removed] operator:atlas-mathias-pc".into());
        assert!(is_removed(&removed));
        assert!(!is_other_key(&removed));
        let identity = OperatorIdentity::generate("atlas-test").unwrap();
        let message = removed_message(&identity);
        assert!(message.starts_with("This computer's operator was removed on the board."));
        assert!(message.contains("orionctl operators enroll operator:atlas-test --fingerprint"));
    }

    #[test]
    fn the_message_names_the_key_the_name_and_both_commands() {
        let message = key().message();
        assert!(message.starts_with(
            "This computer's Orion key (sha256:0123456789abcdef0123456789abcdef) isn't the one the board enrolled for atlas-mathias-pc."
        ));
        assert!(message.contains("`orionctl operators remove operator:atlas-mathias-pc`"));
        assert!(message.contains("`orionctl operators enroll operator:atlas-mathias-pc --fingerprint sha256:0123456789abcdef0123456789abcdef --action 'locate'"));
    }

    #[test]
    fn each_refusal_is_logged_once_per_board_operator_and_key() {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = lines.clone();
        let log = PairingLog::new(move |line| sink.lock().unwrap().push(line.to_owned()));
        for _ in 0..3 {
            log.refused("orion+tcp://raze:9200", &key());
        }
        log.refused("orion+tcp://other:9200", &key());
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("refuses operator:atlas-mathias-pc"));
    }

    #[test]
    fn a_refused_board_keeps_its_fix_until_it_accepts_the_key() {
        let identity = OperatorIdentity::generate("atlas-test").unwrap();
        let refusals = Refusals::new(None);
        assert!(refusals.get("orion+tcp://raze:9200").is_none());
        let key = refusals.refuse("orion+tcp://raze:9200", &identity);
        assert_eq!(key.operator_id, "operator:atlas-test");
        assert_eq!(refusals.get("orion+tcp://raze:9200"), Some(key));
        refusals.accept("orion+tcp://raze:9200");
        assert!(refusals.get("orion+tcp://raze:9200").is_none());
    }
}
