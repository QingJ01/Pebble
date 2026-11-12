export const PROXY_PORT_ERROR_KEY = "accountSetup.proxyPortInvalid";

export function parseProxyPort(value: string, errorMessage: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const port = Number(trimmed);
  if (!/^\d+$/.test(trimmed) || !Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(errorMessage);
  }
  return port;
}
