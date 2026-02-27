#!/usr/bin/env node
/* global process, console */

import fs from "node:fs";
import path from "node:path";

const rawInput = (process.argv[2] ?? "").trim();
if (!rawInput) {
  console.error("usage: node scripts/set-project-version.mjs <version-or-tag>");
  process.exit(2);
}

const normalized = rawInput.startsWith("v") ? rawInput.slice(1) : rawInput;
const semverLike = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;
if (!semverLike.test(normalized)) {
  console.error(`invalid version '${rawInput}' (expected semver-like value such as 2026.0.0 or v2026.0.0)`);
  process.exit(2);
}

const repoRoot = process.cwd();
const packageJsonPath = path.join(repoRoot, "package.json");
const tauriConfPath = path.join(repoRoot, "src-tauri", "tauri.conf.json");
const tauriWindowsConfPath = path.join(repoRoot, "src-tauri", "tauri.windows.conf.json");
const cargoTomlPath = path.join(repoRoot, "src-tauri", "Cargo.toml");
const cargoLockPath = path.join(repoRoot, "src-tauri", "Cargo.lock");

function writeText(filePath, contents) {
  const normalized = contents.replace(/\r\n/g, "\n").replace(/\n+$/, "");
  fs.writeFileSync(filePath, `${normalized}\n`, "utf8");
}

function updateJsonVersion(filePath, version) {
  const parsed = JSON.parse(fs.readFileSync(filePath, "utf8"));
  parsed.version = version;
  writeText(filePath, JSON.stringify(parsed, null, 2));
}

function parseCoreVersion(version) {
  const core = version.split(/[+-]/, 1)[0];
  const parts = core.split(".");
  if (parts.length < 3) {
    throw new Error(`version '${version}' must have at least major.minor.patch`);
  }
  const [majorRaw, minorRaw, patchRaw] = parts;
  const major = Number.parseInt(majorRaw, 10);
  const minor = Number.parseInt(minorRaw, 10);
  const patch = Number.parseInt(patchRaw, 10);
  if ([major, minor, patch].some((value) => Number.isNaN(value) || value < 0)) {
    throw new Error(`version '${version}' contains invalid numeric core fields`);
  }
  return { major, minor, patch };
}

function clamp(value, max) {
  return Math.min(Math.max(value, 0), max);
}

function deriveWixVersion(version) {
  const { major, minor, patch } = parseCoreVersion(version);

  if (major <= 255 && minor <= 255 && patch <= 65535) {
    return `${major}.${minor}.${patch}`;
  }

  // MSI/WiX ProductVersion constraints:
  // major/minor <= 255, patch/build <= 65535.
  // For high major versions (e.g. 2026.0.0), encode the original major
  // in the patch field and keep patch in build.
  return `255.${clamp(minor, 255)}.${clamp(major, 65535)}.${clamp(patch, 65535)}`;
}

function updateWindowsWixVersion(filePath, appVersion) {
  const parsed = JSON.parse(fs.readFileSync(filePath, "utf8"));
  parsed.bundle ??= {};
  parsed.bundle.windows ??= {};
  parsed.bundle.windows.wix ??= {};
  parsed.bundle.windows.wix.version = deriveWixVersion(appVersion);
  writeText(filePath, JSON.stringify(parsed, null, 2));
}

function updateCargoTomlVersion(filePath, version) {
  const raw = fs.readFileSync(filePath, "utf8");
  const lines = raw.split(/\r?\n/);
  let inPackageSection = false;
  let replaced = false;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
      inPackageSection = trimmed === "[package]";
      continue;
    }

    if (!inPackageSection) {
      continue;
    }

    if (/^version\s*=/.test(trimmed)) {
      const indentMatch = line.match(/^(\s*)/);
      const indent = indentMatch ? indentMatch[1] : "";
      lines[i] = `${indent}version = "${version}"`;
      replaced = true;
      break;
    }
  }

  if (!replaced) {
    throw new Error("failed to find [package] version in src-tauri/Cargo.toml");
  }

  writeText(filePath, lines.join("\n"));
}

function updateCargoLockAppVersion(filePath, version) {
  const raw = fs.readFileSync(filePath, "utf8");
  const pattern = /(\[\[package\]\]\r?\nname = "atlas-hardware-manager"\r?\nversion = ")([^"]+)(")/;
  if (!pattern.test(raw)) {
    throw new Error('failed to find atlas-hardware-manager package entry in src-tauri/Cargo.lock');
  }
  const updated = raw.replace(pattern, `$1${version}$3`);
  writeText(filePath, updated);
}

updateJsonVersion(packageJsonPath, normalized);
updateJsonVersion(tauriConfPath, normalized);
updateWindowsWixVersion(tauriWindowsConfPath, normalized);
updateCargoTomlVersion(cargoTomlPath, normalized);
updateCargoLockAppVersion(cargoLockPath, normalized);

const wixVersion = deriveWixVersion(normalized);
console.log(`[version] set project version to ${normalized} (windows msi/wix ${wixVersion})`);
