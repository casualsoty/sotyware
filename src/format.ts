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
