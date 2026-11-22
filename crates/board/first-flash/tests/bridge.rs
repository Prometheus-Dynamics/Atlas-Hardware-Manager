//! The first-flash write against a board that answers as Lemnos's bridge does (next da86d0b3).

use std::io::Read;
use std::sync::Mutex;

use async_trait::async_trait;
use atlas_files::PushSource;
use atlas_first_flash::{BridgeFlash, Bytes, CallError, FlasherBoard};
use atlas_flash_flow::{FirstFlash, ImagePlan, StepError, Stream, WritePhase, WriteProgress};

const RAW: u64 = 8_388_608;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Push {
        destination: String,
        bytes: Vec<u8>,
    },
    Call {
        device: String,
        words: Vec<String>,
        job: bool,
    },
}

struct Board {
    seen: Mutex<Vec<Seen>>,
    committed: String,
    verify: Result<Vec<u8>, CallError>,
    reboot: Result<Vec<u8>, CallError>,
}

impl Board {
    fn answering(committed: &str) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            committed: committed.to_owned(),
            verify: Ok(format!("raw_bytes={RAW}").into_bytes()),
            reboot: Ok(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl FlasherBoard for Board {
    async fn push(
        &self,
        destination: &str,
        mut source: Box<dyn PushSource>,
        progress: &Bytes<'_>,
    ) -> Result<String, StepError> {
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).unwrap();
        let size = bytes.len() as u64;
        progress(size / 2, size);
        progress(size, size);
        let manifest = destination.ends_with(".json");
        self.seen.lock().unwrap().push(Seen::Push {
            destination: destination.to_owned(),
            bytes,
        });
        Ok(if manifest {
            String::new()
        } else {
            self.committed.clone()
        })
    }

    async fn call(
        &self,
        device: &str,
        words: &[&str],
        job: bool,
        progress: &Bytes<'_>,
    ) -> Result<Vec<u8>, CallError> {
        self.seen.lock().unwrap().push(Seen::Call {
            device: device.to_owned(),
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            job,
        });
        if words == ["verify"] {
            progress(RAW, RAW);
            self.verify.clone()
        } else {
            self.reboot.clone()
        }
    }
}

fn image(dir: &tempfile::TempDir) -> ImagePlan {
    let file = dir.path().join("image.img.xz");
    std::fs::write(&file, b"the xz file as sent").unwrap();
    ImagePlan {
        file: file.display().to_string(),
        board: "reference".into(),
        slot: "reference-2026.1.0-dev.1".into(),
        raw_size: RAW,
        sha256: "5e".repeat(32),
        stream: Stream::Xz,
        signature: None,
    }
}

async fn write(
    board: &BridgeFlash<Board>,
    image: &ImagePlan,
) -> (
    Result<atlas_flash_flow::WriteReport, StepError>,
    Vec<WriteProgress>,
) {
    let progress = Mutex::new(Vec::new());
    let report = board
        .write(image, &|p| progress.lock().unwrap().push(p), &|_| {})
        .await;
    (report, progress.into_inner().unwrap())
}

#[tokio::test]
async fn the_manifest_then_the_image_then_verify_and_the_answer_is_the_board_s_fields() {
    let dir = tempfile::tempdir().unwrap();
    let image = image(&dir);
    let flash = BridgeFlash::new(Board::answering("raw_bytes=8388608 outcome=1"));
    let (report, progress) = write(&flash, &image).await;
    let report = report.expect("the write is done");
    assert_eq!(report.written, RAW);
    assert_eq!(report.verified, RAW);
    assert_eq!(report.signature, "not checked: unsigned");

    let seen = flash.board().seen();
    let Seen::Push { destination, bytes } = &seen[0] else {
        panic!("{seen:?}")
    };
    assert_eq!(destination, "lemnos/flash/image.img.xz.json");
    let manifest: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(manifest["board"], "reference");
    assert_eq!(manifest["image"]["size"], RAW);
    assert_eq!(
        seen[1],
        Seen::Push {
            destination: "lemnos/flash/image.img.xz".into(),
            bytes: b"the xz file as sent".to_vec()
        }
    );
    assert_eq!(
        seen[2],
        Seen::Call {
            device: "flash".into(),
            words: vec!["verify".into()],
            job: true
        }
    );
    assert_eq!(seen.len(), 3);

    let phases: Vec<WritePhase> = progress.iter().map(|p| p.phase).collect();
    assert_eq!(
        phases,
        [
            WritePhase::Writing,
            WritePhase::Writing,
            WritePhase::Verifying
        ]
    );
    assert_eq!(progress[1].done, progress[1].total);
}

#[tokio::test]
async fn an_answer_without_the_fields_is_refused_not_read_from_prose() {
    let dir = tempfile::tempdir().unwrap();
    let flash = BridgeFlash::new(Board::answering(
        "8388608 bytes written; signature verified",
    ));
    let (report, _) = write(&flash, &image(&dir)).await;
    assert!(matches!(report, Err(StepError::Refused(_))), "{report:?}");
    assert_eq!(
        flash.board().seen().len(),
        2,
        "no verify after an answer Atlas cannot read"
    );
}

#[tokio::test]
async fn a_count_that_is_not_the_image_s_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let short = BridgeFlash::new(Board::answering("raw_bytes=4096 outcome=0"));
    assert!(matches!(
        write(&short, &image(&dir)).await.0,
        Err(StepError::Refused(_))
    ));

    let mut board = Board::answering("raw_bytes=8388608 outcome=0");
    board.verify = Ok(b"raw_bytes=4096".to_vec());
    let misread = BridgeFlash::new(board);
    assert!(matches!(
        write(&misread, &image(&dir)).await.0,
        Err(StepError::Refused(_))
    ));
}

#[tokio::test]
async fn a_verify_whose_answer_is_lost_is_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut board = Board::answering("raw_bytes=8388608 outcome=0");
    board.verify = Err(CallError::Lost("the session ended".into()));
    let flash = BridgeFlash::new(board);
    assert!(matches!(
        write(&flash, &image(&dir)).await.0,
        Err(StepError::Again(_))
    ));
}

#[tokio::test]
async fn a_restart_that_takes_the_session_is_the_restart() {
    for (answer, ok) in [
        (Ok(Vec::new()), true),
        (Err(CallError::Lost("the session ended".into())), true),
        (Err(CallError::NotSent("not connected".into())), false),
        (Err(CallError::Refused("NOT_SUPPORTED".into())), false),
    ] {
        let mut board = Board::answering("");
        board.reboot = answer.clone();
        let flash = BridgeFlash::new(board);
        let result = flash.reboot(&|_| {}).await;
        assert_eq!(result.is_ok(), ok, "{answer:?} -> {result:?}");
        assert_eq!(
            flash.board().seen(),
            [Seen::Call {
                device: "reboot".into(),
                words: vec!["reboot".into()],
                job: false
            }]
        );
    }
}
