const WINDOWS_RESERVED_NAMES = new Set([
  "CON",
  "PRN",
  "AUX",
  "NUL",
  "COM1",
  "COM2",
  "COM3",
  "COM4",
  "COM5",
  "COM6",
  "COM7",
  "COM8",
  "COM9",
  "LPT1",
  "LPT2",
  "LPT3",
  "LPT4",
  "LPT5",
  "LPT6",
  "LPT7",
  "LPT8",
  "LPT9",
]);

export function sanitizeFilename(name: string, fallback = "download"): string {
  const base = name.split(/[/\\]/).pop() ?? name;
  const sanitized = base
    .replace(/\.\./g, "")
    .replace(/[<>:"|?*]/g, "_")
    .replace(/[\u0000-\u001f]/g, "")
    .trim()
    .replace(/^[.\s]+/, "")
    .replace(/[.\s]+$/g, "");

  if (!sanitized) {
    return fallback;
  }

  const stem = sanitized.split(".")[0]?.trim().toUpperCase() ?? "";
  if (WINDOWS_RESERVED_NAMES.has(stem)) {
    return fallback;
  }

  return sanitized;
}
