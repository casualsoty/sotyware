// The page writes every number and date in en-US, the language of its text,
// whatever the webview's language (src/format.ts, since 2026-11-24). A webview
// in another language is stood in for by `withDefaultLocale` (testLocale.ts);
// each check first shows that the default really is that language, so the
// en-US results cannot come from the tests' own default.
import { describe, expect, it } from "vitest";
import { formatMoney } from "./cloudUsage";
import { formatCount, formatDate, formatDateTime, formatDecimalSize, formatSize, formatTime, LOCALE } from "./format";
import { withDefaultLocale } from "./testLocale";

/** Webview languages that write numbers or dates unlike en-US: a space, a
 * point or Indian grouping for thousands, a decimal comma, other digits, the
 * day first, a 24-hour clock, month names in another language. */
const OTHER_LANGUAGES = ["fr-FR", "de-DE", "en-GB", "en-IN", "ar-EG"];

/** 14:05:06 on 4 March 2026 in local time: the page formats in the computer's
 * time zone, so the expected strings hold in any. */
const WHEN = new Date(2026, 2, 4, 14, 5, 6);

/** ICU versions differ in the space before "PM" in en-US: U+202F since CLDR
 * 42 in some, U+0020 in Node 24's. */
function spaced(s: string): string {
  return s.replace(/ /g, " ");
}

/** Fails unless the default locale differs from en-US: the plain call, as the
 * page made it before 2026-11-24, then writes the date another way (every
 * language in OTHER_LANGUAGES does; not all of them group numbers otherwise). */
function expectDefaultIsNotEnUs(language: string): void {
  expect(spaced(WHEN.toLocaleString()), language).not.toBe("3/4/2026, 2:05:06 PM");
}

describe("the page's numbers", () => {
  it("are written in en-US", () => {
    expect(LOCALE).toBe("en-US");
    expect(formatCount(1_234_567)).toBe("1,234,567");
    expect(formatCount(1234.5)).toBe("1,235");
    expect(formatCount(999)).toBe("999");
    expect(formatCount(0)).toBe("0");
    expect(formatSize(1023)).toBe("1023 B");
    expect(formatSize(3 * 1024 * 1024)).toBe("3.0 MiB");
    expect(formatSize(5 * 1024 ** 3)).toBe("5.0 GiB");
    expect(formatDecimalSize(1_234_567)).toBe("1.23 MB");
    expect(formatDecimalSize(5_000_000_000)).toBe("5.00 GB");
    expect(formatMoney(1.39, "USD")).toBe("$1.39");
    expect(formatMoney(6.49, "EUR")).toBe("€6.49");
    expect(formatMoney(1234.5, "USD")).toBe("$1,234.50");
    // A currency code Intl refuses: the code and two decimals.
    expect(formatMoney(1.5, "not a code")).toBe("not a code 1.50");
  });

  it("stay in en-US whatever the webview's language", async () => {
    for (const language of OTHER_LANGUAGES) {
      await withDefaultLocale(language, () => {
        expectDefaultIsNotEnUs(language);
        expect(formatCount(1_234_567), language).toBe("1,234,567");
        expect(formatCount(1234.5), language).toBe("1,235");
        expect(formatSize(5 * 1024 ** 3), language).toBe("5.0 GiB");
        expect(formatDecimalSize(5_000_000_000), language).toBe("5.00 GB");
        expect(formatMoney(1.39, "USD"), language).toBe("$1.39");
        expect(formatMoney(6.49, "EUR"), language).toBe("€6.49");
        expect(formatMoney(1234.5, "USD"), language).toBe("$1,234.50");
      });
    }
  });
});

