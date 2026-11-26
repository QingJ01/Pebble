// Account colours stay clear of the brand orange, the star/warning amber and the
// danger red so an account badge never reads as a status. Ordered so the first
// accounts get the most distinct hues; keep in sync with
// src-tauri/src/account_colors.rs.
export const ACCOUNT_COLOR_PRESETS = [
  { name: "Sky", color: "#0ea5e9" },
  { name: "Emerald", color: "#10b981" },
  { name: "Violet", color: "#8b5cf6" },
  { name: "Pink", color: "#ec4899" },
  { name: "Lime", color: "#65a30d" },
  { name: "Indigo", color: "#6366f1" },
  { name: "Teal", color: "#14b8a6" },
  { name: "Fuchsia", color: "#c026d3" },
  { name: "Blue", color: "#3b82f6" },
  { name: "Slate", color: "#64748b" },
  { name: "Cyan", color: "#06b6d4" },
  { name: "Navy", color: "#1e40af" },
] as const;

const ACCOUNT_COLOR_PALETTE = ACCOUNT_COLOR_PRESETS.map((preset) => preset.color);
const FALLBACK_COLOR = ACCOUNT_COLOR_PRESETS[0].color;

interface AccountColorSource {
  id?: string | null;
  email?: string | null;
  display_name?: string | null;
  color?: string | null;
}

function isValidHexColor(color: string | null | undefined): color is string {
  return !!color && /^#[0-9a-fA-F]{6}$/.test(color);
}

function normalizeColor(color: string): string {
  return color.toLowerCase();
}

export function deriveAccountColor(seed: string): string {
  let hash = 0;
  for (let index = 0; index < seed.length; index += 1) {
    hash = (hash * 31 + seed.charCodeAt(index)) >>> 0;
  }
  return ACCOUNT_COLOR_PALETTE[hash % ACCOUNT_COLOR_PALETTE.length] ?? FALLBACK_COLOR;
}

export function getAccountColor(
  account: AccountColorSource | null | undefined,
  fallbackSeed = "",
): string {
  if (isValidHexColor(account?.color)) {
    return normalizeColor(account.color);
  }

  return deriveAccountColor(account?.id ?? account?.email ?? fallbackSeed);
}

export function assignAccountColors(accounts: AccountColorSource[]): Map<string, string> {
  const usedColors = new Set<string>();
  const colorsByAccountId = new Map<string, string>();

  for (const account of accounts) {
    if (!account.id || !isValidHexColor(account.color)) continue;
    const color = normalizeColor(account.color);
    usedColors.add(color);
    colorsByAccountId.set(account.id, color);
  }

  for (const account of accounts) {
    if (!account.id || colorsByAccountId.has(account.id)) continue;
    const preset = ACCOUNT_COLOR_PRESETS.find((candidate) => !usedColors.has(candidate.color));
    const color = preset?.color ?? deriveAccountColor(account.id ?? account.email ?? "");
    usedColors.add(color);
    colorsByAccountId.set(account.id, color);
  }

  return colorsByAccountId;
}

export function getAccountLabel(account: AccountColorSource | null | undefined, fallback = ""): string {
  if (!account) return fallback;
  const name = account.display_name?.trim();
  const email = account.email?.trim();
  if (name && email) return `${name} <${email}>`;
  return name || email || fallback;
}
