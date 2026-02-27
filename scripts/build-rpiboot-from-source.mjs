/* global process, console */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

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

function capture(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, {
    stdio: ["ignore", "pipe", "pipe"],
    shell: false,
    encoding: "utf8",
    ...options
  });

  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} failed with code ${result.status ?? "unknown"}`);
  }

  return (result.stdout ?? "").toString().trim();
}

function commandExists(command) {
  const checker = process.platform === "win32" ? "where" : "which";
  const result = spawnSync(checker, [command], { stdio: "ignore" });
  return result.status === 0;
}

function makeEnv() {
  const env = { ...process.env };
  if (process.platform !== "win32") {
    return env;
  }

  // Keep pkg-config paths in MSYS form; drive-letter conversion breaks colon-delimited parsing.
  const mingwPrefix = (env.MINGW_PREFIX?.trim() || env.MSYSTEM_PREFIX?.trim() || "/mingw64").replace(/\\/g, "/");
  const pkgConfigDirs = [`${mingwPrefix}/lib/pkgconfig`, `${mingwPrefix}/share/pkgconfig`];
  env.PKG_CONFIG_PATH = pkgConfigDirs.join(";");
  env.PKG_CONFIG_LIBDIR = [...pkgConfigDirs, "/usr/lib/pkgconfig", "/usr/share/pkgconfig"].join(";");
  env.PKG_CONFIG = `${mingwPrefix}/bin/pkg-config`;

  const excludedEnv = new Set(
    (env.MSYS2_ENV_CONV_EXCL ?? "")
      .split(";")
      .map((value) => value.trim())
      .filter(Boolean)
  );
  excludedEnv.add("PKG_CONFIG_PATH");
  excludedEnv.add("PKG_CONFIG_LIBDIR");
  excludedEnv.add("PKG_CONFIG");
  env.MSYS2_ENV_CONV_EXCL = Array.from(excludedEnv).join(";");

  return env;
}

function windowsMakeArgs(env) {
  if (process.platform !== "win32") {
    return [];
  }

  const mingwPrefix = (env.MINGW_PREFIX?.trim() || env.MSYSTEM_PREFIX?.trim() || "/mingw64").replace(/\\/g, "/");
  const compiler = env.ATLAS_RPIBOOT_CC?.trim() || `${mingwPrefix}/bin/gcc`;
  const cppflags = env.ATLAS_RPIBOOT_CPPFLAGS?.trim() || "-D_POSIX_VERSION=200809L";
  const cflags = env.ATLAS_RPIBOOT_CFLAGS?.trim() || `-I${mingwPrefix}/include/libusb-1.0`;
  const ldflags = env.ATLAS_RPIBOOT_LDFLAGS?.trim() || `-L${mingwPrefix}/lib -lusb-1.0`;

  return [`CC=${compiler}`, `CPPFLAGS=${cppflags}`, `CFLAGS=${cflags}`, `LDFLAGS=${ldflags}`];
}

function injectWindowsFmemopenFallback(repoDir) {
  if (process.platform !== "win32") {
    return;
  }

  const mainPath = path.join(repoDir, "main.c");
  const source = fs.readFileSync(mainPath, "utf8");
  if (source.includes("#define fmemopen atlas_fmemopen")) {
    return;
  }

  const eol = source.includes("\r\n") ? "\r\n" : "\n";
  const fallbackBlock = [
    "#ifdef _WIN32",
    "/* MinGW builds may not expose fmemopen; use a tmpfile-backed fallback. */",
    "static FILE *atlas_fmemopen(void *buf, size_t size, const char *mode) {",
    "\tFILE *fp = tmpfile();",
    "\tif (!fp)",
    "\t\treturn NULL;",
    "",
    "\tif (strchr(mode, 'r') != NULL && size > 0) {",
    "\t\tif (fwrite(buf, 1, size, fp) != size) {",
    "\t\t\tfclose(fp);",
    "\t\t\treturn NULL;",
    "\t\t}",
    "\t\trewind(fp);",
    "\t}",
    "",
    "\treturn fp;",
    "}",
    "#define fmemopen atlas_fmemopen",
    "#endif",
  ].join(eol);

  const includePattern = /#include\s*<unistd\.h>\r?\n/;
  if (!includePattern.test(source)) {
    throw new Error("Unable to inject Windows fmemopen fallback: '#include <unistd.h>' not found in usbboot main.c.");
  }

  const patched = source.replace(includePattern, (match) => `${match}${fallbackBlock}${eol}`);
  fs.writeFileSync(mainPath, patched, "utf8");
}

function platformTag() {
  if (process.platform === "linux") return "linux";
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  throw new Error(`Unsupported platform: ${process.platform}`);
}

function archTag() {
  if (process.arch === "x64") return "x64";
  if (process.arch === "arm64") return "arm64";
  if (process.arch === "ia32") return "x86";
  return "unknown";
}

function ensureExecutable(filePath) {
  if (process.platform !== "win32") {
    fs.chmodSync(filePath, 0o755);
  }
}

function copyDir(sourceDir, targetDir) {
  fs.rmSync(targetDir, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(targetDir), { recursive: true });
  fs.cpSync(sourceDir, targetDir, { recursive: true, dereference: true });
}

function syncToToolDirs(binaryPath, bootDirPath, binaryName) {
  const rootDir = process.cwd();
  const platform = platformTag();
  const arch = archTag();

  const platformDir = path.join(rootDir, "tools", platform);
  const profileDir = path.join(rootDir, "tools", `${platform}-${arch}`);

  for (const dir of [platformDir, profileDir]) {
    fs.mkdirSync(dir, { recursive: true });
    const targetBinary = path.join(dir, binaryName);
    fs.copyFileSync(binaryPath, targetBinary);
    ensureExecutable(targetBinary);
    copyDir(bootDirPath, path.join(dir, "mass-storage-gadget64"));
  }

  console.log(`Bundled rpiboot into ${path.relative(rootDir, platformDir)} and ${path.relative(rootDir, profileDir)}`);
}

function main() {
  if (!commandExists("git")) {
    throw new Error("git is required to pull usbboot source.");
  }
  if (!commandExists("make")) {
    throw new Error("make is required to build rpiboot from source.");
  }

  const rootDir = process.cwd();
  const cacheRoot = path.join(rootDir, ".cache", "rpiboot-source");
  const repoDir = path.join(cacheRoot, "usbboot");
  const repoUrl = "https://github.com/raspberrypi/usbboot.git";

  fs.mkdirSync(cacheRoot, { recursive: true });

  if (!fs.existsSync(path.join(repoDir, ".git"))) {
    run("git", ["clone", "--depth", "1", repoUrl, repoDir]);
  } else {
    run("git", ["-C", repoDir, "fetch", "--depth", "1", "origin"]);
    const defaultHead = capture("git", ["-C", repoDir, "symbolic-ref", "refs/remotes/origin/HEAD"]);
    run("git", ["-C", repoDir, "reset", "--hard", defaultHead]);
    run("git", ["-C", repoDir, "clean", "-fdx"]);
  }

  injectWindowsFmemopenFallback(repoDir);

  const env = makeEnv();
  run("make", windowsMakeArgs(env), { cwd: repoDir, env });

  const candidateBinaries = process.platform === "win32"
    ? [path.join(repoDir, "rpiboot.exe"), path.join(repoDir, "rpiboot")]
    : [path.join(repoDir, "rpiboot")];

  const binaryPath = candidateBinaries.find((candidate) => fs.existsSync(candidate) && fs.statSync(candidate).isFile());
  if (!binaryPath) {
    throw new Error("Built rpiboot binary was not found after source build.");
  }

  const bootDirPath = path.join(repoDir, "mass-storage-gadget64");
  if (!fs.existsSync(bootDirPath) || !fs.statSync(bootDirPath).isDirectory()) {
    throw new Error("usbboot source missing mass-storage-gadget64 directory after build.");
  }

  const binaryName = process.platform === "win32" ? "rpiboot.exe" : "rpiboot";
  syncToToolDirs(binaryPath, bootDirPath, binaryName);
}

main();
