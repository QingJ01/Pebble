import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const BUNDLE_TARGETS_BY_PLATFORM = {
  win32: "nsis",
  darwin: "app,dmg",
};

export function bundleTargetsForPlatform(platform = process.platform) {
  const targets = BUNDLE_TARGETS_BY_PLATFORM[platform];
  if (!targets) {
    throw new Error(
      `Unsupported desktop package platform '${platform}'. Use pnpm tauri build --bundles <targets> for this platform.`,
    );
  }
  return targets;
}

export function tauriBuildArgsForPlatform(platform = process.platform, extraArgs = []) {
  return ["tauri", "build", "--bundles", bundleTargetsForPlatform(platform), ...extraArgs];
}

function isEntrypoint() {
  return process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
}

if (isEntrypoint()) {
  let args;
  try {
    args = tauriBuildArgsForPlatform(process.platform, process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }

  const result = spawnSync("pnpm", args, {
    stdio: "inherit",
    shell: process.platform === "win32",
  });

  process.exit(result.status ?? 1);
}
