# Plan: board data through Orion, streamed live (for review, nothing built yet)

Goal (from the user, via HeliOS): live values in Atlas at about 10 Hz or the
source rate, all the time a page is open, over one persistent connection per
board; graphs full the moment a page opens; under 1% of one board core while
streaming to one app; no forks in steady state; actions through Orion; the
security switch behaving as it does now.

## What Orion can and can't do today (Orion f3efb26, the version Atlas pins)

Checked in the Orion source:

- **No push to remote clients.** Watches (`WatchStatus`, `WatchState`,
  `WatchActions`, `PollClientEvents`) work only over the node's local Unix
  socket. A remote operator over `orion+tcp` gets request/response only;
  `RemoteOperator::watch_status` polls (500 ms default, 50 ms minimum), one
  signed request per poll, and returns the whole matching set, not a delta.
  Orion's TODO lists "push-based watches over orion+tcp" as future work.
- **High-rate data is out of Orion by design.** Its TODO: "Bulk and high-rate
  data (frames, detections) stays out of Orion and uses resource endpoints."
  The status lane keeps only the latest value per key (4096 entries,
  1 KB values), with no history and no sequence numbers.
- **Actions work remotely.** `RemoteOperator::call_action` is request/response
  with server-side wait. board-agent already claims node actions
  (update, cancel, rollback, reboot, locate, clock.set, usb-route.set).
- **orion-node has no network listener on the board.** `ORION_NODE_PEER_ADDR`
  is unset by default, which matches what HeliOS saw.
- **Auth:** an operator is an ed25519 key enrolled on the node, by admin
  approval or with a shared enrollment key. Policy is read on or off, plus
  action patterns. An anonymous caller can't `QueryStatus` or `RunAction`.
  `orion+tcp` has integrity but no encryption.

So "fast streaming through Orion" can't be a stream over Orion's own
protocol without changing Orion. The plan uses Orion the way it's designed:
Orion is where the board is found, where trust is decided and where actions
run. The live data travels on the board's own resource endpoint, which Orion
advertises and authorizes.

## Shape

```
Atlas app ──orion+tcp (one connection)──▶ orion-node   identity, facts, status, actions
    │                                         │ local IPC (push watches work here)
    │                                       board-agent  (one long-running process, Rust)
    └──── live stream (one TCP connection) ──▶ board-agent's live endpoint
                                                 readings: lemnosd IPC, /sys, /proc
                                                 history ring per series
```

board-agent is already the board's long-running Rust process and Orion
client. It gains the live endpoint and takes over everything the shell
scripts (`status`, `identity-http`) and the per-connection `board-stream`
units do today.

## Data model

**Series.** A reading has an id (`cpu-thermal.temp`, `cpu`, `cpu.core3`,
`memory`, `fan.rpm`, `fps`, `power.voltage`, `imu.accel_x`, `led.0`…), a unit,
a kind (gauge, counter or colour) and a native rate.

Sources, all read in Rust with no process started:
- **lemnosd devices:** the IMU, power, fan and LEDs, through the
  `lemnos_ipc` client board-stream already uses. They arrive at device rate.
- **/sys and /proc**, sampled at 10 Hz:
  - thermal zones;
  - CPU from `/proc/stat`, per core;
  - memory from `/proc/meminfo`;
  - uptime;
  - network bytes from `/sys/class/net/*/statistics`.
- **PhotonVision fps and latency**, from its NT topics. board-agent could read
  them with `orion-nt4`; that's the one source still to be designed.

**History ring per series**, in board-agent:
- **Gauges, 10 Hz:** 5 minutes at full rate, which is 3,000 samples.
- **Fast sources** (the IMU at about 100 Hz): 30 s at full rate, plus the same
  5 minutes downsampled to 10 Hz.
- **Encoding:** a sample is a time delta (u16, in units of 100 µs) and a value
  (f32), so 6 bytes. About 30 series × 3,000 × 6 bytes plus the IMU's 6
  channels at full rate comes to about 1 MB. HeliOS can choose a smaller ring
  if memory is tight.

**Live protocol**, on one TCP connection per board:
- **Framing:** length-prefixed postcard frames (the encoding Orion's MCU link
  already uses), with no JSON on the hot path.
- **Client → board:**
  - `Hello{token}` opens the session.
  - `Subscribe{series: pattern list, rate cap, since_seq?}` picks the data.
  - `Unsubscribe` stops it.
- **Board → client:**
  - `Catalog{series…}` lists the series.
  - `Backfill{seq_end, per series: t0 and packed samples}` arrives once, right
    after `Subscribe`. That's what fills a graph in a single message.
  - `Batch{seq, t0, per series: packed samples since the last batch}` is sent
    every 50 ms while there's anything new.
  - Slow fields go only when they change: `Changed{key, value}` for status,
    update state and the newest event.
  - `Ping` is a keep-alive every 10 s.
- **Resume:** `since_seq` within the ring sends only what was missed. A gap
  (the ring has moved on, or the board restarted, which the boot id tells
  apart) sends a fresh `Backfill`.
- **Nothing is sampled for the network** while no client is subscribed.
  lemnosd subscriptions are dropped too. The /sys and /proc sampling that
  feeds the ring can keep running at 10 Hz, since it costs microseconds. That
  way a page opened later still gets 5 minutes of history. Say if you'd
  rather it stopped as well.

## Mapping from today's endpoints

