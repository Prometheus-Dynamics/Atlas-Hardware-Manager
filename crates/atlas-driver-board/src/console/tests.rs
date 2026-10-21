use super::*;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

/// A shell on pipes, as the board's console is (without the tty).
struct Shell {
    child: Child,
    out: Receiver<Vec<u8>>,
}

impl Shell {
    fn new() -> Self {
        let mut child = Command::new("sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, out) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self { child, out }
    }
}

impl Port for Shell {
    fn read_some(&mut self, until: Instant) -> std::io::Result<Vec<u8>> {
        let wait = until.saturating_duration_since(Instant::now());
        Ok(self.out.recv_timeout(wait).unwrap_or_default())
    }
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        // The shell ignores Ctrl-C as a byte; drop it as the tty would.
        let bytes: Vec<u8> = bytes.iter().copied().filter(|b| *b != 3).collect();
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(&bytes)?;
        stdin.flush()
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn has_base64() -> bool {
    Command::new("sh")
        .args(["-c", "command -v base64"])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn base64_round_trips() {
    for text in ["", "a", "ab", "abc", "abcd", "héllo\n\0\u{7f}"] {
        let encoded = base64_encode(text.as_bytes());
        assert_eq!(
            base64_decode(&encoded).unwrap(),
            text.as_bytes(),
            "{text:?}"
        );
    }
    assert_eq!(base64_encode(b"Man"), "TWFu");
    assert_eq!(base64_encode(b"Ma"), "TWE=");
    assert!(base64_decode("@@").is_none());
}

#[test]
fn escapes_are_stripped_and_a_login_prompt_is_seen() {
    let mut lines = Lines::default();
    let got = lines.push(b"\x1b[?2004h\x1b[0m# \x1b[?2004l\r@Pab\r\nnext\x1b]0;title\x07 ok\n");
    assert_eq!(got, vec!["# @Pab".to_owned(), "next ok".to_owned()]);
    assert!(!lines.at_login());
    lines.push(b"\r\nraze login: ");
    assert!(lines.at_login());
}

#[test]
fn markers_never_appear_in_what_is_sent() {
    let tag = "t1";
    for line in command_lines("echo '@Et1 x'; printf hi", tag, Duration::from_secs(60)) {
        for marker in ["@Et1", "@Bt1", "@Rt1", "@Pt1"] {
            assert!(!line.contains(marker), "{line}");
        }
        assert!(line.len() < 1024, "short lines only");
    }
}

#[test]
fn a_script_runs_with_its_stdout_stderr_and_status() {
    if !has_base64() {
        return;
    }
    let mut shell = Shell::new();
    // Quotes, long output, and stderr as it comes.
    let script = "echo \"it's here\"; echo step >&2; i=0; while [ $i -lt 500 ]; do echo line $i; i=$((i+1)); done";
    let seen = Mutex::new(Vec::new());
    let on_line = |line: &str| seen.lock().unwrap().push(line.to_owned());
    let out = exchange(
        &mut shell,
        script,
        "a1",
        Duration::from_secs(10),
        Some(&on_line),
    )
    .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.starts_with("it's here\nline 0\n"));
    assert!(out.ends_with("line 499\n"));
    assert_eq!(*seen.lock().unwrap(), vec!["step".to_owned()]);

    // The same shell, the next command: a failure and its reason.
    let error = exchange(
        &mut shell,
        "echo nope >&2; exit 3",
        "a2",
        Duration::from_secs(10),
        None,
    )
    .unwrap_err();
    match error {
        ConsoleError::Failed(DriverError::Other(reason)) => assert_eq!(reason, "nope"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn binary_output_comes_back_whole() {
    if !has_base64() {
        return;
    }
    let mut shell = Shell::new();
    let out = exchange(
        &mut shell,
        "printf '\\000\\377\\n\\001'",
        "b1",
        Duration::from_secs(10),
        None,
    )
    .unwrap();
    assert_eq!(out, [0, 255, b'\n', 1]);
}

#[test]
fn a_silent_console_is_not_there_and_nothing_runs() {
    struct Silent(Vec<u8>);
    impl Port for Silent {
        fn read_some(&mut self, until: Instant) -> std::io::Result<Vec<u8>> {
            std::thread::sleep(
                until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            );
            Ok(Vec::new())
        }
        fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.0.extend_from_slice(bytes);
            Ok(())
        }
    }
    let mut port = Silent(Vec::new());
    let started = Instant::now();
    let error = exchange(&mut port, "reboot", "s1", Duration::from_secs(10), None).unwrap_err();
    assert!(matches!(error, ConsoleError::NotThere(_)), "{error:?}");
    assert!(started.elapsed() < PROBE + Duration::from_secs(1));
    let sent = String::from_utf8_lossy(&port.0);
    assert!(
        !sent.contains("atlas-s1"),
        "only the probe was sent: {sent}"
    );
}
