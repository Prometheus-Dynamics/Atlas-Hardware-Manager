# Working on the Atlas `next` branch

`next` is a clean-slate rebuild of Atlas. It started empty on purpose. `dev` is the frozen
reference, and nothing here changes it. Read [`docs/design.md`](docs/design.md) first.

## If you are picking this up

1. Read [`docs/design.md`](docs/design.md).
2. Run `python3 scripts/pr-status.py` to see what's merged, open, ready and blocked.
3. Take a PR marked ready and read its brief at `docs/prs/<id>.md`. The brief is the whole
   assignment.

[`TODO.md`](TODO.md) is the full list.

## Working a PR

- Base branch `next`, working branch `pr/<id>`, one PR into `next`.
- Read `dev` as a reference: `git show origin/dev:<path>`. Bring in only what the brief lists, and
  rework it to fit. Never check out another agent's checkout.
- Read Orion, Lemnos and Gaia `next` the same way. Orion `docs/design.md` and `docs/client.md` say
  how a board is reached. Gaia `docs/design.md` and its `examples/` say what a board description is.
- Stay inside the paths your PR creates. Don't edit the root `Cargo.toml`, `TODO.md` or a changelog.
- Add a crate by creating its directory under `crates/<folder>/`. Name it `atlas-<directory>`.
- If your PR needs a core interface changed (`crates/core/*`), stop and say so in the PR text. Core
  interfaces change only through A-series PRs.
- A need named `X<n>` is an outside block (see `docs/design.md`). Don't work around it and don't
  open PRs in other repositories for it.
- A PR merges when `scripts/gate.sh` passes and its own CI checks exit 0. Paste the proof named in
  the brief into the PR text.

## Rules

- One way to a board: Orion. No SSH commands, console commands, HTTP gateways or protocols of
  Atlas's own for board jobs.
- Atlas runs nothing on a board. Don't add a board-side program or script. If a board needs one,
  it is a Lemnos driver or an Orion feature: say so in the PR text.
- Loaders are our own Rust, no vendor tools, for every chip family.
- Board, product and module names appear only under `devices/` and in tests. Code reads them as
  data.
- No driver depends on another driver. One capability set, in the driver contract.
- The app never blocks on a device. Every Tauri command is async; scans and jobs run in the
  background and report by events.
- UI work starts from an approved mockup, and is proven with real pointer clicks against the
  simulated node or a live board, never by calling functions from a test.
- The app logs itself: UI errors, every failed call, breadcrumbs.
- No confirmation on a reversible action. A confirmation never changes the layout.
- Failure rule: never give up, backoff capped at 5 to 10 s, no default auto-off, an easy manual off.
- Lean: report the size of anything that lands on a board (the flasher image, the SystemCore
  package) and its change.
- Boards you can't reach are marked `untested on board`. Don't hold work back for one.
- Never claim something works without having run it. Report failures with their output.
- Never push tags. Never force-push `next` or `dev`. Don't change `dev`.
- Don't estimate how long anything will take. Describe size by scope and risk.
- Builds and scratch go on NVMe or SSD, never the Github HDD. Each worktree has its own
  `CARGO_TARGET_DIR`. Builds use `taskset -c 6-31` and `-j 13`.
- Never kill processes by name on this machine; only PIDs you started.

## Board runs

Briefs marked `Runs: board` need real hardware and run in a local session on the user's machine.
The Raze, the SystemCore chain and the ZERO 3E are on the bench. Board work on a Raze needs no
approval. Never reflash the SystemCore; installing a package on it is fine. For a board's USB
network use its own bound, never-default connection profile, never `nmcli dev connect`.
