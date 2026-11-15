//! `atlas-next usb-boot`: USB boots a board with the boot files of a built image, through the
//! SystemCore agent's boot-only job. The eMMC is never written: the job has no write path.
//!
//! Before anything is sent the tool checks, and refuses on any failure, naming each reason:
//!
//! 1. the slot bundle's signature verifies with the given public key, and its manifest is read;
//! 2. the boot partition (a FAT image from `lemnos-ctl boot-partition`) holds a bay set whose
//!    committed record names the bundle's slot, and every boot file in the manifest is in that bay
//!    with the same SHA-256 (of the stored bytes, the record's length);
//! 3. the partition's `config.txt` (or a file it includes) carries `dtoverlay=ramoops-pi5`, and
//!    `overlays/ramoops-pi5.dtbo` is in the tree: without them link 1 cannot be judged, because
//!    its proof is the board's pstore;
//! 4. every file of the partition's tree can be named to the agent (the boot-only API serves
//!    names and `overlays/<name>`).
//!
//! The served set is the partition's own file tree as it is, plus `bootcode5.bin` (the Pi's
//! second stage, from the flasher files). The job's events are followed to `boot-end`.

mod fat;
mod http;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use atlas_bundle::inspect_bundle;
use atlas_release::PublicKey;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::Env;
use crate::args::UsbBootOptions;

/// The line that makes the board's kernel write its pstore (crash log) to ramoops.
pub const RAMOOPS_LINE: &str = "dtoverlay=ramoops-pi5";
/// The overlay file `RAMOOPS_LINE` loads.
pub const RAMOOPS_OVERLAY: &str = "overlays/ramoops-pi5.dtbo";
/// The Pi's second-stage bootloader the boot ROM sends first.
pub const SECOND_STAGE: &str = "bootcode5.bin";
/// The default folder of `bootcode5.bin`: the repository's Pi flasher files.
pub const DEFAULT_BOOTFILES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../devices/families/bcm2712/flasher"
);
/// The bay sets of the boot partition: the default, the previous and the one being installed.
const BAY_SETS: usize = 3;
const POLL: Duration = Duration::from_millis(500);

/// One file the job serves: its path, its bytes, their SHA-256 and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub path: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub role: String,
}

/// The checked plan: what would be sent, and which slot it is.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub slot: String,
    pub board: String,
    pub bay: String,
    pub files: Vec<Served>,
}

