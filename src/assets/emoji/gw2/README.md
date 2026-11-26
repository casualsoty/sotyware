# Guild Wars 2 profession icons (tango style)

The notes overlay's emojis (`src/noteEmoji.ts`, ADR-053): the nine Guild
Wars 2 professions and their 36 elite specializations, one 48 × 48 PNG per
icon, named after its shortcode (`guardian.png` is `:guardian:`).

- **Source:** the Guild Wars 2 Wiki,
  <https://wiki.guildwars2.com/wiki/Guild_Wars_2_Wiki:Profession_icons>,
  section "Tango icons", the files `<Name> tango icon 48px.png`
  (for example
  <https://wiki.guildwars2.com/wiki/File:Guardian_tango_icon_48px.png>),
  downloaded on 2026-11-26 and copied unchanged, only renamed.
- **Licence:** every one of these files is tagged `{{GFDL image}}` on its
  wiki page: the GNU Free Documentation License, version 1.2 (the
  wiki's licence). Each file's page holds its history and authors. The
  licence's text ships with sotyware: `licenses/GFDL-1.2.txt` next to
  this notice, which is installed as `licenses/gw2-profession-icons.md`
  (`bundle.resources` in `src-tauri/tauri.conf.json`; in the repository,
  `src-tauri/licenses/GFDL-1.2.txt`).
- **Authors named on the file pages:** the wiki's contributors; the
  Visions of Eternity icons (Amalgam, Antiquary, Conduit, Evoker, Galeshot,
  Luminary, Paragon, Ritualist, Troubadour) were drawn by the wiki user
  Tery3065; the Revenant icon was built from Chieftain Alex's
  "Revenant icon option E" drawing; Scrapper and Tempest were scaled from
  their 200 px versions.
- **Trademarks:** Guild Wars 2, its professions and their symbols are
  ArenaNet's. sotyware is not affiliated with or endorsed by ArenaNet or
  NCSOFT.

Replacing or adding an icon: keep the file name the lowercase name of the
profession or specialization listed in `PROFESSIONS`, and update this
file.
