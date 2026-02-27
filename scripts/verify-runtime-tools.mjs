/* global process, console */

import fs from "node:fs";
import path from "node:path";

function parseArgs(argv) {
  const args = { target: null };
  for (let index = 2; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--target") {
      args.target = argv[index + 1] ?? null;
      index += 1;
    }
  }
  return args;
}

function hostProfile() {
  const platformMap = {
    linux: "linux",
    darwin: "macos",
    win32: "windows"
  };
  const archMap = {
    x64: "x64",
    arm64: "arm64",
    ia32: "x86"
  };

  const platform = platformMap[process.platform];
  const arch = archMap[process.arch];
  if (!platform || !arch) {
    throw new Error(`Unsupported host platform for tool verification: ${process.platform}/${process.arch}`);
  }
  return `${platform}-${arch}`;
}

function profilesForTarget(target) {
  if (!target) {
    return [hostProfile()];
  }

  const table = {
    "x86_64-unknown-linux-gnu": ["linux-x64"],
    "aarch64-unknown-linux-gnu": ["linux-arm64"],
    "x86_64-pc-windows-msvc": ["windows-x64"],
    "aarch64-pc-windows-msvc": ["windows-arm64"],
    "x86_64-apple-darwin": ["macos-x64"],
    "aarch64-apple-darwin": ["macos-arm64"],
    "universal-apple-darwin": ["macos-x64", "macos-arm64"]
  };

  return table[target] ?? [];
}

function binaryCandidates(rootDir, profile, binary) {
  const platform = profile.split("-")[0];
  return [
    path.join(rootDir, "tools", profile, binary),
    path.join(rootDir, "tools", platform, binary),
    path.join(rootDir, "tools", binary)
  ];
}

function directoryCandidates(rootDir, profile, directoryName) {
  const platform = profile.split("-")[0];
  return [
    path.join(rootDir, "tools", profile, directoryName),
    path.join(rootDir, "tools", platform, directoryName),
    path.join(rootDir, "tools", directoryName)
  ];
}

function firstExistingPath(candidates) {
  for (const candidate of candidates) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return candidate;
    }
  }
  return null;
}

function firstExistingDirectory(candidates) {
  for (const candidate of candidates) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isDirectory()) {
      return candidate;
    }
  }
  return null;
}

function verifyProfile(rootDir, profile, manifest) {
  const profileConfig = manifest.profiles?.[profile];
  if (!profileConfig) {
    throw new Error(`No tool profile configured for ${profile}. Add it to tools/manifest.json.`);
  }

  const missingRequired = [];
  const missingRequiredDirs = [];
  const missingOptional = [];
  const found = [];

  for (const requirement of profileConfig.requiredBundled ?? []) {
    const located = firstExistingPath(binaryCandidates(rootDir, profile, requirement.binary));
    if (!located) {
      missingRequired.push(requirement);
      continue;
    }
    found.push({ ...requirement, path: located, required: true });
  }

  for (const requirement of profileConfig.optionalBundled ?? []) {
    const located = firstExistingPath(binaryCandidates(rootDir, profile, requirement.binary));
    if (!located) {
      missingOptional.push(requirement);
      continue;
    }
    found.push({ ...requirement, path: located, required: false });
  }

  for (const directoryName of profileConfig.requiredBundledDirs ?? []) {
    const located = firstExistingDirectory(directoryCandidates(rootDir, profile, directoryName));
    if (!located) {
      missingRequiredDirs.push(directoryName);
      continue;
    }
    found.push({
      name: directoryName,
      path: located,
      required: true,
      kind: "dir"
    });
  }

  return {
    profile,
    found,
    missingRequired,
    missingRequiredDirs,
    missingOptional,
    hostRuntimeDependencies: profileConfig.hostRuntimeDependencies ?? []
  };
}

function main() {
  const args = parseArgs(process.argv);
  const rootDir = process.cwd();
  const manifestPath = path.join(rootDir, "tools", "manifest.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

  const profiles = profilesForTarget(args.target);
  if (profiles.length === 0) {
    throw new Error(`Unsupported target triple: ${args.target}`);
  }

  let hasMissingRequired = false;

  console.log(`Runtime tool verification for ${profiles.join(", ")}`);
  for (const profile of profiles) {
    const result = verifyProfile(rootDir, profile, manifest);
    console.log(`\n[${profile}]`);

    if (result.found.length > 0) {
      console.log("Found bundled tools:");
      for (const entry of result.found) {
        const tag = entry.required ? "required" : "optional";
        const kind = entry.kind === "dir" ? "directory" : "binary";
        console.log(`- ${entry.name} (${tag}, ${kind}) -> ${path.relative(rootDir, entry.path)}`);
      }
    }

    if (result.missingOptional.length > 0) {
      console.log("Missing optional bundled tools:");
      for (const entry of result.missingOptional) {
        console.log(`- ${entry.name} (${entry.binary})`);
      }
    }

    if (result.hostRuntimeDependencies.length > 0) {
      console.log(`Host runtime dependencies (not bundled): ${result.hostRuntimeDependencies.join(", ")}`);
    }

    if (result.missingRequired.length > 0) {
      hasMissingRequired = true;
      console.log("Missing required bundled tools:");
      for (const entry of result.missingRequired) {
        console.log(`- ${entry.name} (${entry.binary})`);
      }
    }

    if (result.missingRequiredDirs.length > 0) {
      hasMissingRequired = true;
      console.log("Missing required bundled directories:");
      for (const directoryName of result.missingRequiredDirs) {
        console.log(`- ${directoryName}`);
      }
    }
  }

  if (hasMissingRequired) {
    process.exitCode = 1;
    console.error("\nTool verification failed: missing required bundled tools.");
    return;
  }

  console.log("\nTool verification passed.");
}

main();
