// The notes overlay's custom emojis (ADR-052): small pictures a note
// holds as a shortcode such as `:guardian:`, shown one line high where
// the note is read and picked from a menu while it is edited. The
// pictures are files bundled with the page (`src/assets/emoji/gw2/`), so
// a note never loads anything from elsewhere (CSP `img-src 'self' data:`).
//
// The set is Guild Wars 2's nine professions and their elite
// specializations in the wiki's "tango" style
// (https://wiki.guildwars2.com/wiki/Guild_Wars_2_Wiki:Profession_icons,
// GFDL; src/assets/emoji/gw2/README.md), one 48 px file per icon named
// after its shortcode. Only the icons whose file is present are offered
// and read; any other `:word:` stays text.

export interface Emoji {
  /** The shortcode without its colons: lowercase letters. */
  id: string;
  /** What a screen reader says and the menu's tooltip shows. */
  name: string;
  /** The picture's address in the page. */
  src: string;
}

/** Each profession with its elite specializations in release order
 * (Heart of Thorns, Path of Fire, End of Dragons, Visions of Eternity):
 * the menu's rows. */
export const PROFESSIONS: readonly (readonly string[])[] = [
  ["Guardian", "Dragonhunter", "Firebrand", "Willbender", "Luminary"],
  ["Revenant", "Herald", "Renegade", "Vindicator", "Conduit"],
  ["Warrior", "Berserker", "Spellbreaker", "Bladesworn", "Paragon"],
  ["Engineer", "Scrapper", "Holosmith", "Mechanist", "Amalgam"],
  ["Ranger", "Druid", "Soulbeast", "Untamed", "Galeshot"],
  ["Thief", "Daredevil", "Deadeye", "Specter", "Antiquary"],
  ["Elementalist", "Tempest", "Weaver", "Catalyst", "Evoker"],
  ["Mesmer", "Chronomancer", "Mirage", "Virtuoso", "Troubadour"],
  ["Necromancer", "Reaper", "Scourge", "Harbinger", "Ritualist"],
];

/** The emojis whose picture is among `files` (path → address), in the
 * rows of `PROFESSIONS`; a row without any picture is left out. */
export function emojiSet(files: Readonly<Record<string, string>>): Emoji[][] {
  const byId = new Map<string, string>();
  for (const [path, src] of Object.entries(files)) {
    const m = /([a-z]+)\.(?:png|webp|svg)$/.exec(path);
    if (m?.[1] !== undefined) {
      byId.set(m[1], src);
    }
  }
  return PROFESSIONS.map((row) =>
    row.flatMap((name) => {
      const id = name.toLowerCase();
      const src = byId.get(id);
      return src === undefined ? [] : [{ id, name, src }];
    }),
  ).filter((row) => row.length > 0);
}

/** The emojis bundled with this build, by profession. */
export const EMOJI_ROWS: readonly (readonly Emoji[])[] = emojiSet(
  import.meta.glob<string>("./assets/emoji/gw2/*.{png,webp,svg}", { eager: true, query: "?url", import: "default" }),
);

/** The bundled emojis by shortcode. */
export const EMOJI_BY_ID: ReadonlyMap<string, Emoji> = new Map(EMOJI_ROWS.flat().map((e) => [e.id, e]));

/** What the menu inserts at the caret: the shortcode, with a space before
 * it unless the caret starts a word already, so it is read as one. */
export function emojiInsert(value: string, start: number, id: string): string {
  const before = value[start - 1];
  const pad = before !== undefined && /[\p{L}\p{N}:]/u.test(before) ? " " : "";
  return `${pad}:${id}:`;
}
