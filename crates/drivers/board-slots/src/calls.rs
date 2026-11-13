//! The calls on a board's slots. Each is one short call through the actions, so a refusal comes
//! back with the board's words, verbatim.
//!
//! Retry safety:
//! - [`BoardSlots::list`] changes nothing: it may be made again.
//! - [`BoardSlots::set_default`], [`BoardSlots::pin`] and [`BoardSlots::set_keep`] set a value to
//!   the same value again: a repeat changes nothing, so they may be made again.
//! - [`BoardSlots::start_trial`] restarts the board: it is NEVER retried here. A call that did not
//!   come back is `Board(Unknown)`, and the caller checks the list before asking again.
//! - [`BoardSlots::remove`] is never retried here either: a removal that was sent and not
//!   answered may have happened. The caller checks the list.

use atlas_actions::{Action, ActionError, Arg, ArgKind, BoardActions, Reported};
use atlas_session::Handle;
use atlas_slots::{SlotList, check_remove};

use crate::SlotError;

/// Lemnos's `system.slots/1` method names, as the bridge names them. The list method is the only
/// one whose reply is an out buffer (`list`, L1/M8a); it is called in ONE place: [`BoardSlots::list`].
pub const LIST: &str = "system.slots.list";
pub const ACCEPT: &str = "system.slots.accept";
pub const INSTALL_FILE: &str = "system.slots.install-file";
pub const SET_DEFAULT: &str = "system.slots.set-default";
pub const TRIAL: &str = "system.slots.trial";
pub const REMOVE: &str = "system.slots.remove";
pub const PIN: &str = "system.slots.pin";
pub const SET_KEEP: &str = "system.slots.set-keep";

/// One short action the board names, with its argument kinds. Not reported as long, so it runs
/// as a call, not a job.
pub(crate) fn short(name: &str, args: &[(&str, ArgKind)]) -> Action {
    Action {
        name: name.to_owned(),
        label: Reported::NotReported,
        description: Reported::NotReported,
        args: Reported::Known(
            args.iter()
                .map(|(arg, kind)| Arg {
                    name: (*arg).to_owned(),
                    kind: kind.clone(),
                })
                .collect(),
        ),
        long_running: Reported::Known(false),
    }
}

/// The slots of one board, through its node's session.
#[derive(Clone, Debug)]
pub struct BoardSlots {
    actions: BoardActions,
    /// This board's id (the board file's `[board] id`). A bundle for another board is refused.
    board: String,
    /// Whether the board's offers include `system.slots/1`.
    offered: bool,
}

impl BoardSlots {
    /// The slots of the node `handle` keeps a session with. `board` is this board's id, and
    /// `offered` says whether its `hello.offers` carries the slot contract (see
    /// `atlas_slots::offers_slots`).
    #[must_use]
    pub fn new(handle: Handle, board: impl Into<String>, offered: bool) -> Self {
        Self {
            actions: BoardActions::new(handle),
            board: board.into(),
            offered,
        }
    }

    /// This board's id.
    #[must_use]
    pub fn board(&self) -> &str {
        &self.board
    }

    /// Whether the board offers slots at all.
    #[must_use]
    pub const fn offered(&self) -> bool {
        self.offered
    }

    pub(crate) fn require_offered(&self) -> Result<(), SlotError> {
        if self.offered {
            Ok(())
        } else {
            Err(SlotError::NoSlots)
        }
    }

    /// Reads the board's slot list. The one place `list` is called: a board's refusal of the
    /// reply (over 60 KiB, M8a) comes back as `Board(Refused)` with the board's words, never
    /// truncated and never retried.
    ///
    /// // LOCK: Lemnos M8a 5dcf1a78: the reply is an out buffer; the Atlas lock does not carry it yet.
    /// // LOCK: Lemnos L1 (paging, M8b): when the reply can be paged, this one call is what changes.
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`], [`SlotError::Board`] or [`SlotError::BadList`].
    pub async fn list(&self) -> Result<SlotList, SlotError> {
        self.require_offered()?;
        let done = self.actions.run(&short(LIST, &[]), &[]).await?;
        let text = String::from_utf8(done.output).map_err(|_| {
            SlotError::Board(ActionError::Failed(
                "the board's slot list is not text".into(),
            ))
        })?;
        Ok(SlotList::parse(&text)?)
    }

    /// Makes `id` the default slot. The board refuses a slot it does not hold as good, or for
    /// another board; its words are returned.
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`] or [`SlotError::Board`].
    pub async fn set_default(&self, id: &str, force: bool) -> Result<(), SlotError> {
        self.require_offered()?;
        self.actions
            .run(
                &short(
                    SET_DEFAULT,
                    &[("id", ArgKind::Text), ("force", ArgKind::Bool)],
                ),
                &[id.to_owned(), flag(force)],
            )
            .await?;
        Ok(())
    }

    /// Restarts the board into `id` as a trial. Atlas does not confirm it: the board confirms
    /// itself, or falls back. NOT retried here (see the module docs).
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`] or [`SlotError::Board`].
    pub async fn start_trial(
        &self,
        id: &str,
        keep_default: bool,
        keep_old: bool,
        force: bool,
    ) -> Result<(), SlotError> {
        self.require_offered()?;
        self.actions
            .run(
                &short(
                    TRIAL,
                    &[
                        ("id", ArgKind::Text),
                        ("keep_default", ArgKind::Bool),
                        ("keep_old", ArgKind::Bool),
                        ("force", ArgKind::Bool),
                    ],
                ),
                &[
                    id.to_owned(),
                    flag(keep_default),
                    flag(keep_old),
                    flag(force),
                ],
            )
            .await?;
        Ok(())
    }

    /// Removes `id`. Refused here, before the board is asked, when it is running, default, on
    /// trial, the only slot, or starred and `allow_starred` is not given. The board still has the
    /// last word: its own refusal comes back verbatim.
    ///
    /// # Errors
    ///
    /// [`SlotError::Removal`] (nothing sent), [`SlotError::NoSlots`], [`SlotError::Board`].
    pub async fn remove(&self, id: &str, allow_starred: bool) -> Result<(), SlotError> {
        self.require_offered()?;
        let list = self.list().await?;
        check_remove(&list, id, allow_starred)?;
        self.actions
            .run(&short(REMOVE, &[("id", ArgKind::Text)]), &[id.to_owned()])
            .await?;
        Ok(())
    }

    /// Stars (`pinned = true`) or unstars a version. Starred is the board's pin.
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`] or [`SlotError::Board`].
    pub async fn set_starred(&self, id: &str, starred: bool) -> Result<(), SlotError> {
        self.require_offered()?;
        self.actions
            .run(
                &short(PIN, &[("id", ArgKind::Text), ("pinned", ArgKind::Bool)]),
                &[id.to_owned(), flag(starred)],
            )
            .await?;
        Ok(())
    }

    /// Sets how many versions to keep (N; zero keeps all) and runs the board's housekeeping.
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`] or [`SlotError::Board`].
    pub async fn set_keep(&self, keep: u32) -> Result<(), SlotError> {
        self.require_offered()?;
        self.actions
            .run(
                &short(SET_KEEP, &[("keep", ArgKind::Int)]),
                &[keep.to_string()],
            )
            .await?;
        Ok(())
    }
}

fn flag(value: bool) -> String {
    if value { "true".into() } else { "false".into() }
}
