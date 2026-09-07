/** Formatting helpers shared across views. */

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number, fractionDigits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** exponent;
  // Whole bytes never want a decimal point.
  return `${value.toFixed(exponent === 0 ? 0 : fractionDigits)} ${UNITS[exponent]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatPlaytime(minutes: number): string {
  if (minutes <= 0) return "Never played";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** Remaining time for a transfer, or null when it cannot be estimated yet. */
export function formatEta(receivedBytes: number, totalBytes: number, bytesPerSecond: number): string | null {
  if (bytesPerSecond <= 0 || totalBytes <= receivedBytes) return null;
  const seconds = Math.round((totalBytes - receivedBytes) / bytesPerSecond);
  if (seconds < 60) return `${seconds}s left`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} min left`;
  return `${(seconds / 3600).toFixed(1)} h left`;
}
