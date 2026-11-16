// Formatting shared by the cloud browser and its upload panel.

const UNITS = ["KiB", "MiB", "GiB", "TiB", "PiB"];

/** A byte count in binary units ("3.0 MiB"). */
export function formatSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${UNITS[unit]}`;
}

/** A file or folder name as shown; S3 allows an empty one. */
export function shownName(name: string): string {
  return name === "" ? "(no name)" : name;
}

const DECIMAL_UNITS = ["kB", "MB", "GB", "TB", "PB"];

/** A byte count in decimal units ("1.23 GB"), as providers bill storage. */
export function formatDecimalSize(bytes: number): string {
  if (bytes < 1000) {
    return `${bytes} bytes`;
  }
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < DECIMAL_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 2 : value < 100 ? 1 : 0)} ${DECIMAL_UNITS[unit]}`;
}

/** How long ago `ms` was, in words ("just now", "5 minutes ago", "3 days ago"). */
export function formatAgo(ms: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) {
    return "just now";
  }
  const steps: [number, string][] = [
    [60, "minute"],
    [3600, "hour"],
    [86400, "day"],
  ];
  let unit = steps[0]!;
  for (const step of steps) {
    if (seconds >= step[0]) {
      unit = step;
    }
  }
  const count = Math.floor(seconds / unit[0]);
  return `${count} ${unit[1]}${count === 1 ? "" : "s"} ago`;
}
