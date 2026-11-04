import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";

describe("macOS release configuration", () => {
  it("keeps release version metadata in sync", () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));
    const tauriConfig = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );
    const cargoToml = readFileSync(resolve(process.cwd(), "src-tauri", "Cargo.toml"), "utf8");
    const changelog = readFileSync(resolve(process.cwd(), "CHANGELOG.md"), "utf8");
    const releaseWorkflow = readFileSync(resolve(process.cwd(), ".github", "workflows", "release.yml"), "utf8");
    const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

    expect(packageJson.version).toBe("0.1.6");
    expect(tauriConfig.version).toBe(packageJson.version);
    expect(cargoVersion).toBe(packageJson.version);
    expect(changelog).toContain(`## [${packageJson.version}] - `);
    expect(changelog).toContain(`[Unreleased]: https://github.com/QingJ01/Pebble/compare/v${packageJson.version}...HEAD`);
    expect(releaseWorkflow).toContain(`default: v${packageJson.version}`);
  });

  it("defines explicit desktop build scripts for Windows, macOS, and Linux bundles", () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));

    expect(packageJson.scripts["build:windows"]).toBeTypeOf("string");
    expect(packageJson.scripts["build:macos"]).toBeTypeOf("string");
    expect(packageJson.scripts["build:linux"]).toBeTypeOf("string");
    expect(packageJson.scripts["build:windows"]).toContain("--bundles nsis");
    expect(packageJson.scripts["build:macos"]).toContain("--bundles app,dmg");
    expect(packageJson.scripts["build:linux"]).toContain("--bundles appimage,deb,rpm");
  });

  it("routes the generic build command to platform-specific bundles", async () => {
    const packageJson = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8"));
    const buildScriptPath = resolve(process.cwd(), "scripts", "build-tauri.mjs");
    const buildScriptSource = readFileSync(buildScriptPath, "utf8");
    const buildScript = await import(pathToFileURL(buildScriptPath).href);

    expect(packageJson.scripts.build).toBe("node scripts/build-tauri.mjs");
    expect(buildScriptSource).not.toMatch(/^#!/);
    expect(buildScript.bundleTargetsForPlatform("win32")).toBe("nsis");
    expect(buildScript.bundleTargetsForPlatform("darwin")).toBe("app,dmg");
    expect(buildScript.bundleTargetsForPlatform("linux")).toBe("appimage,deb,rpm");
    expect(buildScript.tauriBuildEnvironmentForPlatform("linux", {})).toEqual({
      NO_STRIP: "1",
    });
    expect(
      buildScript.tauriBuildEnvironmentForPlatform("linux", {
        NO_STRIP: "0",
      }),
    ).toEqual({ NO_STRIP: "0" });
    expect(buildScript.tauriBuildEnvironmentForPlatform("win32", {})).toEqual({});
  });

  it("keeps desktop notification clicks routable to the target message", () => {
    const indexingSource = readFileSync(
      resolve(process.cwd(), "src-tauri", "src", "commands", "indexing.rs"),
      "utf8",
    ).replace(/\r\n/g, "\n");
    const eventsSource = readFileSync(resolve(process.cwd(), "src-tauri", "src", "events.rs"), "utf8").replace(
      /\r\n/g,
      "\n",
    );
    const cargoToml = readFileSync(resolve(process.cwd(), "src-tauri", "Cargo.toml"), "utf8");

    expect(eventsSource).toContain('pub const MAIL_NOTIFICATION_OPEN: &str = "mail:notification-open";');
    expect(eventsSource).not.toContain('#[cfg(windows)]\npub const MAIL_NOTIFICATION_OPEN');
    expect(indexingSource).toContain("fn notification_open_payload");
    expect(indexingSource).not.toContain("#[cfg(any(windows, test))]\nfn notification_open_payload");
    expect(indexingSource).toContain("fn open_message_from_notification");
    expect(indexingSource).not.toContain("#[cfg(windows)]\nfn open_message_from_notification");
    expect(indexingSource).toContain("fn show_linux_new_mail_notification");
    expect(indexingSource).toContain("wait_for_action");
    expect(indexingSource).toContain("fn show_macos_new_mail_notification");
    expect(indexingSource).toContain("wait_for_click(true)");
    expect(cargoToml).toContain('[target.\'cfg(target_os = "linux")\'.dependencies]');
    expect(cargoToml).toContain('notify-rust = "4"');
    expect(cargoToml).toContain('[target.\'cfg(target_os = "macos")\'.dependencies]');
    expect(cargoToml).toContain('mac-notification-sys = "0.6"');
  });

  it("includes a macOS icon in the Tauri bundle config", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri", "tauri.conf.json"), "utf8"),
    );

    expect(config.bundle.icon).toContain("icons/icon.icns");
    expect(existsSync(resolve(process.cwd(), "src-tauri", "icons", "icon.icns"))).toBe(true);
  });

  it("enables native credential storage backends for Windows, macOS, and Linux", () => {
    const cargoToml = readFileSync(resolve(process.cwd(), "Cargo.toml"), "utf8");

    expect(cargoToml).toContain('"apple-native"');
    expect(cargoToml).toContain('"windows-native"');
    expect(cargoToml).toContain('"linux-native-sync-persistent"');
    expect(cargoToml).toContain('"crypto-rust"');
  });

  it("runs package builds on Windows, macOS, and Linux in CI", () => {
    const ciWorkflow = readFileSync(resolve(process.cwd(), ".github", "workflows", "ci.yml"), "utf8");

    expect(ciWorkflow).toContain("windows-latest");
    expect(ciWorkflow).toContain("macos-15");
    expect(ciWorkflow).toContain("ubuntu-latest");
    expect(ciWorkflow).toContain("Install Linux system dependencies");
    expect(ciWorkflow).toContain("pnpm ${{ matrix.build_script }}");
    expect(ciWorkflow).toContain("Upload Linux package artifacts");
    expect(ciWorkflow).toContain("target/release/bundle/appimage/*.AppImage");
    expect(ciWorkflow).toContain("target/release/bundle/deb/*.deb");
    expect(ciWorkflow).toContain("target/release/bundle/rpm/*.rpm");
    expect(ciWorkflow).toContain("build:windows");
    expect(ciWorkflow).toContain("build:macos");
    expect(ciWorkflow).toContain("build:linux");
  });

  it("uploads unsigned macOS DMG artifacts during tagged releases", () => {
    const releaseWorkflow = readFileSync(
      resolve(process.cwd(), ".github", "workflows", "release.yml"),
      "utf8",
    );

    expect(releaseWorkflow).toContain("macOS Release");
    expect(releaseWorkflow).toContain("runs-on: ${{ matrix.os }}");
    expect(releaseWorkflow).toContain("macos-15");
    expect(releaseWorkflow).toContain("macos-15-intel");
    expect(releaseWorkflow).toContain("aarch64-apple-darwin");
    expect(releaseWorkflow).toContain("x86_64-apple-darwin");
    expect(releaseWorkflow).toContain("pnpm tauri build --target ${{ matrix.target }} --bundles app,dmg");
    expect(releaseWorkflow).toContain("target/${{ matrix.target }}/release/bundle/dmg");
    expect(releaseWorkflow).toContain("pebble-macos-${{ matrix.arch }}-${{ env.PEBBLE_VERSION }}");
  });

  it("uploads Linux package artifacts during tagged releases", () => {
    const releaseWorkflow = readFileSync(
      resolve(process.cwd(), ".github", "workflows", "release.yml"),
      "utf8",
    );

    expect(releaseWorkflow).toContain("Linux Package Release");
    expect(releaseWorkflow).toContain("runs-on: ubuntu-latest");
    expect(releaseWorkflow).toContain("Install Linux system dependencies");
    expect(releaseWorkflow).toContain("pnpm build:linux");
    expect(releaseWorkflow).toContain("target/release/bundle/appimage");
    expect(releaseWorkflow).toContain("target/release/bundle/deb");
    expect(releaseWorkflow).toContain("target/release/bundle/rpm");
    expect(releaseWorkflow).toContain("*.AppImage");
    expect(releaseWorkflow).toContain("*.deb");
    expect(releaseWorkflow).toContain("*.rpm");
    expect(releaseWorkflow).toContain("pebble-linux-packages-${{ env.PEBBLE_VERSION }}");
  });

  it("publishes a release only after every platform artifact is available", () => {
    const releaseWorkflow = readFileSync(
      resolve(process.cwd(), ".github", "workflows", "release.yml"),
      "utf8",
    ).replace(/\r\n/g, "\n");
    const publishIndex = releaseWorkflow.indexOf("\n  publish:");

    expect(publishIndex).toBeGreaterThan(0);
    expect(releaseWorkflow).toContain("needs: [windows, linux, macos]");
    expect(releaseWorkflow).toContain("actions/download-artifact@v4");
    expect(releaseWorkflow).toContain("merge-multiple: true");
    expect(releaseWorkflow).toContain("appimages=(release-artifacts/*.AppImage)");
    expect(releaseWorkflow).toContain("debs=(release-artifacts/*.deb)");
    expect(releaseWorkflow).toContain("rpms=(release-artifacts/*.rpm)");
    expect(releaseWorkflow).toContain("macos_arm=(release-artifacts/*-arm64.dmg)");
    expect(releaseWorkflow).toContain("macos_x64=(release-artifacts/*-x64.dmg)");
    expect(releaseWorkflow).toContain("sha256sum --check --strict *.sha256");
    expect(releaseWorkflow).toContain("group: release-${{ inputs.tag || github.ref_name }}");
    expect(releaseWorkflow).toContain("cancel-in-progress: false");
    expect(releaseWorkflow).toContain('if gh release view "$tag"');
    expect(releaseWorkflow).toContain('gh release create "$tag" --draft');
    expect(releaseWorkflow).toContain('gh release upload "$tag" release-artifacts/*');
    expect(releaseWorkflow).toContain('gh release edit "$tag" --draft=false');
    expect(releaseWorkflow).not.toContain("gh release edit \"$tag\" --draft --title");
    expect(releaseWorkflow).not.toContain("--clobber");
    expect(releaseWorkflow.slice(0, publishIndex)).not.toContain("gh release create");
    expect(releaseWorkflow.slice(0, publishIndex)).not.toContain("gh release upload");
  });

  it("does not interpolate an untrusted release tag directly into shell scripts", () => {
    const releaseWorkflow = readFileSync(
      resolve(process.cwd(), ".github", "workflows", "release.yml"),
      "utf8",
    ).replace(/\r\n/g, "\n");
    const lines = releaseWorkflow.split("\n");
    const runBlocks: string[] = [];

    for (let index = 0; index < lines.length; index += 1) {
      const match = lines[index]?.match(/^(\s*)run:\s*(.*)$/);
      if (!match) continue;
      const indent = match[1].length;
      const block = [match[2]];
      while (index + 1 < lines.length) {
        const nextLine = lines[index + 1] ?? "";
        const nextIndent = nextLine.match(/^\s*/)?.[0].length ?? 0;
        if (nextLine.trim() && nextIndent <= indent) break;
        block.push(nextLine);
        index += 1;
      }
      runBlocks.push(block.join("\n"));
    }

    expect(releaseWorkflow).toContain("RELEASE_TAG: ${{ inputs.tag || github.ref_name }}");
    expect(runBlocks.join("\n")).not.toContain("${{ inputs.tag || github.ref_name }}");
    expect(runBlocks.join("\n")).not.toContain("${{ inputs.tag || github.ref }}");
  });
});
