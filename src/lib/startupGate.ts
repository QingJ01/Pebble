import { invoke } from "@tauri-apps/api/core";

/** Backend startup stage (src-tauri/src/startup.rs). */
export type StartupStage =
  | { stage: "starting" }
  | { stage: "migrating"; done: number; total: number }
  | { stage: "ready" }
  | { stage: "failed"; message: string };

const POLL_MS = 120;

/**
 * Wait until the backend has opened the database (and finished upgrading
 * it, if an upgrade was pending), reporting each stage as it changes.
 * Resolves when ready; rejects with the backend's message if startup failed.
 * A backend without the status command counts as ready.
 */
export async function waitForStartup(
  onStage: (stage: StartupStage) => void = () => {},
  invokeFn: (cmd: string) => Promise<unknown> = (cmd) => invoke(cmd),
  pollMs = POLL_MS,
): Promise<void> {
  let last = "";
  for (;;) {
    let stage: StartupStage;
    try {
      stage = ((await invokeFn("get_startup_status")) as StartupStage | null) ?? { stage: "ready" };
    } catch {
      stage = { stage: "ready" };
    }
    const key = JSON.stringify(stage);
    if (key !== last) {
      last = key;
      onStage(stage);
    }
    if (stage.stage === "ready") return;
    if (stage.stage === "failed") throw new Error(stage.message);
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
}

interface SplashApi {
  progress: (fraction: number | null, label: string, hint: string) => void;
  fail: (title: string, detail: string) => void;
}

/** The boot splash's progress API (index.html), when the splash is up. */
export function splashApi(): SplashApi | null {
  return (window as unknown as { __pebbleSplash?: SplashApi }).__pebbleSplash ?? null;
}
