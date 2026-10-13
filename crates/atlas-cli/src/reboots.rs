//! `atlas reboot-test`: restart a board N times and tell, for each restart,
//! how the boot before ended, from what the board records at boot
//! (boot.json, through its status): whether it shut down cleanly, what the
//! kernel restarted with, whether pstore kept anything (none: RAM was lost,
//! a power-level reset), the boot count (a jump means boots nobody saw), the
//! last journal lines of the boot before, and the supply reading it logged
//! at the start of its shutdown. Ends with a summary; `--json` prints one
//! object per restart.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use atlas_core::Atlas;
use atlas_driver::{BootInfo, DeviceKey};
use serde_json::json;

use crate::commands::{CommandResult, resolve};
use crate::output;

pub(crate) struct RebootOptions {
    pub(crate) device: String,
    pub(crate) count: u32,
    /// How long one restart may take to come back.
    pub(crate) wait: Duration,
    /// Time on the new boot before the next restart.
    pub(crate) settle: Duration,
    pub(crate) json: bool,
}

/// One restart, as the boot after it saw it.
struct Row {
    n: u32,
    seconds: Option<u64>,
    boot: Option<BootInfo>,
    jumped: Option<u64>,
    power: Option<String>,
    error: Option<String>,
}

async fn boot(atlas: &Atlas, key: &DeviceKey) -> Option<BootInfo> {
    atlas
        .device_status(key)
        .await
        .ok()
        .and_then(|status| status.boot)
}

/// The supply reading the board logged as the boot before shut down.
fn last_power(atlas: &Atlas, key: &DeviceKey, boot_id: Option<&str>) -> Option<String> {
    atlas
        .device_history(key, 400)
        .into_iter()
        .filter(|entry| entry.kind == "power" && entry.boot_id.as_deref() != boot_id)
        .max_by_key(|entry| entry.at_ms)
        .map(|entry| {
            entry
                .message
                .trim_start_matches("board supply at ")
                .to_string()
        })
}

pub(crate) async fn reboot_test(atlas: &Atlas, options: RebootOptions) -> CommandResult {
    atlas.scan().await;
    let key = resolve(atlas, &options.device)?;
    let mut before = boot(atlas, &key).await.ok_or_else(|| {
        format!(
            "{}: it doesn't report its boots (no status)",
            options.device
        )
    })?;
    let mut rows = Vec::new();
    for n in 1..=options.count {
        if !options.json {
            output::line(&format!("restart {n} of {}…", options.count));
        }
        let started = Instant::now();
        if let Err(error) = atlas.run_action(&key, "reboot").await {
            rows.push(Row {
                n,
                seconds: None,
                boot: None,
                jumped: None,
                power: None,
                error: Some(format!("restart: {error}")),
            });
            break;
        }
        // Back when its boot id is a new one.
        let after = loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            atlas.scan().await;
            if let Some(now) = boot(atlas, &key).await
                && now.id.is_some()
                && now.id != before.id
            {
                break Some(now);
            }
            if started.elapsed() >= options.wait {
                break None;
            }
        };
        let Some(after) = after else {
            rows.push(Row {
                n,
                seconds: None,
                boot: None,
                jumped: None,
                power: None,
                error: Some(format!("not back within {} s", options.wait.as_secs())),
            });
            break;
        };
        // History is fetched with the status; give it a moment.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let row = Row {
            n,
            seconds: Some(started.elapsed().as_secs()),
            jumped: match (before.count, after.count) {
                (Some(a), Some(b)) if b > a + 1 => Some(b - a - 1),
                _ => None,
            },
            power: last_power(atlas, &key, after.id.as_deref()),
            boot: Some(after.clone()),
            error: None,
        };
        if options.json {
            output::json(&row_json(&row));
        } else {
            output::line(&format!("  {}", summary(&row)));
        }
        rows.push(row);
        before = after;
        tokio::time::sleep(options.settle).await;
    }

    let done: Vec<&Row> = rows.iter().filter(|r| r.boot.is_some()).collect();
    let unclean = done
        .iter()
        .filter(|r| r.boot.as_ref().and_then(|b| b.previous_clean) == Some(false))
        .count();
    let lost = done
        .iter()
        .filter(|r| r.boot.as_ref().and_then(|b| b.pstore_saved) == Some(false))
        .count();
    let jumps: u64 = done.iter().filter_map(|r| r.jumped).sum();
    let failed = rows.iter().find_map(|r| r.error.clone());
    if options.json {
        output::json(&json!({
            "device": key, "restarts": done.len(), "unclean": unclean, "ram_lost": lost,
            "boots_unseen": jumps, "stopped": failed,
        }));
    } else {
        let table: Vec<Vec<String>> = rows.iter().map(table_row).collect();
        output::table(
            &[
                "#",
                "BACK",
                "COUNT",
                "CLEAN",
                "RESTARTED WITH",
                "PSTORE",
                "UNDERVOLT",
                "SUPPLY AT SHUTDOWN",
                "LAST JOURNAL LINE",
            ],
            &table,
        );
        output::line(&format!(
            "{} restarts: {unclean} not clean, {lost} with RAM lost (no pstore), {jumps} boots in between nobody saw{}",
            done.len(),
            failed
                .map(|f| format!("; stopped: {f}"))
                .unwrap_or_default()
        ));
    }
    Ok(
        if unclean == 0 && lost == 0 && rows.iter().all(|r| r.error.is_none()) {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        },
    )
}

