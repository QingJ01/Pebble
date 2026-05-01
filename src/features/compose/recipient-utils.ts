export function isValidEmailAddress(value: string) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value.trim());
}

export function mergePendingRecipient(addresses: string[], pendingValue: string) {
  const pending = pendingValue.trim();
  if (!pending || !isValidEmailAddress(pending) || addresses.includes(pending)) {
    return addresses;
  }
  return [...addresses, pending];
}