/// Checks everything before anything is sent. `Err` carries every refusal, each in plain words.
///
/// # Errors
///
/// The list of refusals, when any check fails.
pub fn make_plan(opts: &UsbBootOptions) -> Result<Plan, Vec<String>> {
    let key_text = std::fs::read_to_string(&opts.pubkey)
        .map_err(|error| vec![format!("the public key {}: {error}", opts.pubkey.display())])?;
    let key = PublicKey::parse(&key_text)
        .map_err(|error| vec![format!("the public key {}: {error}", opts.pubkey.display())])?;
    let bundle = std::fs::File::open(&opts.bundle)
        .map_err(|error| vec![format!("the bundle {}: {error}", opts.bundle.display())])?;
    let inspected = inspect_bundle(bundle, &[key])
        .map_err(|error| vec![format!("the bundle is refused: {error}: nothing was sent")])?;
    let manifest = inspected.manifest;
    let image = std::fs::read(&opts.boot_partition).map_err(|error| {
        vec![format!(
            "the boot partition {}: {error}",
            opts.boot_partition.display()
        )]
    })?;
    let tree = fat::read_tree(image).map_err(|error| vec![format!("{error}: nothing was sent")])?;

    let mut refusals = Vec::new();
    for group in fat::collisions(&tree) {
        refusals.push(format!(
            "{} collide ignoring case on the boot partition (the firmware's requests would match both): nothing was sent",
            group.join(" and ")
        ));
    }
    let listed: BTreeSet<&str> = manifest.boot.files.keys().map(String::as_str).collect();
    let stored: BTreeSet<&str> = fat::BAY_PATHS.into_iter().collect();
    if listed != stored {
        refusals.push(format!(
            "the bundle's boot files are {listed:?}, not the five the boot partition stores: nothing was sent"
        ));
    }

    // The bay set that holds the slot, and its record.
    let bay = (0..BAY_SETS).find_map(|set| {
        let (_, record) = fat::find(&tree, &format!("slots/bay{set}/bayinfo.rec"))?;
        let info = fat::parse_bayinfo(record)?;
        (info.slot == manifest.id).then_some((set, info))
    });
    // What each served file's bytes are: the tree's bytes, except a bay file, which is served at
    // its stored length (the bytes the manifest hashes), never padded to the bay's size.
    let mut served_bytes: BTreeMap<String, Vec<u8>> = tree.clone();
    let mut bay_set = 0;
    match bay {
        None => refusals.push(format!(
            "no bay set of the boot partition holds slot {:?} (a committed bayinfo.rec names it): nothing was sent",
            manifest.id
        )),
        Some((set, info)) => {
            bay_set = set;
            for (path, bundle_sha) in &manifest.boot.files {
                let Some(index) = fat::BAY_PATHS.iter().position(|p| p == path) else {
                    continue;
                };
                let (len, record_hash) = info.files[index];
                let wanted = format!("slots/bay{set}/{path}");
                let Some((real, bytes)) = fat::find(&tree, &wanted) else {
                    refusals.push(format!(
                        "{path} is missing from bay{set} of the boot partition (bundle SHA-256 {bundle_sha}): nothing was sent"
                    ));
                    continue;
                };
                let Some(stored_bytes) = usize::try_from(len).ok().and_then(|n| bytes.get(..n))
                else {
                    refusals.push(format!(
                        "{path} in bay{set} is shorter than its record's {len} bytes: nothing was sent"
                    ));
                    continue;
                };
                let actual = sha256_hex(stored_bytes);
                if record_hash.iter().map(|b| format!("{b:02x}")).collect::<String>() != actual {
                    refusals.push(format!(
                        "{path} in bay{set}: the file does not match the boot partition's own record: nothing was sent"
                    ));
                }
                if &actual != bundle_sha {
                    refusals.push(format!(
                        "{path}: bundle SHA-256 {bundle_sha}, boot partition SHA-256 {actual}: refused, nothing was sent"
                    ));
                }
                served_bytes.insert(real.to_string(), stored_bytes.to_vec());
            }
        }
    }

    refusals.extend(ramoops_refusals(&tree));

    // The agent takes one to four segments, each a valid name.
    let unservable: Vec<&String> = tree.keys().filter(|path| !servable(path)).collect();
    if !unservable.is_empty() {
        refusals.push(format!(
            "the agent's boot-only API cannot take these paths (one to four valid name segments): {}: nothing was sent",
            unservable.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }

    let bootfiles = opts
        .bootfiles
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BOOTFILES));
    let second_stage_path = bootfiles.join(SECOND_STAGE);
    let second_stage = match std::fs::read(&second_stage_path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            refusals.push(format!(
                "the second stage {}: {error}: nothing was sent",
                second_stage_path.display()
            ));
            None
        }
    };
    if fat::find(&tree, SECOND_STAGE).is_some() {
        refusals.push(format!(
            "the boot partition already holds {SECOND_STAGE} (ignoring case): nothing was sent"
        ));
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }
    let mut files: Vec<Served> = served_bytes
        .into_iter()
        .map(|(path, bytes)| {
            let role = if path
                .to_ascii_lowercase()
                .starts_with(&format!("slots/bay{bay_set}/"))
            {
                format!("slot {} bay{bay_set} (boot partition)", manifest.id)
            } else {
                "boot partition tree".to_string()
            };
            Served {
                sha256: sha256_hex(&bytes),
                path,
                bytes,
                role,
            }
        })
        .collect();
    if let Some(bytes) = second_stage {
        files.push(Served {
            path: SECOND_STAGE.to_string(),
            sha256: sha256_hex(&bytes),
            bytes,
            role: format!("second stage, from {}", bootfiles.display()),
        });
    }
    Ok(Plan {
        slot: manifest.id,
        board: manifest.board,
        bay: format!("bay{bay_set}"),
        files,
    })
}

/// Re-hashes every served file's exact bytes against the hash it was planned with. A changed
/// byte between the check and the upload is a refusal, so what is served is what was verified.
///
/// # Errors
///
/// One line per file whose bytes no longer hash to its planned SHA-256.
pub fn verify_served(files: &[Served]) -> Result<(), Vec<String>> {
    let bad: Vec<String> = files
        .iter()
        .filter(|file| sha256_hex(&file.bytes) != file.sha256)
        .map(|file| {
            format!(
                "{} changed after it was verified (planned SHA-256 {}): nothing was sent",
                file.path, file.sha256
            )
        })
        .collect();
    if bad.is_empty() { Ok(()) } else { Err(bad) }
}

