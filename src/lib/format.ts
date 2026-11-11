/** Small display formatters shared by the settings UI. */

const BYTES_PER_UNIT = 1024;
const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(BYTES_PER_UNIT)), UNITS.length - 1);
  const value = bytes / BYTES_PER_UNIT ** exponent;
  const unit = UNITS[exponent] ?? "B";
  return `${value >= 100 || exponent === 0 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}

export function formatPercent(done: number, total: number): string {
  if (total <= 0) return "0%";
  return `${Math.min(100, Math.floor((done / total) * 100))}%`;
}

export function formatSeconds(ms: number): string {
  return `${(ms / 1000).toFixed(1).replace(".", ",")} sn`;
}

export function formatClock(epochMs: number): string {
  return new Date(epochMs).toLocaleTimeString("tr-TR", { hour: "2-digit", minute: "2-digit" });
}

/** Random id for new snippets; uniqueness is re-validated by the backend. */
export function newId(): string {
  return crypto.randomUUID();
}
