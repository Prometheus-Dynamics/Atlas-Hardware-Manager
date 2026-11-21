//! `atlas-next usb-boot` against a fake agent and a real FAT16 boot partition (written with
//! `fatfs` in the test) and a bundle signed with a test seed. The fake agent records every
//! connection; a refusal must leave it with none.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use atlas_bundle::{BootFile, BundleError, SlotMeta, SlotSource, root_hash, write_bundle};
use atlas_image::{Verity, tree_bytes};
use atlas_next::{Env, execute};
use sha2::{Digest, Sha256};

const SIGN_SEED: [u8; 32] = [7; 32];
const OTHER_SEED: [u8; 32] = [9; 32];
const BAY_PATHS: [&str; 5] = [
    "kernel8.img",
    "initramfs.cpio",
    "board.dtb",
    "cmdline.txt",
    "overlays/lemnos-board.dtbo",
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The boot files of the slot, one per role, with their contents.
fn boot_contents() -> Vec<(String, Vec<u8>)> {
    vec![
        ("kernel8.img".into(), b"kernel bytes".to_vec()),
        ("initramfs.cpio".into(), b"initramfs bytes".to_vec()),
        ("board.dtb".into(), b"device tree bytes".to_vec()),
        (
            "cmdline.txt".into(),
            b"console=tty1 root=PARTUUID=abc panic=5\n".to_vec(),
        ),
        (
            "overlays/lemnos-board.dtbo".into(),
            b"overlay bytes".to_vec(),
        ),
    ]
}

/// A one-block slot: data, its verity values, the five boot files.
struct Slot {
    data: Vec<u8>,
    verity: Verity,
    boot: Vec<(String, Vec<u8>)>,
}

impl Slot {
    fn new(boot: Vec<(String, Vec<u8>)>) -> Self {
        let data = vec![0x5a; 4096];
        let salt = Sha256::digest(&data);
        let root = root_hash(&salt, &data).expect("root");
        let verity = Verity::from_manifest(&root, Some(&hex(&salt)), 1, 4096).expect("verity");
        Self { data, verity, boot }
    }
}

impl SlotSource for Slot {
    fn verity(&self) -> Result<Verity, BundleError> {
        Ok(self.verity.clone())
    }
    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        assert_eq!(tree_bytes(1), 0);
        Ok(Box::new(self.data.as_slice()))
    }
    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        Ok(self
            .boot
            .iter()
            .map(|(path, bytes)| BootFile {
                path: path.clone(),
                size: bytes.len() as u64,
            })
            .collect())
    }
    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        let (_, bytes) = self
            .boot
            .iter()
            .find(|(p, _)| p == path)
            .expect("a listed file");
        Ok(Box::new(bytes.as_slice()))
    }
}

fn bundle_bytes(seed: &[u8; 32], boot: Vec<(String, Vec<u8>)>) -> Vec<u8> {
    let slot = Slot::new(boot);
    let meta = SlotMeta {
        product: "fixture".into(),
        version: "1.0.0".into(),
        board: "fixture-board".into(),
        ..SlotMeta::default()
    };
    let mut out = Vec::new();
    write_bundle(&slot, "slot-a", &meta, seed, &mut out).expect("the bundle is written");
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// A bay record (`bayinfo.rec`): two valid copies of the same payload, as Lemnos writes them.
fn bayinfo(slot: &str, files: &[(u64, [u8; 32])]) -> Vec<u8> {
    let mut payload = vec![slot.len() as u8];
    payload.extend_from_slice(slot.as_bytes());
    for (len, hash) in files {
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(hash);
    }
    let mut out = Vec::new();
    for sequence in [1u64, 2] {
        let mut copy = vec![0u8; 512];
        copy[0..4].copy_from_slice(b"LMRC");
        copy[4] = 1;
        copy[5] = 6;
        copy[8..16].copy_from_slice(&sequence.to_le_bytes());
        copy[16..18].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        copy[18..18 + payload.len()].copy_from_slice(&payload);
        let crc = crc32(&copy[..508]);
        copy[508..512].copy_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&copy);
    }
    out
}

