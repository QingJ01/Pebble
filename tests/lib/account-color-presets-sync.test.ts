import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { ACCOUNT_COLOR_PRESETS } from "@/lib/accountColors";

function rustPresets(...segments: string[]): string[] {
  const source = readFileSync(join(process.cwd(), ...segments), "utf8");
  const block = source.match(/const ACCOUNT_COLOR_PRESETS: \[&str; \d+\] = \[([\s\S]*?)\];/);
  expect(block, segments.join("/")).not.toBeNull();
  return [...block![1].matchAll(/"(#[0-9a-fA-F]{6})"/g)].map((m) => m[1].toLowerCase());
}

describe("account colour presets", () => {
  const ts = ACCOUNT_COLOR_PRESETS.map((preset) => preset.color.toLowerCase());

  it("match the Rust default-colour copy", () => {
    expect(rustPresets("src-tauri", "src", "account_colors.rs")).toEqual(ts);
  });

  it("match the migration backfill copy", () => {
    expect(rustPresets("crates", "pebble-store", "src", "migrations", "mod.rs")).toEqual(ts);
  });
});