describe("the page's dates and times", () => {
  it("are written in en-US, with the options a page gives", () => {
    expect(spaced(formatDateTime(WHEN))).toBe("3/4/2026, 2:05:06 PM");
    expect(formatDate(WHEN)).toBe("3/4/2026");
    expect(spaced(formatTime(WHEN))).toBe("2:05:06 PM");
    // The bucket browser's Last modified, the release date, the last check.
    expect(spaced(formatDateTime(WHEN, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }))).toBe(
      "Mar 4, 2026, 02:05 PM",
    );
    expect(formatDate(WHEN, { year: "numeric", month: "long", day: "numeric" })).toBe("March 4, 2026");
    expect(spaced(formatTime(WHEN, { hour: "2-digit", minute: "2-digit" }))).toBe("02:05 PM");
    // As before: the pages check for an invalid date themselves.
    expect(formatDateTime(new Date(Number.NaN))).toBe("Invalid Date");
  });

  it("stay in en-US whatever the webview's language", async () => {
    for (const language of OTHER_LANGUAGES) {
      await withDefaultLocale(language, () => {
        expectDefaultIsNotEnUs(language);
        expect(spaced(formatDateTime(WHEN)), language).toBe("3/4/2026, 2:05:06 PM");
        expect(formatDate(WHEN), language).toBe("3/4/2026");
        expect(spaced(formatTime(WHEN)), language).toBe("2:05:06 PM");
        expect(formatDate(WHEN, { year: "numeric", month: "long", day: "numeric" }), language).toBe("March 4, 2026");
      });
    }
  });
});

describe("withDefaultLocale", () => {
  it("changes the default only while it runs, and a named locale still wins", async () => {
    const inside = await withDefaultLocale("de-DE", () => [(1234).toLocaleString(), (1234).toLocaleString("en-US")]);
    expect(inside).toEqual(["1.234", "1,234"]);
    expect((1234).toLocaleString()).toBe("1,234");
    await expect(
      withDefaultLocale("fr-FR", () => {
        throw new Error("failed inside");
      }),
    ).rejects.toThrow("failed inside");
    expect((1234).toLocaleString()).toBe("1,234");
    // "zxx" (no linguistic content) has no data: Node's default, the
    // machine's, would be used, so it is refused.
    await expect(withDefaultLocale("zxx", () => undefined)).rejects.toThrow("no locale data for zxx");
  });
});

/** The formatting calls whose output depends on a locale. Sorting
 * (`localeCompare`, `Intl.Collator`) is not formatting: it still follows the
 * webview's language. */
const LOCALE_CALL =
  /\.(?:toLocaleString|toLocaleDateString|toLocaleTimeString)\(|\bIntl\.(?:NumberFormat|DateTimeFormat|RelativeTimeFormat|ListFormat|PluralRules|DisplayNames|DurationFormat)\(/g;

describe("every locale-sensitive formatting call in src/", () => {
  it("names LOCALE, so no format is left to the webview's language", () => {
    const sources = import.meta.glob<string>(["./**/*.ts", "./**/*.svelte", "!./**/*.test.ts", "!./testLocale.ts"], {
      query: "?raw",
      import: "default",
      eager: true,
    });
    // The glob must have found the page, or the rule would pass on nothing.
    expect(Object.keys(sources)).toEqual(expect.arrayContaining(["./format.ts", "./cloudUsage.ts", "./CloudUsage.svelte", "./AutoclickerMacros.svelte"]));
    const found: string[] = [];
    const unnamed: string[] = [];
    for (const [file, source] of Object.entries(sources)) {
      for (const call of source.matchAll(LOCALE_CALL)) {
        const at = call.index + call[0].length;
        const line = source.slice(0, at).split("\n").length;
        const where = `${file}:${line}: ${source.split("\n")[line - 1]?.trim() ?? ""}`;
        found.push(where);
        if (!/^\s*LOCALE\b/.test(source.slice(at))) {
          unnamed.push(where);
        }
      }
    }
    expect(unnamed).toEqual([]);
    // The four in format.ts and formatMoney's.
    expect(found.length).toBeGreaterThanOrEqual(5);
  });
});