/// A FAT16 boot partition with the given files (paths with `/`), written by `fatfs`.
fn boot_partition(dir: &Path, files: &[(String, Vec<u8>)]) -> PathBuf {
    let path = dir.join("boot.img");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .expect("the image file");
    file.set_len(32 << 20).expect("sized");
    fatfs::format_volume(
        &mut file,
        fatfs::FormatVolumeOptions::new()
            .fat_type(fatfs::FatType::Fat16)
            .bytes_per_cluster(2048),
    )
    .expect("formatted");
    let fs = fatfs::FileSystem::new(file, fatfs::FsOptions::new()).expect("mounted");
    for (name, bytes) in files {
        let mut parts: Vec<&str> = name.split('/').collect();
        let leaf = parts.pop().expect("a name");
        let mut dir = fs.root_dir();
        for part in parts {
            dir = match dir.open_dir(part) {
                Ok(next) => next,
                Err(_) => dir.create_dir(part).expect("a directory"),
            };
        }
        dir.create_file(leaf)
            .expect("a file")
            .write_all(bytes)
            .expect("written");
    }
    drop(fs);
    path
}

/// The boot partition of a slot whose bay 0 stores `stored` (the bytes the record hashes), with
/// `padding` bytes of zeros after each file, as Lemnos pads a bay to its fixed size.
fn partition_files(
    slot: &str,
    stored: &[(String, Vec<u8>)],
    record_hashes: &[(u64, [u8; 32])],
    extra: Vec<(String, Vec<u8>)>,
) -> Vec<(String, Vec<u8>)> {
    let mut files = vec![
        (
            "config.txt".to_string(),
            b"kernel_watchdog_timeout=15\ninclude default.txt\ndtoverlay=ramoops-pi5\n".to_vec(),
        ),
        (
            "default.txt".to_string(),
            b"os_prefix=slots/bay0/\n".to_vec(),
        ),
        (
            "slots/bay0/bayinfo.rec".to_string(),
            bayinfo(slot, record_hashes),
        ),
        (
            "overlays/ramoops-pi5.dtbo".to_string(),
            b"ramoops overlay".to_vec(),
        ),
    ];
    for ((path, bytes), _) in stored.iter().zip(BAY_PATHS.iter()) {
        let mut padded = bytes.clone();
        padded.resize(bytes.len() + 64, 0);
        files.push((format!("slots/bay0/{path}"), padded));
    }
    files.extend(extra);
    files
}

fn record_hashes(boot: &[(String, Vec<u8>)]) -> Vec<(u64, [u8; 32])> {
    BAY_PATHS
        .iter()
        .map(|path| {
            let (_, bytes) = boot.iter().find(|(p, _)| p == path).expect("a role file");
            (bytes.len() as u64, Sha256::digest(bytes).into())
        })
        .collect()
}

/// One request the fake agent received.
#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A fake agent: reads each request whole, records it, and answers the boot-only API. Its job
/// serves `kernel8.img` and ends `done`.
struct Fake {
    address: String,
    seen: Arc<Mutex<Vec<Req>>>,
}

fn fake_agent() -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
    let address = listener.local_addr().expect("address").to_string();
    let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some(req) = read_request(&mut stream) else {
                continue;
            };
            let (status, body) = answer(&req, &record);
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    Fake { address, seen }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Req> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).ok()?;
    Some(Req {
        method,
        path,
        headers,
        body,
    })
}

fn answer(req: &Req, record: &Mutex<Vec<Req>>) -> (&'static str, String) {
    record.lock().expect("the record").push(req.clone());
    let json = |value: serde_json::Value| ("200 OK", value.to_string());
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/agent") => {
            json(serde_json::json!({"version": "test", "boot_files": true, "running": null}))
        }
        ("PUT", p) if p.starts_with("/v1/boot-only/files/") => json(serde_json::json!({})),
        ("POST", "/v1/boot-only") => json(serde_json::json!({"id": "job-1"})),
        ("GET", p) if p.starts_with("/v1/jobs/job-1") => {
            let from: usize = p
                .rsplit("from=")
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            if from == 0 {
                json(serde_json::json!({
                    "id": "job-1",
                    "events": [
                        {"event": "usb-step", "step": {"kind": "waiting"}},
                        {"event": "boot-file", "name": "kernel8.img", "served": true, "bytes": 12, "sha256": "x"},
                        {"event": "boot-end", "outcome": "done", "message": null}
                    ],
                    "next": 3,
                    "finished": true
                }))
            } else {
                json(
                    serde_json::json!({"id": "job-1", "events": [], "next": from, "finished": true}),
                )
            }
        }
        ("DELETE", _) => json(serde_json::json!({})),
        _ => ("404 Not Found", "{}".to_string()),
    }
}

