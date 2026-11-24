// Formatting shared by the pages: sizes, counts, dates and times.
//
// Every number and date the page writes is in en-US (`LOCALE`), the language
// of its text, whatever the webview's language (WebView2 follows the Windows
// display language, WebKitGTK the environment's locale). Formats left to the
// webview mixed with the fixed ones in one line under another language:
// "5.00 GB in 1 234 files" in fr-FR, "5.00 GB in 1.234 files" in de-DE. So
// every locale-sensitive formatting call in src/ names LOCALE, which
// format.test.ts checks; sorting (fileList.ts's collator, the macro library)
// still follows the webview's language.

/** The locale of every number and date the page writes. */
export const LOCALE = "en-US";

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

/** A count, rounded to a whole number and grouped ("1,234"). */
export function formatCount(n: number): string {
  return Math.round(n).toLocaleString(LOCALE);
}

/** A date and time ("3/4/2026, 2:05:00 PM" without options). */
export function formatDateTime(date: Date, options?: Intl.DateTimeFormatOptions): string {
  return date.toLocaleString(LOCALE, options);
}

/** A date ("3/4/2026" without options). */
export function formatDate(date: Date, options?: Intl.DateTimeFormatOptions): string {
  return date.toLocaleDateString(LOCALE, options);
}

/** A time of day ("2:05:00 PM" without options). */
export function formatTime(date: Date, options?: Intl.DateTimeFormatOptions): string {
  return date.toLocaleTimeString(LOCALE, options);
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
