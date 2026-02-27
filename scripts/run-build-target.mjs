#!/usr/bin/env node
/* global process, console */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const task = process.argv[2] ?? "";
if (!task) {
  console.error("usage: bun run scripts/run-build-target.mjs <target-task>");
  process.exit(2);
}

const hostPlatform = process.platform;
const hostArch = process.arch;
const strictHost = process.env.ATLAS_STRICT_HOST_BUILDS === "1";
const outputRoot = path.resolve(process.cwd(), process.env.ATLAS_OUTPUT_DIR ?? "outputs");

function run(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, {
    stdio: "inherit",
    shell: false,
    ...options
  });

  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} failed with code ${result.status ?? "unknown"}`);
  }
}

function note(message) {
  console.log(`[build] ${message}`);
}

function softSkip(message) {
  if (strictHost) {
    throw new Error(`${message} Set ATLAS_STRICT_HOST_BUILDS=0 to allow skip.`);
  }
  note(`${message} Skipping by default on this host.`);
}

function bunRun(scriptName) {
  run("bun", ["run", scriptName]);
}

function bunTauriBuild(args) {
  run("bun", ["x", "tauri", "build", ...args]);
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function sanitizeTask(taskName) {
  return taskName.replace(/[^a-zA-Z0-9_.-]+/g, "-");
}

function expectedBundlePrefixes(taskName) {
  switch (taskName) {
    case "linux":
      return ["release/bundle/"];
    case "linux:x64":
      return ["x86_64-unknown-linux-gnu/release/bundle/"];
    case "linux:arm64":
      return ["aarch64-unknown-linux-gnu/release/bundle/"];
    case "linux:appimage:x64":
      return ["x86_64-unknown-linux-gnu/release/bundle/appimage/"];
    case "linux:appimage:arm64":
      return ["aarch64-unknown-linux-gnu/release/bundle/appimage/"];
    case "windows:x64":
      return ["x86_64-pc-windows-msvc/release/bundle/"];
    case "windows:arm64":
      return ["aarch64-pc-windows-msvc/release/bundle/"];
    case "macos:x64":
      return ["x86_64-apple-darwin/release/bundle/"];
    case "macos:arm64":
      return ["aarch64-apple-darwin/release/bundle/"];
    case "macos:universal":
      return ["universal-apple-darwin/release/bundle/"];
    default:
      return [];
  }
}

function listBundleArtifacts(taskName) {
  const targetRoot = path.join(process.cwd(), "src-tauri", "target");
  if (!fs.existsSync(targetRoot)) {
    return [];
  }

  const artifacts = [];
  const bundlePrefixes = expectedBundlePrefixes(taskName);
  if (bundlePrefixes.length === 0) {
    return artifacts;
  }
  const allowedFileExtensions = new Set([
    ".deb",
    ".rpm",
    ".msi",
    ".exe",
    ".dmg",
    ".appimage",
    ".app",
  ]);
  const allowedFileNames = new Set(["AppImage"]);

  function walkBundleRoot(currentPath) {
    const entries = fs.readdirSync(currentPath, { withFileTypes: true });
    for (const entry of entries) {
      const entryPath = path.join(currentPath, entry.name);
      if (entry.isDirectory()) {
        if (entry.name.endsWith(".app")) {
          artifacts.push({ type: "dir", absolutePath: entryPath });
          continue;
        }
        walkBundleRoot(entryPath);
        continue;
      }

      if (!entry.isFile()) {
        continue;
      }

      const lowerName = entry.name.toLowerCase();
      const ext = path.extname(lowerName);
      if (allowedFileExtensions.has(ext) || allowedFileNames.has(entry.name)) {
        artifacts.push({ type: "file", absolutePath: entryPath });
      }
    }
  }

  for (const prefix of bundlePrefixes) {
    const trimmedPrefix = prefix.replace(/\/+$/, "");
    if (!trimmedPrefix) continue;
    const bundleRoot = path.join(targetRoot, ...trimmedPrefix.split("/"));
    if (!fs.existsSync(bundleRoot)) continue;
    walkBundleRoot(bundleRoot);
  }
  return artifacts;
}

function expectedCrossTriples(taskName) {
  switch (taskName) {
    case "linux":
    case "linux:x64":
    case "linux:appimage:x64":
      return ["x86_64-unknown-linux-gnu"];
    case "linux:arm64":
    case "linux:appimage:arm64":
      return ["aarch64-unknown-linux-gnu"];
    case "windows:x64":
      return ["x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"];
    case "windows:arm64":
      return ["aarch64-pc-windows-gnullvm", "aarch64-pc-windows-msvc"];
    default:
      return [];
  }
}

function toolProfilesForTargetTriple(targetTriple) {
  switch (targetTriple) {
    case "x86_64-unknown-linux-gnu":
      return ["linux-x64", "linux"];
    case "aarch64-unknown-linux-gnu":
      return ["linux-arm64", "linux"];
    case "x86_64-pc-windows-gnu":
    case "x86_64-pc-windows-msvc":
      return ["windows-x64"];
    case "aarch64-pc-windows-gnullvm":
    case "aarch64-pc-windows-msvc":
      return ["windows-arm64"];
    case "x86_64-apple-darwin":
      return ["macos-x64"];
    case "aarch64-apple-darwin":
      return ["macos-arm64"];
    case "universal-apple-darwin":
      return ["macos-universal"];
    default:
      return [];
  }
}

function copyBundledToolsForTriple(targetTriple, tripleOutputDir, records) {
  const toolsRoot = path.join(process.cwd(), "tools");
  if (!fs.existsSync(toolsRoot)) {
    return;
  }

  const seen = new Set();
  for (const profile of toolProfilesForTargetTriple(targetTriple)) {
    if (seen.has(profile)) continue;
    seen.add(profile);

    const src = path.join(toolsRoot, profile);
    if (!fs.existsSync(src)) {
      continue;
    }
    const dest = path.join(tripleOutputDir, "tools", profile);
    copyPath(src, dest, "dir");
    records.push(`tool:dir:${path.relative(process.cwd(), src)}=>${path.relative(process.cwd(), dest)}`);
  }
}

function copyPath(srcPath, destPath, type) {
  ensureDir(path.dirname(destPath));
  if (type === "dir") {
    fs.rmSync(destPath, { recursive: true, force: true });
    fs.cpSync(srcPath, destPath, { recursive: true });
  } else {
    fs.copyFileSync(srcPath, destPath);
  }
}

function captureBuildOutputs(taskName) {
  const targetDir = path.join(outputRoot, sanitizeTask(taskName));
  fs.rmSync(targetDir, { recursive: true, force: true });
  ensureDir(targetDir);

  const records = [];
  const bundleArtifacts = listBundleArtifacts(taskName);
  const targetRoot = path.join(process.cwd(), "src-tauri", "target");
  for (const artifact of bundleArtifacts) {
    const rel = path.relative(targetRoot, artifact.absolutePath);
    const dest = path.join(targetDir, "bundles", rel);
    copyPath(artifact.absolutePath, dest, artifact.type);
    records.push(`bundle:${artifact.type}:${rel}`);
  }

  const crossRoot = path.join(process.cwd(), "dist", "cross");
  for (const triple of expectedCrossTriples(taskName)) {
    const sourceDir = path.join(crossRoot, triple);
    if (!fs.existsSync(sourceDir)) {
      continue;
    }
    const destDir = path.join(targetDir, "cross", triple);
    copyPath(sourceDir, destDir, "dir");
    records.push(`cross:dir:${path.relative(process.cwd(), sourceDir)}`);
    copyBundledToolsForTriple(triple, destDir, records);
  }

  const summaryLines = [
    `task=${taskName}`,
    `capturedAt=${new Date().toISOString()}`,
    `records=${records.length}`,
    ...records,
    "",
  ];
  fs.writeFileSync(path.join(targetDir, "SUMMARY.txt"), summaryLines.join("\n"), "utf8");
  note(`Captured ${records.length} artifact entr${records.length === 1 ? "y" : "ies"} to ${path.relative(process.cwd(), targetDir)}`);
}

function buildSigningConfigOverlay(platform, baseConfigPath) {
  const source = JSON.parse(fs.readFileSync(baseConfigPath, "utf8"));
  const overlay = JSON.parse(JSON.stringify(source));
  let changed = false;

  if (platform === "windows") {
    const thumbprint = process.env.ATLAS_WINDOWS_CERT_THUMBPRINT?.trim();
    const timestampUrl = process.env.ATLAS_WINDOWS_TIMESTAMP_URL?.trim();
    if (thumbprint) {
      overlay.bundle ??= {};
      overlay.bundle.windows ??= {};
      overlay.bundle.windows.certificateThumbprint = thumbprint;
      changed = true;
    }
    if (timestampUrl) {
      overlay.bundle ??= {};
      overlay.bundle.windows ??= {};
      overlay.bundle.windows.timestampUrl = timestampUrl;
      changed = true;
    }
  }

  if (platform === "macos") {
    const signingIdentity = process.env.ATLAS_APPLE_SIGNING_IDENTITY?.trim();
    const providerShortName = process.env.ATLAS_APPLE_PROVIDER_SHORT_NAME?.trim();
    if (signingIdentity) {
      overlay.bundle ??= {};
      overlay.bundle.macOS ??= {};
      overlay.bundle.macOS.signingIdentity = signingIdentity;
      changed = true;
    }
    if (providerShortName) {
      const looksLikeRunnerImageConfig = /^macos-\d+(-[a-z0-9]+)*$/i.test(providerShortName);
      if (looksLikeRunnerImageConfig) {
        note(
          `Ignoring ATLAS_APPLE_PROVIDER_SHORT_NAME='${providerShortName}' because it looks like a runner image label, not an Apple provider short name.`
        );
      } else {
        overlay.bundle ??= {};
        overlay.bundle.macOS ??= {};
        overlay.bundle.macOS.providerShortName = providerShortName;
        changed = true;
      }
    }
  }

  if (!changed) {
    return baseConfigPath;
  }

  const outputDir = path.join(process.cwd(), ".cache", "build-config");
  fs.mkdirSync(outputDir, { recursive: true });
  const outputPath = path.join(outputDir, `tauri.${platform}.signed.conf.json`);
  fs.writeFileSync(outputPath, `${JSON.stringify(overlay, null, 2)}\n`, "utf8");
  note(`Applied signing overlay config: ${path.relative(process.cwd(), outputPath)}`);
  return outputPath;
}

function nativeLinuxDebRpm(target) {
  bunRun("rpiboot:build");
  bunRun(target === "x86_64-unknown-linux-gnu" ? "tools:verify:linux:x64" : "tools:verify:linux:arm64");
  bunTauriBuild([
    "--target",
    target,
    "--bundles",
    "deb,rpm",
    "--config",
    "src-tauri/tauri.linux.conf.json"
  ]);
}

function nativeLinuxAppImage(target) {
  bunRun("rpiboot:build");
  bunRun(target === "x86_64-unknown-linux-gnu" ? "tools:verify:linux:x64" : "tools:verify:linux:arm64");
  bunTauriBuild([
    "--target",
    target,
    "--bundles",
    "appimage",
    "--config",
    "src-tauri/tauri.linux.conf.json"
  ]);
}

function nativeWindowsInstallers(target) {
  bunRun("rpiboot:build");
  bunRun(target === "x86_64-pc-windows-msvc" ? "tools:verify:windows:x64" : "tools:verify:windows:arm64");
  const windowsConfig = buildSigningConfigOverlay("windows", "src-tauri/tauri.windows.conf.json");
  bunTauriBuild([
    "--target",
    target,
    "--bundles",
    "nsis,msi",
    "--config",
    windowsConfig
  ]);
}

function nativeMacInstallers(target) {
  bunRun("rpiboot:build");
  if (target === "x86_64-apple-darwin") bunRun("tools:verify:macos:x64");
  else if (target === "aarch64-apple-darwin") bunRun("tools:verify:macos:arm64");
  else bunRun("tools:verify:macos:universal");
  const macosConfig = buildSigningConfigOverlay("macos", "src-tauri/tauri.macos.conf.json");

  bunTauriBuild([
    "--target",
    target,
    "--bundles",
    "app,dmg",
    "--config",
    macosConfig
  ]);
}

function isDebianLikeLinux() {
  if (hostPlatform !== "linux") return false;
  try {
    const raw = fs.readFileSync("/etc/os-release", "utf8");
    const lines = Object.fromEntries(
      raw
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line && !line.startsWith("#") && line.includes("="))
        .map((line) => {
          const [key, ...rest] = line.split("=");
          const value = rest.join("=").replace(/^"/, "").replace(/"$/, "");
          return [key, value.toLowerCase()];
        })
    );
    const id = lines.ID ?? "";
    const idLike = lines.ID_LIKE ?? "";
    return id === "ubuntu" || id === "debian" || idLike.includes("debian") || idLike.includes("ubuntu");
  } catch {
    return false;
  }
}

function fallbackCross(scriptName, reason) {
  note(`${reason} Falling back to ${scriptName} for raw binary output.`);
  bunRun(scriptName);
}

try {
  switch (task) {
    case "linux": {
      if (hostPlatform !== "linux") {
        fallbackCross("docker:cross:linux:x64", "Native linux deb/rpm build requires a Linux host.");
        break;
      }
      bunRun("rpiboot:build");
      bunRun("tools:verify:host");
      bunTauriBuild(["--bundles", "deb,rpm", "--config", "src-tauri/tauri.linux.conf.json"]);
      break;
    }

    case "linux:x64": {
      if (hostPlatform !== "linux" || hostArch !== "x64") {
        fallbackCross("docker:cross:linux:x64", "Linux x64 deb/rpm installer build is host-specific.");
        break;
      }
      nativeLinuxDebRpm("x86_64-unknown-linux-gnu");
      break;
    }

    case "linux:arm64": {
      if (hostPlatform === "linux" && hostArch === "arm64") {
        nativeLinuxDebRpm("aarch64-unknown-linux-gnu");
        break;
      }
      fallbackCross("docker:cross:linux:arm64", "Linux ARM64 installer build requires an ARM64 Linux host.");
      break;
    }

    case "linux:appimage:x64": {
      if (hostPlatform === "linux" && hostArch === "x64" && isDebianLikeLinux()) {
        nativeLinuxAppImage("x86_64-unknown-linux-gnu");
        break;
      }
      fallbackCross(
        "docker:cross:linux:x64",
        "AppImage packaging is unreliable on this host distro (linuxdeploy strip/RELR mismatch)."
      );
      break;
    }

    case "linux:appimage:arm64": {
      if (hostPlatform === "linux" && hostArch === "arm64" && isDebianLikeLinux()) {
        nativeLinuxAppImage("aarch64-unknown-linux-gnu");
        break;
      }
      fallbackCross(
        "docker:cross:linux:arm64",
        "ARM64 AppImage packaging requires an ARM64 Debian/Ubuntu builder."
      );
      break;
    }

    case "windows:x64": {
      if (hostPlatform === "win32") {
        nativeWindowsInstallers("x86_64-pc-windows-msvc");
      } else {
        fallbackCross("docker:cross:windows:x64", "Windows MSI/NSIS installers require a Windows host.");
      }
      break;
    }

    case "windows:arm64": {
      if (hostPlatform === "win32") {
        nativeWindowsInstallers("aarch64-pc-windows-msvc");
      } else {
        fallbackCross("docker:cross:windows:arm64", "Windows ARM64 MSI/NSIS installers require a Windows host.");
      }
      break;
    }

    case "macos:x64": {
      if (hostPlatform !== "darwin") {
        softSkip("macOS Intel DMG build requires a macOS host.");
        break;
      }
      nativeMacInstallers("x86_64-apple-darwin");
      break;
    }

    case "macos:arm64": {
      if (hostPlatform !== "darwin") {
        softSkip("macOS Apple Silicon DMG build requires a macOS host.");
        break;
      }
      nativeMacInstallers("aarch64-apple-darwin");
      break;
    }

    case "macos:universal": {
      if (hostPlatform !== "darwin") {
        softSkip("macOS universal DMG build requires a macOS host.");
        break;
      }
      nativeMacInstallers("universal-apple-darwin");
      break;
    }

    default:
      throw new Error(`unsupported build target: ${task}`);
  }
  captureBuildOutputs(task);
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`[build] ${message}`);
  process.exit(1);
}