struct Case {
    _dir: tempfile::TempDir,
    agent: Fake,
    bundle: PathBuf,
    pubkey: PathBuf,
    partition: PathBuf,
    bootfiles: PathBuf,
}

const SECOND_STAGE: &[u8] = b"second stage bytes";

fn case(bundle_seed: &[u8; 32], partition_files: Vec<(String, Vec<u8>)>) -> Case {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let bundle = dir.path().join("slot.tar");
    std::fs::write(&bundle, bundle_bytes(bundle_seed, boot_contents())).expect("bundle");
    let key = atlas_release::public_key_for(&SIGN_SEED);
    let pubkey = dir.path().join("bench.pub");
    std::fs::write(&pubkey, format!("{key}\n")).expect("pubkey");
    let partition = boot_partition(dir.path(), &partition_files);
    let bootfiles = dir.path().join("flasher");
    std::fs::create_dir_all(&bootfiles).expect("a flasher folder");
    std::fs::write(bootfiles.join("bootcode5.bin"), SECOND_STAGE).expect("second stage");
    Case {
        agent: fake_agent(),
        bundle,
        pubkey,
        partition,
        bootfiles,
        _dir: dir,
    }
}

async fn usb_boot(case: &Case, extra: &[&str]) -> (i32, String) {
    let mut args: Vec<String> = [
        "usb-boot",
        "--agent",
        &case.agent.address,
        "--port",
        "3-1.1.4.1",
        "--bundle",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(case.bundle.display().to_string());
    args.push("--boot-partition".into());
    args.push(case.partition.display().to_string());
    args.push("--pubkey".into());
    args.push(case.pubkey.display().to_string());
    args.push("--bootfiles".into());
    args.push(case.bootfiles.display().to_string());
    args.extend(extra.iter().map(|s| s.to_string()));
    let env = Env::new(None);
    let mut out = Vec::new();
    let code = execute(&args, &env, &mut out).await;
    (code, String::from_utf8(out).expect("text"))
}

fn sent(case: &Case) -> Vec<Req> {
    case.agent.seen.lock().expect("the record").clone()
}

fn sent_lines(case: &Case) -> Vec<String> {
    sent(case)
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect()
}

/// The files the job must serve: each path with the SHA-256 of the bytes served. A bay file is
/// served at its stored length, which is the content the manifest hashes, not its padded size.
fn expected_served(files: &[(String, Vec<u8>)]) -> std::collections::BTreeMap<String, String> {
    let mut map: std::collections::BTreeMap<String, String> = files
        .iter()
        .map(|(path, bytes)| {
            let stored = boot_contents()
                .into_iter()
                .find(|(role, _)| path.ends_with(role.as_str()) && path.contains("/bay0/"))
                .map_or(bytes.clone(), |(_, content)| content);
            (path.clone(), sha(&stored))
        })
        .collect();
    map.insert("bootcode5.bin".into(), sha(SECOND_STAGE));
    map
}

#[tokio::test]
async fn a_bay_file_that_differs_is_refused_with_both_hashes_and_nothing_is_sent() {
    let boot = boot_contents();
    let mut stored = boot.clone();
    // The partition's kernel differs from the bundle's; the record is consistent with it.
    stored[0].1 = b"a different kernel".to_vec();
    let hashes = record_hashes(&stored);
    let partition = partition_files("slot-a", &stored, &hashes, vec![]);
    let case = case(&SIGN_SEED, partition);
    let (code, out) = usb_boot(&case, &["--dry-run"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("REFUSED"), "{out}");
    assert!(out.contains("kernel8.img"), "{out}");
    assert!(
        out.contains(&sha(b"kernel bytes")),
        "bundle hash missing: {out}"
    );
    assert!(
        out.contains(&sha(b"a different kernel")),
        "partition hash missing: {out}"
    );
    assert!(
        sent(&case).is_empty(),
        "the agent received {:?}",
        sent(&case)
    );
}

#[tokio::test]
async fn a_bundle_that_fails_its_signature_is_refused_and_nothing_is_sent() {
    let boot = boot_contents();
    let partition = partition_files("slot-a", &boot, &record_hashes(&boot), vec![]);
    let case = case(&OTHER_SEED, partition);
    let (code, out) = usb_boot(&case, &[]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("the bundle is refused"), "{out}");
    assert!(
        sent(&case).is_empty(),
        "the agent received {:?}",
        sent(&case)
    );
}

#[tokio::test]
async fn a_partition_without_the_ramoops_line_is_refused_and_nothing_is_sent() {
    let boot = boot_contents();
    let mut partition = partition_files("slot-a", &boot, &record_hashes(&boot), vec![]);
    partition[0].1 = b"kernel_watchdog_timeout=15\ninclude default.txt\n".to_vec();
    partition.retain(|(name, _)| name != "overlays/ramoops-pi5.dtbo");
    let case = case(&SIGN_SEED, partition);
    let (code, out) = usb_boot(&case, &[]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("REFUSED"), "{out}");
    assert!(out.contains("dtoverlay=ramoops-pi5"), "{out}");
    assert!(out.contains("pstore"), "{out}");
    assert!(out.contains("overlays/ramoops-pi5.dtbo"), "{out}");
    assert!(
        sent(&case).is_empty(),
        "the agent received {:?}",
        sent(&case)
    );
}

#[tokio::test]
async fn matching_files_are_uploaded_with_exact_paths_and_hashes_then_the_job_is_followed() {
    let boot = boot_contents();
    let files = partition_files("slot-a", &boot, &record_hashes(&boot), vec![]);
    let want = expected_served(&files);
    let case = case(&SIGN_SEED, files);
    let (code, out) = usb_boot(&case, &[]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("boot-end done"), "{out}");
    assert!(out.contains("waiting"), "{out}");

    let requests = sent(&case);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1/agent");
    let mut uploaded = std::collections::BTreeMap::new();
    for req in requests.iter().filter(|r| r.method == "PUT") {
        let path = req
            .path
            .strip_prefix("/v1/boot-only/files/")
            .expect("the upload path");
        let header = req
            .headers
            .iter()
            .find(|(n, _)| n == "x-sha256")
            .map(|(_, v)| v.clone())
            .expect("an x-sha256 header");
        assert_eq!(
            header,
            sha(&req.body),
            "the header is the body's hash for {path}"
        );
        uploaded.insert(path.to_string(), header);
    }
    assert_eq!(
        uploaded, want,
        "every served file is uploaded with its exact path and hash"
    );

    let post = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/boot-only")
        .expect("a POST");
    let body: serde_json::Value = serde_json::from_slice(&post.body).expect("json");
    assert_eq!(body["port"], "3-1.1.4.1");
    let posted: std::collections::BTreeMap<String, String> = body["files"]
        .as_object()
        .expect("files")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().expect("a hash").to_string()))
        .collect();
    assert_eq!(posted, want, "the job names every file by path and hash");
    assert!(body.get("timeout_s").is_none());
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.path.starts_with("/v1/jobs/job-1"))
    );
}

