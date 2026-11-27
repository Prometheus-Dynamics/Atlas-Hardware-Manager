# Sourced by scripts/gate.sh, scripts/ui-gate.sh and scripts/check-scoped.sh: the build environment the gates share,
# and the result cache that keeps a gate from running a step whose result is already known.
#
# The result cache (no ceremony runs). A lane's key is the SHA-256 of the bytes of every file it can read (tracked and
# untracked, not ignored) under its paths, plus the lane's name and command and the tools that decide its result. A
# lane that passed with a key keeps a marker named by that key in "$CARGO_TARGET_DIR/.gate-pass/". The next run with
# the same key reuses the pass and says so; any changed input gives a new key, so the lane runs. Only passes are kept:
# a failure always runs again. ATLAS_GATE_FRESH=1 runs every lane whatever the markers say.

# What a test can read: the whole tree but the prose. Markdown is not read by any build or test (include_str! names
# none), so a docs-only change (docs/, TODO.md, a PR brief) changes no lane's key.
GATE_CODE=(. ':(exclude)*.md' ':(exclude)docs/prs' ':(exclude).github')
# Scripts no lane runs (a person's tools: the area, the installers, the first-boot check, the PR summary): in no key,
# so a change to one reruns nothing (2026-11-27: an area.sh change reran every lane cold, g17 68 s at load 67-94).
GATE_TOOLS_ONLY=(scripts/area.sh scripts/bundle.sh scripts/deb-fast.sh scripts/first-boot-check.sh scripts/pr-status.py)
for _tool in "${GATE_TOOLS_ONLY[@]}"; do GATE_CODE+=(":(exclude)$_tool"); done
unset _tool
# The scripts a code lane (clippy, alone, tests, ui) runs; no build or test reads any other script (the crates name
# scripts only in comments). A code lane's key is the tree without the other scripts, so a change to a data lane's
# check (check-devices.sh, the kernel and u-boot checks) does not rerun the code lanes.
GATE_CODE_LANE_SCRIPTS=(scripts/gate.sh scripts/gate-lib.sh scripts/ui-gate.sh scripts/doc-crates.py scripts/cargo-seed.sh
  scripts/seed-strip-members.py)

gate_paths() {  # <code|data> <array name>: sets the array to that kind of lane's pathspecs
  local -n _out=$2
  local file keep
  if [ "$1" = data ]; then
    _out=(devices scripts ':(exclude)*.md')
    for file in "${GATE_TOOLS_ONLY[@]}"; do _out+=(":(exclude)$file"); done
    return
  fi
  _out=("${GATE_CODE[@]}")
  while IFS= read -r file; do
    for keep in "${GATE_CODE_LANE_SCRIPTS[@]}"; do [ "$file" = "$keep" ] && continue 2; done
    _out+=(":(exclude)$file")
  done < <(git ls-files -co --exclude-standard -- scripts)
}

# Each key's inputs (one line per tool and file, with its hash) are kept under "$(gate_pass_dir)/inputs/<key>", the
# newest 64, so a lane that ran when nothing seemed changed is explained by a diff, not a guess:
#   diff "$(gate_pass_dir)/inputs/<old key>" "$(gate_pass_dir)/inputs/<new key>"
gate_tree_key() {  # <label> <pathspec...>: prints the key of the files those paths select, with the label and tools
  local label=$1 dir tmp key
  shift
  dir="$(gate_pass_dir)/inputs"
  mkdir -p "$dir"
  tmp=$(mktemp "$dir/.key-XXXXXX")
  {
    printf 'lane %s\n' "$label"
    rustc --version 2>/dev/null || true
    cargo --version 2>/dev/null || true
    printf 'flags %s|%s|%s\n' "${RUSTFLAGS:-}" "${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS:-}" "${CARGO_INCREMENTAL:-}"
    git ls-files -co --exclude-standard -z -- "$@" | sort -z | xargs -0 -r sha256sum
  } >"$tmp"
  key=$(sha256sum <"$tmp" | cut -c1-32)
  mv -f "$tmp" "$dir/$key"
  find "$dir" -maxdepth 1 -type f -name '[0-9a-f]*' -printf '%T@ %p\n' | sort -rn | tail -n +65 | cut -d' ' -f2- \
    | while IFS= read -r old; do rm -f -- "$old"; done
  printf '%s\n' "$key"
}

# Kept in the worktree's own target (GATE_TARGET, set by the gate), whatever target a lane builds in.
gate_pass_dir() { printf '%s/.gate-pass' "${GATE_TARGET:-${CARGO_TARGET_DIR:-target}}"; }

gate_known() {  # <lane> <key>: 0 when this lane passed with this key (and ATLAS_GATE_FRESH is not set)
  [ "${ATLAS_GATE_FRESH:-0}" = 1 ] && return 1
  [ -f "$(gate_pass_dir)/$1-$2" ]
}

gate_mark() {  # <lane> <key>: records a pass
  mkdir -p "$(gate_pass_dir)"
  printf '%s %s\n' "$(git rev-parse HEAD 2>/dev/null || echo none)" "$(date -u +%FT%TZ)" > "$(gate_pass_dir)/$1-$2"
}

gate_known_since() {  # <lane> <key>: the commit and time the pass was recorded at
  cat "$(gate_pass_dir)/$1-$2" 2>/dev/null || true
}

# The build environment. sccache keeps every compiled dependency across worktrees and lanes, so a fresh worktree or
# a new lane target compiles only what changed (a cold workspace otherwise recompiles ~600 crates). mold links the
# ~90 test binaries in a fraction of ld's time. Both are used only where installed; CI sets its own.
gate_build_env() {
  if [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
  fi
  if [ -z "${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS:-}" ] && command -v mold >/dev/null 2>&1; then
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=mold"
  fi
  # The app's version, fixed for the gate: from a checkout it names the commit, so every commit
  # reran the app's build script and recompiled atlas-app in each lane and profile (the bridge
  # build alone 7-8 s on the ui lane's path, 2026-11-27). Nothing the gate runs reads the real
  # one; the simulated update feed is newer than any version. A release or a .deb sets its own.
  if [ -z "${ATLAS_VERSION:-}" ]; then
    local release
    release=$(sed -n '/^\[workspace.metadata.atlas\]/,/^\[/s/^release *= *"\(.*\)"/\1/p' Cargo.toml)
    export ATLAS_VERSION="${release:-0.0.0}-dev.0+gate"
  fi
}