| Today (board-http :5899, board-stream :5898) | After |
|---|---|
| `GET /.well-known/pd-device` (identity) | Discovery stays: mDNS (avahi, as now) plus `pd-device`, served from memory by board-agent's listener with no unit per request. Identity facts go to Orion `NodeHostFacts` (hostname, image, kernel, board model and serial already exist) and the identity's extra fields (slots, camera views, the gadget network) to the status lane under `board.*`. |
| `GET /status` (temps, fan, failed units, update, clock, drift, hardware) | Readings become live series. Slow state (failed units, update, clock, drift, boot record) becomes `Changed` frames on the live connection, and the same keys on Orion's status lane, written only on change, for Orion-only readers. |
| `GET /events?after_seq=` | Events stay numbered: `Changed{event}` on the live connection, plus a `board.events` action that returns a page after a seq for catch-up. |
| board-stream `/stream` (hardware at device rate, change pushes) | Folded into the live endpoint, which already uses the same lemnosd client and batching. |
| `POST /actions/<id>` (reboot, usb-boot, locate…) | Orion actions claimed by board-agent: today's seven plus `usb-boot`, `power-off`, `ssh-key.add`, hardware actions (`hardware.<device>.<name>`) and `support-bundle`. |

## What changes where

**orion-node (Orion's owner, configuration only for this plan):**
- On images that want it, set `ORION_NODE_PEER_ADDR` to listen on the robot
  and USB networks, port 9200.
- Map the security switch to `ORION_NODE_PEER_AUTH` (below).
- Later, from Orion's owner: push watches over `orion+tcp` (their planned
  item). Atlas would then read the status lane by push, not by poll. This
  plan doesn't depend on it.

**board-agent (this repo):**
- **Live endpoint:**
  - `live/`: series registry, sampler, ring, encoder.
  - One listener, socket-activated with `Accept=no` (or started at boot), so
    there's one process and no unit per connection.
- **Sources:** lemnos (moved from board-stream), sysfs/proc readers, and the
  PhotonVision NT reader.
- **Status lane:** board-agent publishes `board.*` when values change. This
  replaces the `status` and `identity` scripts on the hot path; the scripts
  stay for SSH and the support bundle.
- **New claimed actions**, listed in the mapping table.
- **Discovery:** `/.well-known/pd-device` from memory on the same listener.
  board-http and board-stream stay installed for older Atlas versions, with
  their own units, until we retire them.

**Atlas:**
- **atlas-driver-board:** a `LiveClient` per board, one connection that
  reconnects and resumes by seq. Its ring holds what was received. It feeds
  telemetry, the hardware stream and the status pushes. Polling remains only
  as the fallback for boards without the endpoint.
- **atlas-driver-orion:** actions and status through `RemoteOperator`, which
  exists today. Once trust is set up, the app prefers Orion.
- **App:** the live store becomes push-driven. A view subscribes, gets the
  backfill, then batches over a Tauri channel. Charts draw from typed arrays,
  so 10 Hz across several charts is cheap. Graphs are full on open because of
  the backfill.

## Security (the same rules as now)

- **Switch off, as FRC robots run:**
  - **Live endpoint:** open read-only to the robot and USB networks. It's no
    more than what `/status` serves unauthenticated today.
  - **Orion:** `ORION_NODE_PEER_AUTH=optional`.
  - **Actions:** they need an enrolled operator, because Orion requires it.
    To keep reboot and usb-boot working from the USB cable without setup,
    board-agent auto-approves enrollment from the USB-link subnet while the
    switch is off. That's the same trust board-http gives that link today.
    It needs a small enrollment hook from Orion; until then, the existing
    board-http USB-link actions stay as the fallback.
- **Switch on:**
  - **Orion:** `ORION_NODE_PEER_AUTH=required`, so only enrolled operators.
  - **Live endpoint:** `Hello` must carry a ticket from the Orion action
    `live.ticket`, which only an enrolled operator can run. The ticket is a
    short-lived token board-agent issues and checks.
  - **USB-link exceptions:** none, as now.
- `orion+tcp` and the live stream are integrity-only, not encrypted, as Orion
  documents. The robot network is a trusted network.

## Measurement

On a board with this app connected, streaming at 10 Hz:
- **Board CPU and forks:** `/proc/<pid>/stat` deltas for board-agent,
  orion-node and systemd, plus a fork counter from `/proc/stat` processes.
- **Ring memory:** RSS of board-agent.
- **Time from opening a device page to a full graph:** the app logs from
  subscribe to the first backfill drawn. Target: under 0.5 s.

Same as the fork test now: a board-agent test that counts allocations or
forks per batch, so a regression shows up.

## Order of work

1. **board-agent live endpoint and the Atlas `LiveClient`**, behind an app
   setting, with polling as the fallback. This is most of the felt
   difference: 10 Hz, instant graphs, no forks.
2. **Actions and status through Orion**, and `NodeHostFacts` for identity.
3. **pd-device from board-agent**; board-http and board-stream units left only
   for old apps.
4. **When Orion ships push over tcp:** move the slow keys to it, and the live
   endpoint carries only samples.

## Stopgap shipped meanwhile

49e04c8 (on dev): live readings every 1 s on a device page and Monitor, 2 s
in lists, 15 minutes kept. The board side (bb1c277) makes each request
3 forks, but systemd still starts a unit per connection, so it's about 1–2
requests a second while a device page is open. Only the plan above gets to
zero.