#[tokio::test]
async fn a_mixed_case_real_name_is_uploaded_as_it_is_and_matched_by_the_job() {
    let boot = boot_contents();
    let mut files = partition_files("slot-a", &boot, &record_hashes(&boot), vec![]);
    for (name, _) in files.iter_mut() {
        if name == "slots/bay0/kernel8.img" {
            *name = "slots/bay0/Kernel8.img".into();
        }
    }
    let case = case(&SIGN_SEED, files);
    let (code, out) = usb_boot(&case, &[]).await;
    assert_eq!(code, 0, "{out}");
    let requests = sent(&case);
    assert!(
        requests
            .iter()
            .any(|r| r.method == "PUT" && r.path == "/v1/boot-only/files/slots/bay0/Kernel8.img"),
        "{:?}",
        sent_lines(&case)
    );
    let post = requests
        .iter()
        .find(|r| r.method == "POST")
        .expect("a POST");
    let body: serde_json::Value = serde_json::from_slice(&post.body).expect("json");
    assert_eq!(
        body["files"]["slots/bay0/Kernel8.img"],
        sha(b"kernel bytes")
    );
    assert!(body["files"].get("slots/bay0/kernel8.img").is_none());
}

#[tokio::test]
async fn the_dry_run_lists_every_file_and_sends_nothing() {
    let boot = boot_contents();
    let files = partition_files("slot-a", &boot, &record_hashes(&boot), vec![]);
    let want = expected_served(&files);
    let case = case(&SIGN_SEED, files);
    let (code, out) = usb_boot(&case, &["--dry-run"]).await;
    eprintln!("{out}");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("usb-boot plan"), "{out}");
    assert!(out.contains("slot       slot-a"), "{out}");
    assert!(out.contains("dry run: nothing was sent"), "{out}");
    for (path, hash) in &want {
        assert!(out.contains(path.as_str()), "the plan names {path}: {out}");
        assert!(
            out.contains(hash.as_str()),
            "the plan carries the hash of {path}: {out}"
        );
    }
    assert!(
        sent(&case).is_empty(),
        "the agent received {:?}",
        sent_lines(&case)
    );
}