/// The ramoops refusals: `config.txt`, or a file it includes, must carry the ramoops line, and
/// the overlay must be in the tree. Nothing is added to fix them.
fn ramoops_refusals(tree: &fat::Tree) -> Vec<String> {
    let mut found = false;
    let mut seen = BTreeSet::new();
    visit_config(tree, "config.txt", &mut found, &mut seen, 0);
    let mut refusals = Vec::new();
    if !found {
        refusals.push(format!(
            "config.txt (or a file it includes) has no {RAMOOPS_LINE:?}: link 1 can't be judged without the board's pstore: nothing was sent"
        ));
    }
    if fat::find(tree, RAMOOPS_OVERLAY).is_none() {
        refusals.push(format!(
            "{RAMOOPS_OVERLAY} is not in the boot partition: link 1 can't be judged without the board's pstore: nothing was sent"
        ));
    }
    refusals
}

fn visit_config(
    tree: &fat::Tree,
    name: &str,
    found: &mut bool,
    seen: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth > 8 || !seen.insert(name.to_ascii_lowercase()) {
        return;
    }
    let Some((_, bytes)) = fat::find(tree, name) else {
        return;
    };
    for line in String::from_utf8_lossy(bytes).lines() {
        let line = line.trim();
        if line == RAMOOPS_LINE {
            *found = true;
        }
        if let Some(include) = line.strip_prefix("include ") {
            visit_config(
                tree,
                &include.trim().to_ascii_lowercase(),
                found,
                seen,
                depth + 1,
            );
        }
    }
}

/// The deepest path the agent's boot-only API takes (`BOOT_ONLY_MAX_DEPTH` in the agent).
pub const MAX_DEPTH: usize = 4;

