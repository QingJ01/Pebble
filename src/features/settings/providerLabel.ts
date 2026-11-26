/** Display name for a provider id ("imap" → "IMAP"). Never capitalize ids with CSS. */
const PROVIDER_LABELS: Record<string, string> = {
  imap: "IMAP",
  pop3: "POP3",
  gmail: "Gmail",
  outlook: "Outlook",
};

export function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider.toLowerCase()] ?? provider;
}
