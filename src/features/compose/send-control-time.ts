/** datetime-local has no timezone: reject gaps normalized by Date (DST spring forward). */
export function scheduledInstant(value: string, now = Date.now()): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value);
  if (!match) return null;
  const [, y, m, d, h, min] = match.map(Number);
  const date = new Date(y, m - 1, d, h, min);
  if (date.getFullYear() !== y || date.getMonth() !== m - 1 || date.getDate() !== d
    || date.getHours() !== h || date.getMinutes() !== min) return null;
  const instant = Math.floor(date.getTime() / 1000);
  return instant > Math.floor(now / 1000) && instant <= Math.floor(now / 1000) + 366 * 86400
    ? instant : null;
}

export function localScheduleValue(instant: number): string {
  const date = new Date(instant * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

export function timezoneLabel(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

export function countdownSeconds(dueAt: number, now = Date.now()): number {
  return Math.max(0, Math.ceil(dueAt - now / 1000));
}
