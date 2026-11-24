//! The first-flash write as Lemnos's bridge takes it, over a [`FlasherBoard`]: the manifest pushed, then the image
//! streamed into `system.flash/1`'s write, its answer read as fields, `verify` run, and the restart asked for.

use std::io::Cursor;

use async_trait::async_trait;
use atlas_files::PushSource;
use atlas_flash_flow::{
    FirstFlash, ImagePlan, Say, StepError, WritePhase, WriteProgress, WriteReport,
};

use crate::wire;

/// The flasher board file's first-flash device (Lemnos `testing/boards/bcm2712-flasher.toml`, `id = "flash"`).
/// It names the image's file sink (`lemnos/<device>/<name>`) only: the calls are made by contract
/// (see [`VERIFY`] and [`REBOOT`]), never by this id.
pub const FLASH_DEVICE: &str = "flash";
/// `system.flash/1`'s `verify`: the job that checks the written image (Lemnos's contract method).
// board: Lemnos 214c0ca7 contracts/system-flash/src/lib.rs:63 (verify)
pub const VERIFY: &str = "system.flash.verify";
/// `system.reboot/1`'s `reboot`: the restart after the write.
// board: Lemnos 214c0ca7 contracts/system-reboot/src/lib.rs:12 (reboot)
pub const REBOOT: &str = "system.reboot.reboot";

/// How a call to the board went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// It was not sent: there is no session yet. Trying again is safe.
    NotSent(String),
    /// It may have been sent, and its answer is lost: the session broke, or the board went quiet.
    Lost(String),
    /// The board answered no, or its answer is not one Atlas reads. It will not change by itself.
    Refused(String),
    /// It was sent and ended as failed on the board. A restart ends the run that asked for it this
    /// way too ("the program that runs it went away"), so for a restart the board's state decides.
    Ended(String),
}

impl CallError {
    fn into_step(self) -> StepError {
        match self {
            Self::NotSent(words) | Self::Lost(words) => StepError::Again(words),
            Self::Refused(words) | Self::Ended(words) => StepError::Refused(words),
        }
    }
}

/// Bytes done of a total, as a push or a job reports them.
pub type Bytes<'a> = dyn Fn(u64, u64) + Send + Sync + 'a;

/// The two things the write needs of a flasher's node. [`crate::OrionBoard`] is the real one.
#[async_trait]
pub trait FlasherBoard: Send + Sync {
    /// Pushes `source` to `destination` and returns the board's words with its commit. A lost session is waited for
    /// inside the push; an `Err` is the end of it.
    async fn push(
        &self,
        destination: &str,
        source: Box<dyn PushSource>,
        progress: &Bytes<'_>,
    ) -> Result<String, StepError>;

    /// Runs the Lemnos method `name` (`system.<contract>.<method>`) with `words` through the bridge,
    /// on the device that carries its contract, and returns its output. `job` follows a long job to
    /// its end, with its bytes as progress.
    async fn call(
        &self,
        name: &str,
        words: &[&str],
        job: bool,
        progress: &Bytes<'_>,
    ) -> Result<Vec<u8>, CallError>;
}

/// The first-flash write through Lemnos's bridge on a flasher node.
#[derive(Debug)]
pub struct BridgeFlash<B> {
    board: B,
    flash: String,
}

impl<B: FlasherBoard> BridgeFlash<B> {
    /// The write on `board`, with the flasher board file's [`FLASH_DEVICE`] as the file sink's device.
    pub fn new(board: B) -> Self {
        Self::with_device(board, FLASH_DEVICE)
    }

    /// The write on `board`, with the board file's own device id for the file sink.
    pub fn with_device(board: B, flash: &str) -> Self {
        Self {
            board,
            flash: flash.to_owned(),
        }
    }

    /// The board this writes through.
    pub const fn board(&self) -> &B {
        &self.board
    }
}

#[async_trait]
impl<B: FlasherBoard> FirstFlash for BridgeFlash<B> {
    async fn write(
        &self,
        image: &ImagePlan,
        progress: &(dyn Fn(WriteProgress) + Send + Sync),
        say: &Say<'_>,
    ) -> Result<WriteReport, StepError> {
        let manifest = wire::manifest(image).map_err(StepError::Refused)?;
        let (manifest_to, image_to) = wire::destinations(&self.flash, image.stream);

        // The bridge reads the manifest when the image's push begins, so it goes first, every attempt.
        say(format!("sending the manifest to {manifest_to}"));
        self.board
            .push(&manifest_to, Box::new(Cursor::new(manifest)), &|_, _| {})
            .await?;

        let file = std::fs::File::open(&image.file).map_err(|error| {
            StepError::Refused(format!(
                "the image {} cannot be opened: {error}",
                image.file
            ))
        })?;
        say(format!("streaming {} to {image_to}", image.file));
        // Each piece goes to the board's write before the board takes the next, so the bytes the board took are the
        // bytes it wrote (of the file as sent: compressed for xz).
        let writing = |done, total| {
            progress(WriteProgress {
                phase: WritePhase::Writing,
                done,
                total,
            });
        };
        let answer = self.board.push(&image_to, Box::new(file), &writing).await?;
        let committed = wire::committed(&answer).map_err(StepError::Refused)?;
        if committed.raw_bytes != image.raw_size {
            return Err(StepError::Refused(format!(
                "the board wrote {} bytes of an image of {}",
                committed.raw_bytes, image.raw_size
            )));
        }
        let signature = committed.outcome.words();
        say(format!(
            "the board wrote {} bytes; {signature}",
            committed.raw_bytes
        ));

        say("reading the image back on the board".to_owned());
        let verifying = |done, total| {
            progress(WriteProgress {
                phase: WritePhase::Verifying,
                done,
                total,
            });
        };
        let output = self
            .board
            .call(VERIFY, &[], true, &verifying)
            .await
            .map_err(CallError::into_step)?;
        let verified = wire::verified(&output).map_err(StepError::Refused)?;
        if verified != image.raw_size {
            return Err(StepError::Refused(format!(
                "the board read back {verified} bytes of an image of {}",
                image.raw_size
            )));
        }
        say(format!(
            "the board read back {verified} bytes and they match"
        ));
        Ok(WriteReport {
            written: committed.raw_bytes,
            verified,
            signature: signature.to_owned(),
        })
    }

    async fn reboot(&self, say: &Say<'_>) -> Result<(), StepError> {
        match self.board.call(REBOOT, &[], false, &|_, _| {}).await {
            Ok(_) => Ok(()),
            // A restart takes the session, and the program that ran it, with it: an answer lost or
            // ended as failed after the request went is the restart. It is not asked again (that
            // would restart the new system); the watch of the boot that follows decides.
            Err(CallError::Lost(words) | CallError::Ended(words)) => {
                say(format!(
                    "the board's answer went with the restart ({words}); its boot is watched"
                ));
                Ok(())
            }
            Err(error) => Err(error.into_step()),
        }
    }
}
