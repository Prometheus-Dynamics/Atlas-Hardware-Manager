#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-}"
if [[ -z "$TARGET" ]]; then
  echo "usage: $0 <target-triple>"
  exit 2
fi

tool_profiles_for_target() {
  case "$1" in
    x86_64-unknown-linux-gnu)
      echo "linux-x64 linux"
      ;;
    aarch64-unknown-linux-gnu)
      echo "linux-arm64 linux"
      ;;
    x86_64-pc-windows-gnu|x86_64-pc-windows-msvc)
      echo "windows-x64"
      ;;
    aarch64-pc-windows-gnullvm|aarch64-pc-windows-msvc)
      echo "windows-arm64"
      ;;
    x86_64-apple-darwin)
      echo "macos-x64"
      ;;
    aarch64-apple-darwin)
      echo "macos-arm64"
      ;;
    universal-apple-darwin)
      echo "macos-universal"
      ;;
    *)
      echo ""
      ;;
  esac
}

copy_bundled_tools() {
  local target="$1"
  local out_dir="$2"
  local tools_root="/workspace/tools"
  local profiles
  profiles="$(tool_profiles_for_target "$target")"
  if [[ -z "$profiles" || ! -d "$tools_root" ]]; then
    return
  fi
  mkdir -p "$out_dir/tools"
  for profile in $profiles; do
    local src="$tools_root/$profile"
    if [[ -d "$src" ]]; then
      rm -rf "$out_dir/tools/$profile"
      cp -a "$src" "$out_dir/tools/"
    fi
  done
}

cleanup_owner() {
  if [[ -n "${HOST_UID:-}" && -n "${HOST_GID:-}" ]]; then
    chown -R "${HOST_UID}:${HOST_GID}" /workspace/.svelte-kit /workspace/build /workspace/dist /workspace/outputs /workspace/src-tauri/target 2>/dev/null || true
  fi
}
trap cleanup_owner EXIT

OUTPUT_ROOT="/workspace/${ATLAS_OUTPUT_DIR:-outputs}"
TASK_LABEL="docker-cross-${TARGET}"
TASK_DIR="${OUTPUT_ROOT}/${TASK_LABEL}"
rm -rf "$TASK_DIR"
mkdir -p "$TASK_DIR"

case "$TARGET" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-pc-windows-gnu|aarch64-pc-windows-gnullvm)
    ;;
  x86_64-apple-darwin|aarch64-apple-darwin|universal-apple-darwin)
    echo "error: macOS targets are not supported by this Linux cross container. Build those on macOS runners."
    exit 2
    ;;
  *)
    echo "error: unsupported target triple: $TARGET"
    exit 2
    ;;
esac

echo "[cross] installing dependencies"
bun install --frozen-lockfile

case "$TARGET" in
  aarch64-unknown-linux-gnu)
    echo "[cross] skipping $TARGET in docker-cross: GTK/WebKit ARM64 sysroot is not available in this image."
    OUT_DIR="dist/cross/${TARGET}"
    mkdir -p "$OUT_DIR"
    echo "Skipped: requires a native ARM64 Linux builder with GTK/WebKit dev sysroot." > "$OUT_DIR/SKIPPED.txt"
    copy_bundled_tools "$TARGET" "$OUT_DIR"
    mkdir -p "${TASK_DIR}/cross"
    cp -a "$OUT_DIR" "${TASK_DIR}/cross/"
    printf "task=%s\ncapturedAt=%s\nrecords=1\ncross:dir:dist/cross/%s\n" "$TASK_LABEL" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TARGET" > "${TASK_DIR}/SUMMARY.txt"
    exit 0
    ;;
  aarch64-pc-windows-gnullvm)
    echo "[cross] skipping $TARGET in docker-cross: toolchain lacks aarch64-w64-mingw32-clang for ring/reqwest dependencies."
    OUT_DIR="dist/cross/${TARGET}"
    mkdir -p "$OUT_DIR"
    echo "Skipped: requires dedicated Windows ARM64 cross toolchain image." > "$OUT_DIR/SKIPPED.txt"
    copy_bundled_tools "$TARGET" "$OUT_DIR"
    mkdir -p "${TASK_DIR}/cross"
    cp -a "$OUT_DIR" "${TASK_DIR}/cross/"
    printf "task=%s\ncapturedAt=%s\nrecords=1\ncross:dir:dist/cross/%s\n" "$TASK_LABEL" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TARGET" > "${TASK_DIR}/SUMMARY.txt"
    exit 0
    ;;
esac

if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "[cross] installing missing rust target: $TARGET"
  rustup target add "$TARGET"
fi

echo "[cross] building frontend and rust target: $TARGET"
bun run build

if [[ "$TARGET" == "x86_64-pc-windows-gnu" ]]; then
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
fi

cargo build \
  --manifest-path src-tauri/Cargo.toml \
  --release \
  --target "$TARGET" \
  --features tauri/custom-protocol

if [[ "$TARGET" == *"windows"* ]]; then
  BIN_PATH="src-tauri/target/${TARGET}/release/atlas-hardware-manager.exe"
  BIN_NAME="atlas-hardware-manager.exe"
else
  BIN_PATH="src-tauri/target/${TARGET}/release/atlas-hardware-manager"
  BIN_NAME="atlas-hardware-manager"
fi

if [[ ! -f "$BIN_PATH" ]]; then
  echo "error: expected output binary not found at $BIN_PATH"
  exit 1
fi

OUT_DIR="dist/cross/${TARGET}"
mkdir -p "$OUT_DIR"
cp "$BIN_PATH" "$OUT_DIR/$BIN_NAME"
copy_bundled_tools "$TARGET" "$OUT_DIR"

mkdir -p "${TASK_DIR}/cross"
cp -a "$OUT_DIR" "${TASK_DIR}/cross/"
printf "task=%s\ncapturedAt=%s\nrecords=1\ncross:dir:dist/cross/%s\n" "$TASK_LABEL" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TARGET" > "${TASK_DIR}/SUMMARY.txt"

echo "[cross] output copied to $OUT_DIR/$BIN_NAME"
echo "[cross] output captured in ${TASK_DIR}/cross/${TARGET}"