#[tokio::test]
async fn a_partition_with_no_bay_for_the_slot_is_refused() {
    let boot = boot_contents();
    let partition = partition_files("other-slot", &boot, &record_hashes(&boot), vec![]);
    let case = case(&SIGN_SEED, partition);
    let (code, out) = usb_boot(&case, &[]).await;
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("no bay set of the boot partition holds slot \"slot-a\""),
        "{out}"
    );
    assert!(
        sent(&case).is_empty(),
        "the agent received {:?}",
        sent(&case)
    );
}

// Lemnos 7d91d645's layout: the board's firmware overlays live in the bay (with its overlays/README),
// the bay record lists them after the five roles, and config.txt loads ramoops with parameters.

const RAMOOPS: &[u8] = b"ramoops overlay bytes";

/// The bundle's boot files at 7d91d645: the five roles, then the firmware overlay.
fn boot_with_firmware() -> Vec<(String, Vec<u8>)> {
    let mut boot = boot_contents();
    boot.push(("overlays/ramoops-pi5.dtbo".into(), RAMOOPS.to_vec()));
    boot
}

/// A bay record as Lemnos encodes it (`bays::Info`, `twocopy::encode`), firmware overlays included.
fn lemnos_bayinfo(slot: &str, boot: &[(String, Vec<u8>)]) -> Vec<u8> {
    use lemnos_slot_store::bays::Info;
    use lemnos_slot_store::twocopy::{self, Kind, SECTOR};
    let entry = |path: &str| {
        let (_, bytes) = boot.iter().find(|(p, _)| p == path).expect("a boot file");
        (bytes.len() as u64, <[u8; 32]>::from(Sha256::digest(bytes)))
    };
    let info = Info {
        slot: slot.into(),
        files: std::array::from_fn(|i| entry(BAY_PATHS[i])),
        firmware: boot
            .iter()
            .filter(|(p, _)| !BAY_PATHS.contains(&p.as_str()))
            .map(|(p, _)| {
                let (len, hash) = entry(p);
                (p.clone(), len, hash)
            })
            .collect(),
    };
    let mut out = Vec::new();
    for seq in [1u64, 2] {
        let mut sector = [0u8; SECTOR];
        twocopy::encode(Kind::BAY_INFO, seq, &info.encode(), &mut sector).expect("encoded");
        out.extend_from_slice(&sector);
    }
    out
}

/// The 7d91d645 boot partition of `slot`: every bay member zero-padded after its stored bytes, the
/// way Lemnos pads a bay (the Nov 20 halt's form), the overlay only in the bay.
fn lemnos_partition(slot: &str, boot: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut files = vec![
        (
            "config.txt".to_string(),
            b"kernel_watchdog_timeout=15\ninclude default.txt\ndtoverlay=ramoops-pi5,total-size=0x40000,record-size=0x10000\n".to_vec(),
        ),
        ("default.txt".to_string(), b"os_prefix=slots/bay0/\n".to_vec()),
        ("slots/bay0/bayinfo.rec".to_string(), lemnos_bayinfo(slot, boot)),
        ("slots/bay0/overlays/README".to_string(), b"bay overlays\n".to_vec()),
    ];
    for (path, bytes) in boot {
        let mut padded = bytes.clone();
        padded.resize(bytes.len() + 4096, 0);
        files.push((format!("slots/bay0/{path}"), padded));
    }
    files
}

