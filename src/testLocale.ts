// Run by Vitest before every test file (`setupFiles` in vite.config.ts): en-US
// becomes the default locale of every Intl service and of the built-in methods
// that format, compare or change case by locale, whatever the machine's. The
// tests run the page in Node, whose Intl takes its default locale from the
// machine (on Windows the regional format, which LANG and LC_ALL do not change;
// on Linux LC_ALL or LANG), while the webview they simulate formats in its own
// language: happy-dom's `navigator.language` is "en-US", and WebView2 on a
// Windows 11 whose regional format is fr-FR formats in its display language,
// en-US. Without this, six tests expecting "1,234" got "1 234" on that machine
// (2026-11-23).
//
// Each call gets the locales it names followed by en-US: ECMA-402 falls back to
// it when none of them is available, as it would to a default locale, and a
// locale a call names is still honoured. One difference: a service whose data
// is per language reports that language (Intl.PluralRules: "en", with the same
// rules), where Node reports a default locale as it is ("en-US"). Checked by
// testLocale.test.ts.
//
// A test can stand in for a webview in another language with
// `withDefaultLocale` (since 2026-11-24): the page writes every number and date
// in en-US whatever the webview's language (src/format.ts), which only a
// default other than en-US can show (format.test.ts).

/** The default locale of every frontend test: the simulated webview's language. */
const TEST_LOCALE = "en-US";

/** The default locale now: the test locale, but inside `withDefaultLocale`. */
let defaultLocale: string = TEST_LOCALE;

/** Runs `fn` with `locale` as the default locale instead of en-US, as if the
 * webview's language were `locale` (`navigator.language` is left alone: the
 * page does not read it), then puts en-US back, also when `fn` fails. A locale
 * a call names is still honoured. `locale` must be one Node has data for, or
 * Node's own default (the machine's) would be used instead. */
export async function withDefaultLocale<T>(locale: string, fn: () => T | Promise<T>): Promise<T> {
  const [canonical] = Intl.getCanonicalLocales(locale);
  if (canonical === undefined || Intl.NumberFormat.supportedLocalesOf(canonical).length === 0) {
    throw new Error(`no locale data for ${locale}`);
  }
  const was = defaultLocale;
  defaultLocale = canonical;
  try {
    return await fn();
  } finally {
    defaultLocale = was;
  }
}

/** The Intl services whose first argument is a list of locales; a newer one
 * fails testLocale.test.ts until it is added here and there. */
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
] as const;

/** The built-in methods that take a list of locales, with its position. */
const PINNED_METHODS: readonly (readonly [object, string, number])[] = [
  [Number.prototype, "toLocaleString", 0],
  [BigInt.prototype, "toLocaleString", 0],
  [Date.prototype, "toLocaleString", 0],
  [Date.prototype, "toLocaleDateString", 0],
  [Date.prototype, "toLocaleTimeString", 0],
  [String.prototype, "localeCompare", 1],
  [String.prototype, "toLocaleLowerCase", 0],
  [String.prototype, "toLocaleUpperCase", 0],
];

/** `args` with the locales at `at` followed by the default locale. An invalid
 * locale throws here as it would in the call itself. */
function withTestLocale(args: readonly unknown[], at: number): unknown[] {
  const out = Array.from({ length: Math.max(args.length, at + 1) }, (_, i) => args[i]);
  out[at] = [...Intl.getCanonicalLocales(args[at] as string | readonly string[] | undefined), defaultLocale];
  return out;
}

for (const name of PINNED_SERVICES) {
  const service: unknown = Reflect.get(Intl, name);
  // Absent from older Node versions (DurationFormat before 23): nothing can
  // format with it then.
  if (typeof service !== "function") {
    continue;
  }
  const pinned = new Proxy(service, {
    construct: (target, args, newTarget): object => Reflect.construct(target, withTestLocale(args, 0), newTarget) as object,
    apply: (target, self, args): unknown => Reflect.apply(target, self, withTestLocale(args, 0)),
  });
  if (!Reflect.set(Intl, name, pinned)) {
    throw new Error(`cannot pin the default locale of Intl.${name}`);
  }
}

for (const [prototype, name, at] of PINNED_METHODS) {
  const method: unknown = Reflect.get(prototype, name);
  if (typeof method !== "function") {
    throw new Error(`no built-in ${name} to pin the default locale of`);
  }
  const pinned = function (this: unknown, ...args: unknown[]): unknown {
    return Reflect.apply(method, this, withTestLocale(args, at));
  };
  Object.defineProperties(pinned, { name: { value: name }, length: { value: method.length } });
  if (!Reflect.defineProperty(prototype, name, { value: pinned })) {
    throw new Error(`cannot pin the default locale of ${name}`);
  }
}
