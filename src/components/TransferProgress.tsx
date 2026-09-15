import { formatBytes, formatSpeed } from "@/lib/format";

/** The bar every download and install shares: percentage, speed and totals. */
export function TransferProgress({
  percent,
  receivedBytes,
  totalBytes,
  bytesPerSecond,
  label,
}: {
  percent?: number;
  receivedBytes?: number;
  totalBytes?: number;
  bytesPerSecond?: number;
  label?: string;
}) {
  const done =
    percent ??
    (totalBytes && totalBytes > 0 ? ((receivedBytes ?? 0) / totalBytes) * 100 : undefined);

  return (
    <div>
      <div className="h-1.5 overflow-hidden rounded-full bg-default-200">
        <div
          className={`h-full rounded-full bg-primary transition-[width] ${done === undefined ? "animate-pulse" : ""}`}
          style={{ width: `${done ?? 100}%` }}
        />
      </div>
      <div className="mt-1 flex justify-between text-[11px] text-foreground/45">
        <span>{label ?? (done === undefined ? "Working…" : `${Math.round(done)}%`)}</span>
        <span>
          {bytesPerSecond !== undefined && bytesPerSecond > 0 ? formatSpeed(bytesPerSecond) : ""}
          {totalBytes !== undefined && totalBytes > 0
            ? ` ${formatBytes(receivedBytes ?? 0)} of ${formatBytes(totalBytes)}`
            : receivedBytes
              ? ` ${formatBytes(receivedBytes)}`
              : ""}
        </span>
      </div>
    </div>
  );
}
