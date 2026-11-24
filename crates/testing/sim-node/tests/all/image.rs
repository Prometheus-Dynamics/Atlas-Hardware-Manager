//! The test image, read back by the real image reader and taken by the node's own bundle check.

use atlas_sim_node::bundle;
use atlas_sim_node::image::{self, Slot};
use atlas_sim_node::slots::{Signed, TrustedKey};
use ed25519_dalek::SigningKey;

const SEED: [u8; 32] = [7; 32];

fn slot(signer: Option<[u8; 32]>) -> Slot<'static> {
    Slot {
        id: "demo-os-1.2.3",
        product: "demo-os",
        version: "1.2.3",
        board: "simulated",
        display_name: Some("Demo OS 1.2.3"),
        signer,
    }
}

/// The bundle Atlas makes from the image holding `slot`, as an install makes it.
fn bundle_of(slot: &Slot<'_>) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let bytes = image::build(slot, dir.path()).expect("the image is made");
    let path = dir.path().join("os.img");
    std::fs::write(&path, bytes).expect("the image is written");
    let probe = atlas_image::probe_board(&path, dir.path()).expect("the image is read");
    assert_eq!(probe.board, "simulated");
    assert_eq!(probe.slots, [slot.id]);
    let mut out = Vec::new();
    let made = atlas_bundle::bundle_from_image(&path, dir.path(), slot.id, &mut out)
        .expect("the slot is read out of the image");
    assert_eq!(made.signed, slot.signer.is_some());
    out
}

#[test]
fn a_signed_slot_out_of_an_image_is_trusted_by_a_board_that_holds_its_key() {
    let trusted = [TrustedKey {
        public: SigningKey::from_bytes(&SEED).verifying_key().to_bytes(),
        owner: "Test Owner".into(),
    }];
    let verified = bundle::verify(&bundle_of(&slot(Some(SEED))), &trusted, "simulated")
        .expect("the node takes it");
    assert_eq!(verified.id, "demo-os-1.2.3");
    assert_eq!(verified.version, "1.2.3");
    assert_eq!(
        verified.signed,
        Signed::Trusted {
            owner: "Test Owner".into()
        }
    );
}

#[test]
fn a_signed_slot_is_not_checked_by_a_board_without_its_key_and_an_unsigned_one_is_unsigned() {
    let signed = bundle::verify(&bundle_of(&slot(Some(SEED))), &[], "simulated").expect("taken");
    assert_eq!(signed.signed, Signed::Untrusted);
    let unsigned = bundle::verify(&bundle_of(&slot(None)), &[], "simulated").expect("taken");
    assert_eq!(unsigned.signed, Signed::Unsigned);
}
