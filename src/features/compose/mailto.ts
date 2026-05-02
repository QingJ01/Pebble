import type { ComposePrefill } from "@/stores/compose.store";

function decodeAddressPart(value: string) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function parseQueryHeaders(query: string) {
  const headers = new Map<string, string[]>();
  for (const part of query.split("&")) {
    if (!part) continue;
    const separator = part.indexOf("=");
    const rawName = separator >= 0 ? part.slice(0, separator) : part;
    const rawValue = separator >= 0 ? part.slice(separator + 1) : "";
    const name = decodeAddressPart(rawName).trim().toLowerCase();
    if (!name) continue;
    headers.set(name, [...(headers.get(name) ?? []), decodeAddressPart(rawValue)]);
  }
  return headers;
}

function parseAddressList(value: string | null) {
  if (!value) return [];
  return decodeAddressPart(value)
    .split(/[;,]/)
    .map((address) => address.trim())
    .filter(Boolean);
}

function parseAddressHeaders(values: string[] | undefined) {
  return (values ?? []).flatMap((value) => parseAddressList(value));
}

function uniqueAddresses(values: string[]) {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (const value of values) {
    const key = value.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(value);
  }
  return unique;
}

function firstHeader(headers: Map<string, string[]>, name: string) {
  return headers.get(name)?.[0] ?? "";
}

export function parseMailtoUrl(url: string): ComposePrefill | null {
  if (!url.toLowerCase().startsWith("mailto:")) return null;

  const withoutScheme = url.slice("mailto:".length);
  const queryStart = withoutScheme.indexOf("?");
  const path = queryStart >= 0 ? withoutScheme.slice(0, queryStart) : withoutScheme;
  const query = queryStart >= 0 ? withoutScheme.slice(queryStart + 1) : "";
  const headers = parseQueryHeaders(query);

  return {
    to: uniqueAddresses([...parseAddressList(path), ...parseAddressHeaders(headers.get("to"))]),
    cc: uniqueAddresses(parseAddressHeaders(headers.get("cc"))),
    bcc: uniqueAddresses(parseAddressHeaders(headers.get("bcc"))),
    subject: firstHeader(headers, "subject"),
    body: firstHeader(headers, "body"),
  };
}
