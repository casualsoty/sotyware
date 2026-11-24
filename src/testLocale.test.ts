// The frontend tests' default locale, set before every test file by
// testLocale.ts (`setupFiles` in vite.config.ts; not imported here, so these
// tests also show that it ran): en-US on any machine, the simulated webview's
// language, while every locale a call names is still honoured. Without it three
// of them fail where the machine's locale is not en-US (`LC_ALL=fr_FR.UTF-8` on
// Linux, the regional format on Windows), and the one of the services also
// where it is: Intl.PluralRules reports a default locale as it is ("en-US") but
// resolves en-US named to its language ("en").
import { describe, expect, it } from "vitest";

const TEST_LOCALE = "en-US";

/** The Intl services testLocale.ts pins: every one Node 24 has. */
const PINNED_SERVICES = [
  "Collator",
  "DateTimeFormat",
  "DisplayNames",
  "DurationFormat",
  "ListFormat",
  "NumberFormat",
  "PluralRules",
  "RelativeTimeFormat",
  "Segmenter",
];

type Service = new (locales?: unknown, options?: object) => { resolvedOptions(): { locale: string } };

/** What a service needs besides its locales. */
const OPTIONS: Record<string, object> = { DisplayNames: { type: "region" } };

/** Every Intl service this Node has, by name (Intl.Locale takes one tag, not a
 * list of locales, and has no default). */
function services(): string[] {
  return Object.getOwnPropertyNames(Intl)
    .filter((name) => /^[A-Z]/.test(name) && name !== "Locale")
    .sort();
}

function create(name: string, locales: unknown): { resolvedOptions(): { locale: string } } {
  const S = Reflect.get(Intl, name) as Service;
  return new S(locales, OPTIONS[name]);
}

describe("the frontend tests' default locale", () => {
  it("is the simulated webview's language", () => {
    expect(TEST_LOCALE).toBe("en-US");
    expect(navigator.language).toBe(TEST_LOCALE);
  });

  it("is the default of every Intl service, also when no locale named is available", () => {
    expect(services()).toEqual(PINNED_SERVICES.filter((name) => name in Intl));
    for (const name of services()) {
      // en-US named, or its language where the service's data is per
      // language (Intl.PluralRules: "en").
      const named = create(name, TEST_LOCALE).resolvedOptions().locale;
      expect([TEST_LOCALE, "en"], name).toContain(named);
      expect(create(name, undefined).resolvedOptions().locale, name).toBe(named);
      expect(create(name, []).resolvedOptions().locale, name).toBe(named);
      // "zxx" (no linguistic content) has no locale data: the default is used.
      expect(create(name, "zxx").resolvedOptions().locale, name).toBe(named);
    }
    expect(Intl.DateTimeFormat().resolvedOptions().locale).toBe(TEST_LOCALE);
  });

  it("is the default of the built-in methods that format by locale", () => {
    expect((1234).toLocaleString()).toBe("1,234");
    expect((1234n).toLocaleString()).toBe("1,234");
    expect((1.39).toLocaleString(undefined, { style: "currency", currency: "USD" })).toBe("$1.39");
    const when = new Date(Date.UTC(2026, 10, 23, 18, 5));
    expect(when.toLocaleDateString(undefined, { timeZone: "UTC" })).toBe("11/23/2026");
    expect(when.toLocaleDateString([], { timeZone: "UTC", month: "long" })).toBe("November");
    expect(when.toLocaleString(undefined, { timeZone: "UTC" })).toBe(when.toLocaleString("en-US", { timeZone: "UTC" }));
    expect(when.toLocaleTimeString([], { timeZone: "UTC", hour: "2-digit", minute: "2-digit" })).toBe(
      when.toLocaleTimeString("en-US", { timeZone: "UTC", hour: "2-digit", minute: "2-digit" }),
    );
    expect("ä".localeCompare("z")).toBeLessThan(0);
    expect("i".toLocaleUpperCase()).toBe("I");
    expect("I".toLocaleLowerCase()).toBe("i");
  });

  it("leaves a locale a call names in place", () => {
    expect((1234).toLocaleString("de-DE")).toBe("1.234");
    expect((1234n).toLocaleString("de-DE")).toBe("1.234");
    expect(new Intl.NumberFormat("fr-FR").resolvedOptions().locale).toBe("fr-FR");
    expect(Intl.NumberFormat(["zxx", "de-CH"]).resolvedOptions().locale).toBe("de-CH");
    expect(new Date(0).toLocaleDateString("de-DE", { timeZone: "UTC" })).toBe("1.1.1970");
    // Swedish sorts "ä" after "z"; the locale is localeCompare's second argument.
    expect("ä".localeCompare("z", "sv")).toBeGreaterThan(0);
    expect("i".toLocaleUpperCase("tr")).toBe("İ");
    expect("I".toLocaleLowerCase("tr")).toBe("ı");
  });

  it("keeps the services and methods otherwise as they were", () => {
    expect(new Intl.NumberFormat()).toBeInstanceOf(Intl.NumberFormat);
    class Grouped extends Intl.NumberFormat {}
    const grouped = new Grouped();
    expect(grouped).toBeInstanceOf(Grouped);
    expect(grouped.format(1234)).toBe("1,234");
    expect(Intl.NumberFormat.supportedLocalesOf(["fr-FR", "zxx"])).toEqual(["fr-FR"]);
    // Services added after ES5 still need `new`.
    expect(() => (Intl.PluralRules as unknown as () => unknown)()).toThrow(TypeError);
    // Invalid locales are refused as before.
    expect(() => new Intl.NumberFormat("en_US")).toThrow(RangeError);
    expect(() => (1).toLocaleString("en_US")).toThrow(RangeError);
    expect(() => (1).toLocaleString(null as unknown as string)).toThrow(TypeError);
    expect(Number.prototype.toLocaleString.name).toBe("toLocaleString");
    expect(String.prototype.localeCompare.length).toBe(1);
    expect(Object.keys(Number.prototype)).toEqual([]);
  });
});