fn case_with(boot: Vec<(String, Vec<u8>)>, partition_files: Vec<(String, Vec<u8>)>) -> Case {
    let case = case(&SIGN_SEED, partition_files);
    std::fs::write(&case.bundle, bundle_bytes(&SIGN_SEED, boot)).expect("bundle");
    case
}

#[tokio::test]
async fn the_7d91d645_layout_passes_in_both_forms() {
    let boot = boot_with_firmware();
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    let (code, out) = usb_boot(&case, &["--dry-run"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("slots/bay0/overlays/ramoops-pi5.dtbo"),
        "{out}"
    );
    let (code, out) = usb_boot(&case, &["--dry-run", "--bay-exact"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("form       BAY-EXACT"), "{out}");
    assert!(sent(&case).is_empty(), "{:?}", sent_lines(&case));
}

#[tokio::test]
async fn a_bundle_overlay_the_bay_record_lacks_is_refused() {
    let boot = boot_with_firmware();
    // The partition's record holds the five roles only; the bundle also lists the overlay.
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot_contents()));
    let (code, out) = usb_boot(&case, &["--dry-run"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("but bay0's record holds"), "{out}");
    assert!(sent(&case).is_empty(), "{:?}", sent_lines(&case));
}

#[tokio::test]
async fn bay_exact_uploads_the_image_byte_for_byte_with_the_ramdisk_config() {
    let boot = boot_with_firmware();
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    let image = std::fs::read(&case.partition).expect("the image");
    let (code, out) = usb_boot(&case, &["--bay-exact"]).await;
    assert_eq!(code, 0, "{out}");
    let requests = sent(&case);
    let puts: std::collections::BTreeMap<String, Vec<u8>> = requests
        .iter()
        .filter(|r| r.method == "PUT")
        .map(|r| {
            (
                r.path
                    .strip_prefix("/v1/boot-only/files/")
                    .expect("path")
                    .to_string(),
                r.body.clone(),
            )
        })
        .collect();
    assert_eq!(
        puts.keys().cloned().collect::<Vec<_>>(),
        vec!["boot.img", "bootcode5.bin", "config.txt"]
    );
    assert!(
        puts["boot.img"] == image,
        "boot.img is the partition image byte for byte"
    );
    assert_eq!(puts["config.txt"], b"boot_ramdisk=1\n");
    assert_eq!(puts["bootcode5.bin"], SECOND_STAGE);
}

#[tokio::test]
async fn a_kernel_less_partition_is_refused_by_name_unless_it_is_the_control() {
    let boot = boot_with_firmware();
    let mut files = lemnos_partition("slot-a", &boot);
    files.retain(|(p, _)| p != "slots/bay0/kernel8.img");
    let case = case_with(boot.clone(), files);
    // Step 0(a): refused by name, nothing sent.
    let (code, out) = usb_boot(&case, &["--bay-exact"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("kernel8.img is missing from bay0"), "{out}");
    assert!(sent(&case).is_empty(), "{:?}", sent_lines(&case));
    // The control needs the BAY-EXACT form.
    let (code, out) = usb_boot(&case, &["--control", "kernel-less"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("needs --bay-exact"), "{out}");
    assert!(sent(&case).is_empty(), "{:?}", sent_lines(&case));
    // As the control it is served, and labelled.
    let (code, out) = usb_boot(
        &case,
        &["--dry-run", "--bay-exact", "--control", "kernel-less"],
    )
    .await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("CONTROL kernel-less"), "{out}");
    assert!(sent(&case).is_empty(), "{:?}", sent_lines(&case));
}

#[tokio::test]
async fn the_control_refuses_a_partition_that_has_its_kernel_or_fails_otherwise() {
    let boot = boot_with_firmware();
    let whole = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    let (code, out) = usb_boot(
        &whole,
        &["--dry-run", "--bay-exact", "--control", "kernel-less"],
    )
    .await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("and it has it"), "{out}");
    // Kernel-less AND without the ramoops line: the control does not cover the second fault.
    let mut files = lemnos_partition("slot-a", &boot);
    files.retain(|(p, _)| p != "slots/bay0/kernel8.img");
    files[0].1 = b"include default.txt\n".to_vec();
    let both = case_with(boot, files);
    let (code, out) = usb_boot(
        &both,
        &["--dry-run", "--bay-exact", "--control", "kernel-less"],
    )
    .await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("fails other checks too"), "{out}");
    assert!(out.contains("ramoops"), "{out}");
}

