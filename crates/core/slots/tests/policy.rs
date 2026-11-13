//! The retention policy as pure rules: keep N, starred versions, the running, default and trial
//! slots, and what an install would remove.

use atlas_slots::{RemoveRefusal, SlotList, View, check_remove, offers_slots, would_remove, would_remove_on_install, with_install};

fn view(id: &str, starred: bool, protected: bool) -> View {
    View {
        id: id.to_owned(),
        starred,
        protected,
    }
}

#[test]
fn keep_zero_keeps_everything() {
    let views = [view("a", false, false), view("b", false, false)];
    assert!(would_remove(&views, 0).is_empty());
}

#[test]
fn keep_last_n_removes_the_oldest_unstarred_beyond_n_newest_first() {
    // Newest first: d, c, b, a. With N = 2, the two newest unprotected stay.
    let views = [
        view("d", false, false),
        view("c", false, false),
        view("b", false, false),
        view("a", false, false),
    ];
    assert_eq!(would_remove(&views, 2), vec!["b".to_owned(), "a".to_owned()]);
}

#[test]
fn a_starred_version_is_never_removed_and_does_not_count_toward_n() {
    // a is starred and oldest. Keep 2: d and c stay; b goes; a is kept by its star.
    let views = [
        view("d", false, false),
        view("c", false, false),
        view("b", false, false),
        view("a", true, false),
    ];
    assert_eq!(would_remove(&views, 2), vec!["b".to_owned()]);
}

#[test]
fn starred_versions_do_not_use_up_the_count() {
    // Two starred, keep 1: the one unstarred newest stays, nothing else goes but the older
    // unstarred one.
    let views = [
        view("e", false, false),
        view("d", true, false),
        view("c", true, false),
        view("b", false, false),
    ];
    assert_eq!(would_remove(&views, 1), vec!["b".to_owned()]);
}

#[test]
fn the_running_and_default_and_trial_slots_are_protected_and_count() {
    // Protected: a (running), c (default). Keep 2: both count, so b and d go.
    let views = [
        view("d", false, false),
        view("c", false, true),
        view("b", false, false),
        view("a", false, true),
    ];
    assert_eq!(would_remove(&views, 2), vec!["d".to_owned(), "b".to_owned()]);
}

#[test]
fn a_protected_slot_is_never_in_the_removal_list() {
    let views = [view("run", false, true), view("old", false, false)];
    let removed = would_remove(&views, 1);
    assert!(!removed.contains(&"run".to_owned()));
}

#[test]
fn an_install_preview_counts_the_new_slot_as_the_newest() {
    // Board: running a, default a, unstarred b (oldest), c. Keep 2, install d unstarred:
    // a counts (protected), d is newest and counts; c counts... only one more fits, so c stays
    // and b goes.
    let json = r#"{"slot_active":"a","slot_default":"a","slot_trial":"","keep":2,"state":"idle","error":"",
        "free_bytes":1,"reserve_bytes":0,"slots":[
        {"id":"c","flags":[],"pinned":false},
        {"id":"b","flags":[],"pinned":false},
        {"id":"a","flags":["running","default"],"pinned":false}]}"#;
    let list = SlotList::parse(json).unwrap();
    assert_eq!(
        would_remove_on_install(&list, 2, "d", false),
        vec!["c".to_owned(), "b".to_owned()],
        "a and d count toward 2; c and b are beyond it"
    );
    // Installed starred, the new one does not count, so one unstarred slot is kept beyond a:
    // Starred install: d is not counted, so c (count 1 -> 2) stays and only b goes.
    assert_eq!(would_remove_on_install(&list, 2, "d", true), vec!["b".to_owned()]);
}

#[test]
fn with_install_puts_the_new_slot_first_and_replaces_a_duplicate() {
    let views = [view("a", false, true), view("b", false, false)];
    let next = with_install(&views, "b", true);
    assert_eq!(next[0], view("b", true, false));
    assert_eq!(next.len(), 2);
}

fn board(json_slots: &str) -> SlotList {
    SlotList::parse(&format!(
        r#"{{"slot_active":"a","slot_default":"a","slot_trial":"","keep":3,"state":"idle","error":"","free_bytes":1,"reserve_bytes":0,"slots":[{json_slots}]}}"#
    ))
    .unwrap()
}

#[test]
fn removal_guard_refuses_running_default_trial_only_missing_and_starred() {
    let list = board(
        r#"{"id":"c","flags":[],"pinned":false},
           {"id":"b","flags":["trial"],"pinned":false},
           {"id":"a","flags":["running","default"],"pinned":false},
           {"id":"s","flags":[],"pinned":true}"#,
    );
    assert!(matches!(check_remove(&list, "a", true), Err(RemoveRefusal::Running(_))));
    assert!(matches!(check_remove(&list, "b", true), Err(RemoveRefusal::Trial(_))));
    assert!(matches!(check_remove(&list, "zz", true), Err(RemoveRefusal::Missing(_))));
    assert!(matches!(check_remove(&list, "s", false), Err(RemoveRefusal::Starred(_))));
    assert_eq!(check_remove(&list, "s", true), Ok(()));
    assert_eq!(check_remove(&list, "c", false), Ok(()));
    let only = board(r#"{"id":"a","flags":["running","default"],"pinned":false}"#);
    assert!(matches!(check_remove(&only, "a", false), Err(RemoveRefusal::Running(_))));
}

#[test]
fn a_default_that_is_not_running_is_refused_as_default() {
    let list = SlotList::parse(
        r#"{"slot_active":"x","slot_default":"d","slot_trial":"","keep":0,"state":"idle","error":"","free_bytes":0,"reserve_bytes":0,"slots":[
        {"id":"d","flags":["default"],"pinned":false},{"id":"x","flags":["running"],"pinned":false}]}"#,
    )
    .unwrap();
    assert!(matches!(check_remove(&list, "d", false), Err(RemoveRefusal::Default(_))));
}

#[test]
fn an_offer_of_the_slot_contract_is_read_from_the_offers_text() {
    assert!(offers_slots("system.reboot/1 system.slots/1"));
    assert!(!offers_slots("system.reboot/1"));
    assert!(!offers_slots(""));
}
