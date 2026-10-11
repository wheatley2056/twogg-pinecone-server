/** Die API speichert Zeiten als Unix-Sekunden; React-Admin arbeitet mit Date/ISO-Strings. */
export const fromUnix = (v: unknown): Date => new Date(Number(v) * 1000);
export const toInputDate = (v: unknown) => (typeof v === "number" ? new Date(v * 1000).toISOString() : v);
export const fromInputDate = (v: unknown) => (v ? Math.floor(new Date(String(v)).getTime() / 1000) : null);

export function bytes(n: number) {
  return n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1e3))} kB`;
}