// `--serve-dir`: the checked BAY-EXACT set written for Raspberry Pi's `rpiboot -d` on this PC.

async fn serve_dir(case: &Case, dir: &Path, extra: &[&str]) -> (i32, String) {
    let mut args: Vec<String> = vec![
        "usb-boot".into(),
        "--serve-dir".into(),
        dir.display().to_string(),
        "--port".into(),
        "3-4.2.1.4.4.2".into(),
        "--bundle".into(),
        case.bundle.display().to_string(),
        "--boot-partition".into(),
        case.partition.display().to_string(),
        "--pubkey".into(),
        case.pubkey.display().to_string(),
        "--bootfiles".into(),
        case.bootfiles.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let env = Env::new(None);
    let mut out = Vec::new();
    let code = execute(&args, &env, &mut out).await;
    (code, String::from_utf8(out).expect("text"))
}

#[tokio::test]
async fn serve_dir_writes_the_image_the_outer_config_and_the_whole_stage() {
    let boot = boot_with_firmware();
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    std::fs::write(case.bootfiles.join("bootmain"), b"bootmain bytes").expect("bootmain");
    let image = std::fs::read(&case.partition).expect("the image");
    let dir = case.bootfiles.parent().expect("scratch").join("serve");
    let (code, out) = serve_dir(&case, &dir, &["--bay-exact"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("rpiboot -v -d"), "{out}");
    assert!(out.contains("-p 3-4.2.1.4.4.2"), "{out}");
    assert!(std::fs::read(dir.join("boot.img")).expect("boot.img") == image);
    assert_eq!(
        std::fs::read(dir.join("config.txt")).expect("config"),
        b"boot_ramdisk=1\n"
    );
    assert_eq!(
        std::fs::read(dir.join("2712/bootcode5.bin")).expect("stage"),
        SECOND_STAGE
    );
    assert_eq!(
        std::fs::read(dir.join("2712/bootmain")).expect("bootmain"),
        b"bootmain bytes"
    );
    assert!(
        !dir.join("bootcode5.bin").exists(),
        "the stage goes under 2712/ only"
    );
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).expect("sums");
    assert!(
        sums.contains(&format!("{}  boot.img", sha(&image))),
        "{sums}"
    );
    assert!(
        sent(&case).is_empty(),
        "nothing goes to an agent: {:?}",
        sent_lines(&case)
    );
    // A second run never writes into an existing directory.
    let (code, out) = serve_dir(&case, &dir, &["--bay-exact"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("must be new"), "{out}");
}

#[tokio::test]
async fn serve_dir_refuses_an_eeprom_file_and_the_tree_form_and_writes_nothing() {
    let boot = boot_with_firmware();
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    let dir = case.bootfiles.parent().expect("scratch").join("serve");
    let (code, out) = serve_dir(&case, &dir, &[]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("--serve-dir needs --bay-exact"), "{out}");
    assert!(!dir.exists());
    std::fs::write(case.bootfiles.join("recovery.bin"), b"eeprom updater").expect("recovery");
    let (code, out) = serve_dir(&case, &dir, &["--bay-exact"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("recovery.bin, an EEPROM file"), "{out}");
    assert!(!dir.exists());
}

#[tokio::test]
async fn agent_and_serve_dir_together_are_refused() {
    let boot = boot_with_firmware();
    let case = case_with(boot.clone(), lemnos_partition("slot-a", &boot));
    let dir = case.bootfiles.parent().expect("scratch").join("serve");
    let (code, out) = serve_dir(
        &case,
        &dir,
        &["--bay-exact", "--agent", &case.agent.address],
    )
    .await;
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("not both"), "{out}");
    assert!(!dir.exists());
}
