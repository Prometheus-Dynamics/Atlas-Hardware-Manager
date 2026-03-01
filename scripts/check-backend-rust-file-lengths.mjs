#!/usr/bin/env node
/* global process, console */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const BACKEND_PREFIX = "src-tauri/src/";
const SOFT_TARGET_LINES = 500;
const HARD_MAX_LINES = 600;

function listTrackedFiles() {
  const gitResult = spawnSync("git", ["ls-files", "-z"], {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (gitResult.status === 0 && gitResult.stdout) {
    return gitResult.stdout.split("\0").filter(Boolean);
  }

  const fallbackResult = spawnSync("rg", ["--files"], {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (fallbackResult.status !== 0) {
    throw new Error("Unable to list files via git ls-files or rg --files.");
  }

  return fallbackResult.stdout
    .split("\n")
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function backendRustFiles() {
  return listTrackedFiles().filter((filePath) => {
    const normalized = filePath.replaceAll("\\", "/");
    return normalized.startsWith(BACKEND_PREFIX) && normalized.endsWith(".rs");
  });
}

function countLines(filePath) {
  const raw = fs.readFileSync(filePath, "utf8");
  if (raw.length === 0) {
    return 0;
  }
  return raw.split(/\r?\n/).length;
}

function main() {
  const files = backendRustFiles();
  const overSoftTarget = [];
  const overHardMax = [];

  for (const relativePath of files) {
    const absolutePath = path.resolve(process.cwd(), relativePath);
    if (!fs.existsSync(absolutePath)) {
      continue;
    }

    let lines;
    try {
      lines = countLines(absolutePath);
    } catch {
      continue;
    }

    if (lines > HARD_MAX_LINES) {
      overHardMax.push({ path: relativePath, lines });
      continue;
    }

    if (lines > SOFT_TARGET_LINES) {
      overSoftTarget.push({ path: relativePath, lines });
    }
  }

  const byLineCountDesc = (left, right) =>
    right.lines - left.lines || left.path.localeCompare(right.path);
  overHardMax.sort(byLineCountDesc);
  overSoftTarget.sort(byLineCountDesc);

  if (overHardMax.length > 0) {
    console.error(
      `[backend-size] hard limit ${HARD_MAX_LINES} lines exceeded in ${overHardMax.length} backend Rust file(s):`,
    );
    for (const entry of overHardMax) {
      console.error(`[backend-size] error ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (overSoftTarget.length > 0) {
    console.warn(
      `[backend-size] files above soft target ${SOFT_TARGET_LINES} lines (${overSoftTarget.length}):`,
    );
    for (const entry of overSoftTarget) {
      console.warn(`[backend-size] warn ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (overHardMax.length === 0) {
    console.log(
      `[backend-size] pass. target=${SOFT_TARGET_LINES}, max=${HARD_MAX_LINES}, checked=${files.length}`,
    );
    process.exit(0);
  }

  process.exit(1);
}

main();
