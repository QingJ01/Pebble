const STARTUP_PREFIX = "[startup]";

type StartupWindow = Window & { __splashStart?: number };

export function getStartupElapsedMs(now = Date.now()) {
  const splashStart = (window as StartupWindow).__splashStart;
  if (typeof splashStart !== "number") return 0;
  return Math.max(0, Math.round(now - splashStart));
}

export function formatStartupTiming(label: string, elapsedMs: number) {
  return `${STARTUP_PREFIX} ${label}: ${elapsedMs}ms since splash start`;
}

export function logStartupTiming(label: string, now = Date.now()) {
  const elapsedMs = getStartupElapsedMs(now);
  console.info(formatStartupTiming(label, elapsedMs));
  return elapsedMs;
}
