//! The board's list read as data: absent signatures are not verified, unknown words are kept,
//! and the trial and resolved records come through.

use atlas_slots::{Reported, Resolution, SignatureOutcome, SlotList, TrialLast};

const LIST: &str = r#"{"slot_active":"s1","slot_default":"s1","slot_previous":"","slot_trial":"s2",
 "slot_staged":"","state":"idle","error":"","free_bytes":9,"reserve_bytes":2,"keep":3,
 "slots":[
  {"id":"s2","product":"p","board":"b","version":"2","variant":"","kernel":"","display_name":"",
   "good":"untried","signed":"trusted","signer":"owner","installed":"","last_boot":"","size":5,
   "verity":true,"intact":true,"flags":["trial"],"pinned":true,"failures":0,"failed":false,
   "signature":"verified"},
  {"id":"s1","product":"p","board":"b","version":"1","variant":"","kernel":"","display_name":"",
   "good":"yes","signed":"unsigned","signer":"","installed":"","last_boot":"","size":5,
   "verity":true,"intact":true,"flags":["running","default"],"pinned":false,"failures":0,"failed":false,
   "signature":"novel_word"},
  {"id":"s0","product":"p","board":"b","version":"0","variant":"","kernel":"","display_name":"",
   "good":"yes","signed":"unsigned","signer":"","installed":"","last_boot":"","size":5,
   "verity":true,"intact":true,"flags":[],"pinned":false,"failures":0,"failed":false}],
 "trial":{"slot":"s2","attempts_left":2,"attempts":[{"n":1,"time":"t1"}],"last":"daemon-reached-not-yet-confirmed",
   "last_time":"t2","reason":"","record_error":"state dir is read-only"},
 "resolved":{"slot":"s0","outcome":"fell-back","by":"auto","caller":7,"reason":"deadline","time":"t3"}}"#;

#[test]
fn a_list_reads_slots_newest_first_with_flags_and_signature_words() {
    let list = SlotList::parse(LIST).unwrap();
    assert_eq!(list.running, "s1");
    assert_eq!(list.keep, 3);
    let ids: Vec<&str> = list.slots.iter().map(|slot| slot.id.as_str()).collect();
    assert_eq!(ids, ["s2", "s1", "s0"]);
    assert!(list.get("s1").unwrap().has_flag("running"));
    assert!(list.get("s2").unwrap().starred());
    assert_eq!(
        list.get("s2").unwrap().signature,
        Reported::Known(SignatureOutcome::Verified)
    );
    assert_eq!(
        list.get("s1").unwrap().signature,
        Reported::Known(SignatureOutcome::Other("novel_word".into())),
        "an unknown word is kept as the board wrote it"
    );
}

#[test]
fn an_absent_signature_is_not_reported_and_never_verified() {
    let list = SlotList::parse(LIST).unwrap();
    assert_eq!(list.get("s0").unwrap().signature, Reported::NotReported);
}

#[test]
fn the_trial_and_resolved_records_come_through() {
    let list = SlotList::parse(LIST).unwrap();
    let trial = list.trial.as_ref().unwrap();
    assert_eq!(trial.slot, "s2");
    assert_eq!(trial.attempts_left, Some(2));
    assert_eq!(trial.attempts[0].n, 1);
    assert_eq!(trial.last, Some(TrialLast::DaemonReachedNotYetConfirmed));
    assert_eq!(trial.record_error.as_deref(), Some("state dir is read-only"));
    let resolved = list.resolved.as_ref().unwrap();
    assert_eq!(resolved.outcome, Resolution::FellBack);
    assert_eq!(resolved.caller, "7", "a numeric caller is kept as text");
    assert_eq!(resolved.reason, "deadline");
}

#[test]
fn absent_trial_and_resolved_are_none_and_a_bad_document_is_refused() {
    let plain = SlotList::parse(r#"{"slots":[]}"#).unwrap();
    assert!(plain.trial.is_none());
    assert!(plain.resolved.is_none());
    assert!(SlotList::parse("not json").is_err());
    assert!(SlotList::parse(r#"{"state":"idle"}"#).is_err());
    assert!(SlotList::parse(r#"{"slots":[{"flags":[]}]}"#).is_err());
}
