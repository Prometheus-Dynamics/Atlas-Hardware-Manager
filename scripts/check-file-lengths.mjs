#!/usr/bin/env node
/* global process, console */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const TARGET_LINES = 600;
const MAX_LINES = 800;

const INCLUDE_EXTENSIONS = new Set([
  ".rs",
  ".svelte",
  ".ts",
  ".js",
  ".mjs",
  ".cjs",
  ".json",
  ".toml",
  ".md",
  ".css",
  ".html",
  ".yml",
  ".yaml",
]);

const IGNORE_PREFIXES = [
  ".git/",
  ".svelte-kit/",
  "build/",
  "dist/",
  "node_modules/",
  "outputs/",
  "src-tauri/target/",
  "tools/",
  "static/fonts/",
];

const ALLOW_OVER_MAX = new Set([
  "src/routes/network/+page.svelte",
]);

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

function shouldCheck(filePath) {
  if (filePath.includes("\0")) {
    return false;
  }

  const normalized = filePath.replaceAll("\\", "/");
  if (IGNORE_PREFIXES.some((prefix) => normalized.startsWith(prefix))) {
    return false;
  }

  const extension = path.extname(normalized).toLowerCase();
  return INCLUDE_EXTENSIONS.has(extension);
}

function countLines(filePath) {
  const raw = fs.readFileSync(filePath, "utf8");
  if (raw.length === 0) {
    return 0;
  }
  return raw.split(/\r?\n/).length;
}

function main() {
  const files = listTrackedFiles().filter(shouldCheck);
  const overTarget = [];
  const allowedOverMax = [];
  const blockedOverMax = [];
  const baselineCleanupCandidates = [];

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

    if (lines > MAX_LINES) {
      if (ALLOW_OVER_MAX.has(relativePath)) {
        allowedOverMax.push({ path: relativePath, lines });
      } else {
        blockedOverMax.push({ path: relativePath, lines });
      }
      continue;
    }

    if (ALLOW_OVER_MAX.has(relativePath)) {
      baselineCleanupCandidates.push({ path: relativePath, lines });
    }

    if (lines > TARGET_LINES) {
      overTarget.push({ path: relativePath, lines });
    }
  }

  const byLineCountDesc = (left, right) => right.lines - left.lines || left.path.localeCompare(right.path);
  blockedOverMax.sort(byLineCountDesc);
  allowedOverMax.sort(byLineCountDesc);
  overTarget.sort(byLineCountDesc);
  baselineCleanupCandidates.sort(byLineCountDesc);

  if (blockedOverMax.length > 0) {
    console.error(`[file-size] hard limit ${MAX_LINES} lines exceeded in ${blockedOverMax.length} file(s):`);
    for (const entry of blockedOverMax) {
      console.error(`[file-size] error ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (allowedOverMax.length > 0) {
    console.warn(
      `[file-size] grandfathered files above ${MAX_LINES} lines (${allowedOverMax.length}):`,
    );
    for (const entry of allowedOverMax) {
      console.warn(`[file-size] debt ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (overTarget.length > 0) {
    console.warn(
      `[file-size] files above target ${TARGET_LINES} lines (${overTarget.length}) but within hard cap ${MAX_LINES}:`,
    );
    for (const entry of overTarget) {
      console.warn(`[file-size] warn ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (baselineCleanupCandidates.length > 0) {
    console.warn(`[file-size] remove from allow list (now <= ${MAX_LINES}):`);
    for (const entry of baselineCleanupCandidates) {
      console.warn(`[file-size] cleanup ${entry.path}: ${entry.lines} lines`);
    }
  }

  if (blockedOverMax.length === 0) {
    console.log(
      `[file-size] pass. target=${TARGET_LINES}, max=${MAX_LINES}, checked=${files.length}, debt=${allowedOverMax.length}`,
    );
    process.exit(0);
  }

  process.exit(1);
}

main();
