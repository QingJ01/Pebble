const STORAGE_KEY = "pebble-signatures";

export function getSignature(accountId: string): string {
  try {
    const data = JSON.parse(localStorage.getItem(STORAGE_KEY) || "{}");
    return data[accountId] || "";
  } catch { return ""; }
}

export function setSignature(accountId: string, signature: string): void {
  try {
    const data = JSON.parse(localStorage.getItem(STORAGE_KEY) || "{}");
    if (signature.trim()) {
      data[accountId] = signature;
    } else {
      delete data[accountId];
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(data));
  } catch { /* quota exceeded */ }
}