/// Whether the agent's boot-only API takes this path: one to four segments, each a valid name
/// (letters, digits, `.`, `-`, `_`, `+`; no leading dot; at most 200 bytes). A slot's bay file
/// is `slots/<bay>/<name>`, which is taken as the partition holds it.
#[must_use]
pub fn servable(path: &str) -> bool {
    fn name_ok(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 200
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
    }
    let segments: Vec<&str> = path.split('/').collect();
    (1..=MAX_DEPTH).contains(&segments.len()) && segments.iter().all(|s| name_ok(s))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The plan as text, for the coordinator to read before any board contact.
pub fn write_plan<W: Write>(plan: &Plan, opts: &UsbBootOptions, out: &mut W) {
    let _ = writeln!(out, "usb-boot plan");
    let _ = writeln!(out, "  agent      {}", opts.agent);
    let _ = writeln!(out, "  port       {}", opts.port);
    let _ = writeln!(
        out,
        "  slot       {} (board {}, bay {})",
        plan.slot, plan.board, plan.bay
    );
    let _ = writeln!(
        out,
        "  files      {} (eMMC is never written)",
        plan.files.len()
    );
    let _ = writeln!(
        out,
        "  path        bytes  sha256                                                             role"
    );
    for file in &plan.files {
        let _ = writeln!(
            out,
            "  {:<11} {:>6}  {}  {}",
            file.path,
            file.bytes.len(),
            file.sha256,
            file.role
        );
    }
    let _ = writeln!(
        out,
        "  ramoops    ok: {RAMOOPS_LINE} and {RAMOOPS_OVERLAY} present"
    );
}

/// The exit status of a job's last event: `done` and `disconnected` are success.
#[must_use]
pub fn exit_for_outcome(outcome: &str) -> i32 {
    match outcome {
        "done" | "disconnected" => 0,
        _ => 1,
    }
}

enum Msg {
    Line(String),
    Job(String),
    Exit(i32),
}

/// Runs `usb-boot`: checks, prints the plan, and (unless `--dry-run`) sends it and follows the
/// job. Returns the exit status.
pub async fn run<W: Write>(opts: &UsbBootOptions, env: &Env, out: &mut W) -> i32 {
    let plan = match make_plan(opts) {
        Ok(plan) => plan,
        Err(refusals) => {
            let _ = writeln!(out, "usb-boot REFUSED: nothing was sent to the agent");
            for refusal in &refusals {
                let _ = writeln!(out, "  - {refusal}");
            }
            return 1;
        }
    };
    write_plan(&plan, opts, out);
    if let Err(problems) = verify_served(&plan.files) {
        let _ = writeln!(out, "usb-boot REFUSED: nothing was sent to the agent");
        for problem in &problems {
            let _ = writeln!(out, "  - {problem}");
        }
        return 1;
    }
    if opts.dry_run {
        let _ = writeln!(out, "dry run: nothing was sent");
        return 0;
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let agent = opts.agent.clone();
    let port = opts.port.clone();
    let timeout_s = opts.timeout_s;
    let files: Vec<(String, Vec<u8>, String)> = plan
        .files
        .into_iter()
        .map(|file| (file.path, file.bytes, file.sha256))
        .collect();
    tokio::task::spawn_blocking(move || {
        let sender = tx.clone();
        let code = drive(&agent, &port, timeout_s, &files, &tx);
        let _ = sender.send(Msg::Exit(code));
    });
    let mut job: Option<String> = None;
    loop {
        tokio::select! {
            message = rx.recv() => match message {
                Some(Msg::Line(text)) => {
                    let _ = writeln!(out, "{text}");
                }
                Some(Msg::Job(id)) => {
                    let _ = writeln!(out, "job {id}");
                    job = Some(id);
                }
                Some(Msg::Exit(code)) => return code,
                None => return 1,
            },
            () = notified(env) => {
                let _ = writeln!(out, "usb-boot: Ctrl-C, cancelling the job");
                if let Some(id) = &job {
                    let agent = opts.agent.clone();
                    let path = format!("/v1/jobs/{id}");
                    let _ = tokio::task::spawn_blocking(move || {
                        http::request(&agent, "DELETE", &path, &[], &[])
                    })
                    .await;
                }
                return 130;
            }
        }
    }
}

async fn notified(env: &Env) {
    match &env.stop {
        Some(stop) => stop.notified().await,
        None => std::future::pending::<()>().await,
    }
}

/// The blocking part: upload, start the job, follow its events. Sends each line for printing and
/// returns the exit status.
fn drive(
    agent: &str,
    port: &str,
    timeout_s: Option<u64>,
    files: &[(String, Vec<u8>, String)],
    tx: &mpsc::UnboundedSender<Msg>,
) -> i32 {
    let say = |text: String| {
        let _ = tx.send(Msg::Line(text));
    };
    let fail = |text: String| {
        let _ = tx.send(Msg::Line(format!("usb-boot FAILED: {text}")));
        1
    };
    match http::request(agent, "GET", "/v1/agent", &[], &[]) {
        Ok(reply) if reply.status == 200 => {
            say(format!("agent: {}", String::from_utf8_lossy(&reply.body)))
        }
        Ok(reply) => return fail(format!("GET /v1/agent answered {}", reply.status)),
        Err(error) => return fail(error),
    }
    for (path, bytes, sha) in files {
        let target = format!("/v1/boot-only/files/{path}");
        match http::request(agent, "PUT", &target, &[("x-sha256", sha)], bytes) {
            Ok(reply) if (200..300).contains(&reply.status) => {
                say(format!("uploaded {path} ({} bytes)", bytes.len()));
            }
            Ok(reply) => {
                return fail(format!(
                    "PUT {path} answered {}: {}",
                    reply.status,
                    String::from_utf8_lossy(&reply.body)
                ));
            }
            Err(error) => return fail(error),
        }
    }
    let mut request = serde_json::json!({
        "port": port,
        "files": files.iter().map(|(p, _, s)| (p.clone(), s.clone())).collect::<BTreeMap<_, _>>(),
    });
    if let Some(seconds) = timeout_s {
        request["timeout_s"] = serde_json::json!(seconds);
    }
    let body = request.to_string();
    let id = match http::request(
        agent,
        "POST",
        "/v1/boot-only",
        &[("Content-Type", "application/json")],
        body.as_bytes(),
    ) {
        Ok(reply) if (200..300).contains(&reply.status) => {
            match serde_json::from_slice::<serde_json::Value>(&reply.body)
                .ok()
                .and_then(|value| value["id"].as_str().map(str::to_string))
            {
                Some(id) => id,
                None => return fail("the job has no id".into()),
            }
        }
        Ok(reply) => {
            return fail(format!(
                "POST /v1/boot-only answered {}: {}",
                reply.status,
                String::from_utf8_lossy(&reply.body)
            ));
        }
        Err(error) => return fail(error),
    };
    let _ = tx.send(Msg::Job(id.clone()));
    let mut next = 0usize;
    loop {
        let target = format!("/v1/jobs/{id}?from={next}");
        let value = match http::request(agent, "GET", &target, &[], &[]) {
            Ok(reply) if reply.status == 200 => {
                match serde_json::from_slice::<serde_json::Value>(&reply.body) {
                    Ok(value) => value,
                    Err(error) => return fail(format!("the job's events: {error}")),
                }
            }
            Ok(reply) => return fail(format!("GET job answered {}", reply.status)),
            Err(error) => return fail(error),
        };
        for event in value["events"].as_array().cloned().unwrap_or_default() {
            say(format_event(&event));
            if event["event"] == "boot-end" {
                let outcome = event["outcome"].as_str().unwrap_or("error");
                return exit_for_outcome(outcome);
            }
        }
        next = value["next"].as_u64().map_or(next, |n| n as usize);
        if value["finished"].as_bool() == Some(true) {
            return fail("the job ended without a boot-end".into());
        }
        std::thread::sleep(POLL);
    }
}

/// One job event as a line of text.
#[must_use]
pub fn format_event(event: &serde_json::Value) -> String {
    match event["event"].as_str() {
        Some("usb-step") => {
            let step = &event["step"];
            match step["kind"].as_str() {
                Some("waiting") => "usb-step waiting for a board in USB boot".into(),
                Some("second-stage") => format!(
                    "usb-step second stage sent to {} ({} bytes)",
                    step["chip"].as_str().unwrap_or("?"),
                    step["bytes"]
                ),
                Some("metadata") => format!(
                    "usb-step board says {}={}",
                    step["name"].as_str().unwrap_or("?"),
                    step["value"].as_str().unwrap_or("?")
                ),
                Some("not-served") => format!(
                    "usb-step not served: {}",
                    step["reason"].as_str().unwrap_or("?")
                ),
                _ => format!("usb-step {}", step),
            }
        }
        Some("boot-file") => format!(
            "boot-file {} served={} bytes={} sha256={}",
            event["name"].as_str().unwrap_or("?"),
            event["served"],
            event["bytes"],
            event["sha256"].as_str().unwrap_or("-")
        ),
        Some("boot-end") => match event["message"].as_str() {
            Some(message) => format!(
                "boot-end {}: {message}",
                event["outcome"].as_str().unwrap_or("?")
            ),
            None => format!("boot-end {}", event["outcome"].as_str().unwrap_or("?")),
        },
        _ => format!("event {event}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_status_follows_the_outcome() {
        assert_eq!(exit_for_outcome("done"), 0);
        assert_eq!(exit_for_outcome("disconnected"), 0);
        assert_eq!(exit_for_outcome("timeout"), 1);
        assert_eq!(exit_for_outcome("cancelled"), 1);
        assert_eq!(exit_for_outcome("error"), 1);
    }

    #[test]
    fn paths_of_one_to_four_valid_segments_are_servable() {
        assert!(servable("config.txt"));
        assert!(servable("overlays/ramoops-pi5.dtbo"));
        assert!(servable("slots/bay0/kernel8.img"));
        assert!(servable("slots/bay0/overlays/lemnos-board.dtbo"));
        assert!(!servable("a/b/c/d/e"));
        assert!(!servable(".hidden"));
        assert!(!servable("slots/../x"));
        assert!(!servable("slots//x"));
        assert!(!servable("slots/bay 0/x"));
    }

    #[test]
    fn ramoops_is_looked_for_through_includes() {
        let mut tree = fat::Tree::new();
        tree.insert("config.txt".into(), b"include default.txt\n".to_vec());
        tree.insert(
            "default.txt".into(),
            format!("{RAMOOPS_LINE}\n").into_bytes(),
        );
        tree.insert(RAMOOPS_OVERLAY.into(), vec![1]);
        assert!(ramoops_refusals(&tree).is_empty());
        tree.remove(RAMOOPS_OVERLAY);
        tree.insert("default.txt".into(), b"# nothing\n".to_vec());
        assert_eq!(ramoops_refusals(&tree).len(), 2);
    }

    #[test]
    fn a_byte_changed_between_verify_and_serve_is_refused() {
        let bytes = b"kernel bytes".to_vec();
        let file = Served {
            path: "slots/bay0/kernel8.img".into(),
            sha256: sha256_hex(&bytes),
            bytes: bytes.clone(),
            role: "test".into(),
        };
        assert!(verify_served(std::slice::from_ref(&file)).is_ok());
        let mut changed = file.clone();
        changed.bytes[0] ^= 1;
        let problems = verify_served(&[changed]).expect_err("refused");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("slots/bay0/kernel8.img changed after it was verified"));
    }

    #[test]
    fn names_that_collide_ignoring_case_are_found() {
        let mut tree = fat::Tree::new();
        tree.insert("Kernel8.img".into(), vec![1]);
        tree.insert("kernel8.img".into(), vec![2]);
        tree.insert("config.txt".into(), vec![3]);
        let groups = fat::collisions(&tree);
        assert_eq!(
            groups,
            vec![vec!["Kernel8.img".to_string(), "kernel8.img".to_string()]]
        );
        let mut mixed = fat::Tree::new();
        mixed.insert("Kernel8.img".into(), vec![1]);
        assert!(fat::collisions(&mixed).is_empty());
        assert_eq!(
            fat::find(&mixed, "kernel8.IMG").map(|(name, _)| name),
            Some("Kernel8.img")
        );
    }
}
