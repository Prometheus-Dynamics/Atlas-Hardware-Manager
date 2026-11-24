//! The sim names the Lemnos methods the way Atlas does (`system.<contract>.<method>`, one spelling). The
//! sim cannot depend on this crate, so this test checks the two agree: the sim's names are the session's.

use atlas_session::lemnos::actions;
use atlas_sim_node::names;

#[test]
fn the_sim_names_each_method_as_the_session_does() {
    let pairs = [
        (names::SLOTS_LIST, actions::SLOTS_LIST),
        (names::SLOTS_ACCEPT, actions::SLOTS_ACCEPT),
        (names::SLOTS_INSTALL_FILE, actions::SLOTS_INSTALL_FILE),
        (names::SLOTS_SET_DEFAULT, actions::SLOTS_SET_DEFAULT),
        (names::SLOTS_TRIAL, actions::SLOTS_TRIAL),
        (names::SLOTS_REMOVE, actions::SLOTS_REMOVE),
        (names::SLOTS_PIN, actions::SLOTS_PIN),
        (names::SLOTS_SET_KEEP, actions::SLOTS_SET_KEEP),
        (names::SLOTS_INFO, actions::SLOTS_INFO),
    ];
    for (sim, session) in pairs {
        assert_eq!(
            sim, session,
            "the sim and the session name one method one way"
        );
    }
}

/// The sim's contract hashes are the pinned contract crates' own hashes (`Contract::HASH`), so a describe
/// the sim writes carries the hash a real board's describe carries.
#[test]
fn the_sim_contract_hashes_are_the_pinned_contracts_hashes() {
    use lemnos_contract_system_reboot::Reboot;
    use lemnos_contract_system_slots::Slots;
    use lemnos_core::Contract;
    assert_eq!(
        <dyn Slots as Contract>::HASH,
        atlas_sim_node::readings::SLOTS_CONTRACT_HASH,
        "system.slots/1 at 214c0ca7"
    );
    assert_eq!(
        <dyn Reboot as Contract>::HASH,
        atlas_sim_node::readings::REBOOT_CONTRACT_HASH,
        "system.reboot/1 at 214c0ca7"
    );
}