fn yes_no(value: Option<bool>) -> String {
    match value {
        Some(true) => "yes".into(),
        Some(false) => "NO".into(),
        None => "?".into(),
    }
}

fn last_line(journal: Option<&str>) -> String {
    journal
        .and_then(|j| j.rsplit(" | ").next())
        .map(|line| line.chars().take(70).collect())
        .unwrap_or_else(|| "—".into())
}

fn table_row(row: &Row) -> Vec<String> {
    let Some(boot) = &row.boot else {
        return vec![
            row.n.to_string(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            row.error.clone().unwrap_or_default(),
        ];
    };
    vec![
        row.n.to_string(),
        row.seconds.map(|s| format!("{s} s")).unwrap_or_default(),
        match (boot.count, row.jumped) {
            (Some(c), Some(j)) => format!("{c} (+{j} unseen)"),
            (Some(c), None) => c.to_string(),
            _ => "?".into(),
        },
        yes_no(boot.previous_clean),
        match (&boot.previous_reboot, boot.pstore_saved) {
            (Some(r), _) if r.is_empty() => "restart".into(),
            (Some(r), _) => r.clone(),
            (None, Some(false)) => "(RAM lost)".into(),
            (None, _) => "?".into(),
        },
        yes_no(boot.pstore_saved),
        undervolt(boot.throttled.as_deref(), row.power.as_deref()),
        row.power.clone().unwrap_or_else(|| "—".into()),
        last_line(boot.previous_journal.as_deref()),
    ]
}

/// Undervoltage as the firmware flagged it: at the previous boot's shutdown
/// (its power event says so) or early in this one (boot.json's flags).
fn undervolt(throttled: Option<&str>, power: Option<&str>) -> String {
    let early = throttled
        .and_then(|flags| u32::from_str_radix(flags.trim_start_matches("0x"), 16).ok())
        .map(|bits| bits & 0x1_0001 != 0);
    let at_shutdown = power.map(|text| text.contains("UNDERVOLTAGE"));
    match (at_shutdown, early) {
        (Some(true), _) => "YES at shutdown".into(),
        (_, Some(true)) => "YES at boot".into(),
        (_, Some(false)) => "no".into(),
        _ => "?".into(),
    }
}

fn summary(row: &Row) -> String {
    table_row(row).join(" · ")
}

fn row_json(row: &Row) -> serde_json::Value {
    json!({
        "restart": row.n,
        "seconds": row.seconds,
        "boot": row.boot,
        "boots_unseen": row.jumped,
        "supply_at_shutdown": row.power,
        "error": row.error,
    })
}
