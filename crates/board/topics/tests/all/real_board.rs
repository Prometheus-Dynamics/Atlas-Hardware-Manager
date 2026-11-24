//! The descriptions a real board answered, read as Atlas reads them. A description Atlas refuses
//! shows on the Hardware tab as a device whose state and bus are "not reported".

use atlas_topics::description::parse;

const FIXTURE: &str = include_str!("../fixtures/board-describes-2026-11-22.txt");

fn sections() -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for line in FIXTURE.lines().filter(|line| !line.starts_with('#')) {
        if let Some(name) = line.strip_prefix("=== ") {
            out.push((name, String::new()));
        } else if let Some((_, text)) = out.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    out
}

#[test]
fn every_description_the_real_board_answered_is_read() {
    let all = sections();
    assert_eq!(all.len(), 19, "the fixture's devices");
    let refused: Vec<String> = all
        .iter()
        .filter_map(|(name, text)| parse(text).err().map(|why| format!("{name}: {why:?}")))
        .collect();
    assert!(refused.is_empty(), "refused:\n{}", refused.join("\n"));
}
