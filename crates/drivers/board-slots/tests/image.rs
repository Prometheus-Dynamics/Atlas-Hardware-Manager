//! An install from the one OS image: the slot is read out of the image, with the signature the
//! image keeps, and a board with security on takes it only when it holds the signer's key. Nothing
//! is signed on the computer. An image that holds several slots is refused until one is named.

mod common;

use atlas_board_slots::{InstallRequest, Source};
use atlas_jobs::JobId;
use atlas_sim_node::image::{self, Slot};
use atlas_sim_node::slots::TrustedKey;
use atlas_slots::Stage;
use common::{BOARD, Rig, device};
use ed25519_dalek::SigningKey;
use std::path::PathBuf;
use std::time::Duration;

const SEED: [u8; 32] = [9; 32];

/// Writes an OS image holding one slot for the rig's board and returns its path.
fn os_image(rig: &Rig, id: &str, signer: Option<[u8; 32]>) -> PathBuf {
    let scratch = rig.dir.join(format!("make-{id}"));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");
    let bytes = image::build(
        &Slot {
            id,
            product: "sim",
            version: "2.0.0",
            board: BOARD,
            display_name: None,
            signer,
        },
        &scratch,
    )
    .expect("the image is made");
    let path = rig.dir.join(format!("{id}.img"));
    std::fs::write(&path, bytes).expect("the image is written");
    path
}

async fn ended(rig: &Rig, job: JobId) -> Stage {
    tokio::time::timeout(atlas_sim_node::DEADLINE, async {
        loop {
            if let Some(stage) = rig.installs.stage(job).filter(Stage::is_final) {
                return stage;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the install ends")
}

async fn install(rig: &Rig, image: PathBuf) -> Stage {
    let job = rig
        .installs
        .submit(
            device(),
            &rig.address(),
            BOARD,
            true,
            InstallRequest {
                source: Source::image(image),
                starred: false,
            },
        )
        .await
        .expect("the install is queued");
    ended(rig, job).await
}

fn trust(rig: &Rig, seed: [u8; 32]) {
    let mut model = rig.node.model();
    model.slots.board_security = true;
    model.slots.trusted.push(TrustedKey {
        public: SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
        owner: "Test Owner".into(),
    });
}

#[tokio::test]
async fn a_board_with_security_on_takes_an_image_s_slot_signed_by_a_key_it_holds() {
    let rig = Rig::start("image-signed").await;
    trust(&rig, SEED);
    let stage = install(&rig, os_image(&rig, "slot-signed", Some(SEED))).await;
    assert_eq!(
        stage,
        Stage::Done {
            slot: "slot-signed".into()
        }
    );
}

#[tokio::test]
async fn a_board_with_security_on_refuses_an_image_s_unsigned_slot_and_one_signed_by_another_key() {
    let rig = Rig::start("image-refused").await;
    trust(&rig, SEED);
    // Unsigned in the image: sent unsigned, never signed on the computer, and the board refuses it.
    let unsigned = install(&rig, os_image(&rig, "slot-unsigned", None)).await;
    assert!(matches!(unsigned, Stage::Failed { .. }), "{unsigned:?}");
    // Signed by a key the board does not hold.
    let other = install(&rig, os_image(&rig, "slot-other", Some([3; 32]))).await;
    assert!(matches!(other, Stage::Failed { .. }), "{other:?}");
    let list = common::answered("the slot list", || rig.slots.list())
        .await
        .expect("the board lists its slots");
    assert!(
        !list
            .slots
            .iter()
            .any(|slot| slot.id == "slot-unsigned" || slot.id == "slot-other"),
        "neither was installed"
    );
}

#[tokio::test]
async fn an_image_with_several_slots_is_refused_by_name_and_nothing_is_sent() {
    let rig = Rig::start("image-several").await;
    // atlas-image's own fixture: one data partition holding the slots alpha and beta.
    let data = include_bytes!("../../../flash/image/tests/fixtures/data.ext4");
    let path = rig.dir.join("two-slots.img");
    std::fs::write(&path, image::disk(data)).expect("the image is written");
    let stage = install(&rig, path).await;
    match stage {
        Stage::Failed { reason } => {
            assert!(reason.contains("holds 2 slots (alpha, beta)"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}
