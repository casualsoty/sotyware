//! The notes overlay's data and its rules (ADR-052): notes and their
//! order, the settings and their validation, the shortcuts' actions, what
//! the overlay remembers between sessions. Pure: no window, no database,
//! no clock (times are passed in), so every rule is unit-tested here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::automation::keys::Chord;
use crate::automation::model::Binding;
use crate::window_state::Saved;

/// The format of the settings and state documents; another value is not
/// read (the defaults are used and the stored document is left alone
/// until the user saves).
pub const FORMAT_VERSION: u32 = 1;
/// How many notes there can be.
pub const MAX_NOTES: usize = 500;
/// A note's title, in characters.
pub const MAX_TITLE_CHARS: usize = 120;
/// A note's text, in characters (about 25 printed pages).
pub const MAX_BODY_CHARS: usize = 50_000;
/// The title of a new note.
pub const DEFAULT_TITLE: &str = "New note";
/// The opacity range of the overlay's background, in percent.
pub const OPACITY_RANGE: (u8, u8) = (20, 100);
/// The overlay's size limits, in logical pixels (its inner size).
pub const MIN_SIZE: (u32, u32) = (240, 160);
pub const MAX_SIZE: (u32, u32) = (1600, 1600);
/// The height of the collapsed overlay (its title bar), logical pixels.
pub const COLLAPSED_HEIGHT: u32 = 38;
/// A link a note may open: its length.
pub const MAX_LINK_CHARS: usize = 2048;

// --------------------------------------------------------------- notes

/// One note, as the overlay page edits it. `body` is Markdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    /// Its place in the order (pinned notes first, see [`display_order`]).
    pub position: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
    /// Bumped by every save; a save must name the revision it edited.
    pub revision: u64,
}

impl Note {
    pub fn meta(&self) -> NoteMeta {
        NoteMeta {
            id: self.id.clone(),
            title: self.title.clone(),
            pinned: self.pinned,
            position: self.position,
            revision: self.revision,
        }
    }
}

/// What the service keeps of a note in memory: everything but its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub position: i64,
    pub revision: u64,
}

/// A note's place after a change of order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteOrder {
    pub id: String,
    pub pinned: bool,
    pub position: i64,
}

/// The ids in the order the overlay shows them: pinned notes first, each
/// group by position (then id, so the order is total).
pub fn display_order(notes: &[NoteMeta]) -> Vec<String> {
    let mut sorted: Vec<&NoteMeta> = notes.iter().collect();
    sorted.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(a.position.cmp(&b.position))
            .then(a.id.cmp(&b.id))
    });
    sorted.into_iter().map(|n| n.id.clone()).collect()
}

/// Gives every note its index in `order` as its position. Returns the
/// notes whose position or pin changed (to be stored).
fn renumber(notes: &mut [NoteMeta], order: &[String], pinned: &[String]) -> Vec<NoteOrder> {
    let mut changed = Vec::new();
    for (i, id) in order.iter().enumerate() {
        if let Some(n) = notes.iter_mut().find(|n| &n.id == id) {
            let position = i as i64;
            let pin = pinned.contains(id);
            if n.position != position || n.pinned != pin {
                n.position = position;
                n.pinned = pin;
                changed.push(NoteOrder {
                    id: n.id.clone(),
                    pinned: pin,
                    position,
                });
            }
        }
    }
    changed
}

/// Every note's place, in display order.
pub fn orders(notes: &[NoteMeta]) -> Vec<NoteOrder> {
    display_order(notes)
        .into_iter()
        .filter_map(|id| notes.iter().find(|n| n.id == id))
        .map(|n| NoteOrder {
            id: n.id.clone(),
            pinned: n.pinned,
            position: n.position,
        })
        .collect()
}

/// Moves note `id` to `index` of the display order, inside its group
/// (pinned notes stay before the others). Returns the changed places,
/// or `None` for an unknown id.
pub fn move_note(notes: &mut [NoteMeta], id: &str, index: usize) -> Option<Vec<NoteOrder>> {
    let mut order = display_order(notes);
    let from = order.iter().position(|x| x == id)?;
    let pinned: Vec<String> = notes
        .iter()
        .filter(|n| n.pinned)
        .map(|n| n.id.clone())
        .collect();
    let pinned_count = pinned.len();
    let moved = order.remove(from);
    let (low, high) = if pinned.contains(&moved) {
        (0, pinned_count.saturating_sub(1))
    } else {
        (pinned_count, order.len())
    };
    order.insert(index.clamp(low, high), moved);
    Some(renumber(notes, &order, &pinned))
}

/// Pins (to the end of the pinned notes) or unpins (to the start of the
/// others) note `id`. `None` for an unknown id.
pub fn set_pinned(notes: &mut [NoteMeta], id: &str, pin: bool) -> Option<Vec<NoteOrder>> {
    let mut order = display_order(notes);
    let from = order.iter().position(|x| x == id)?;
    let mut pinned: Vec<String> = notes
        .iter()
        .filter(|n| n.pinned)
        .map(|n| n.id.clone())
        .collect();
    let was = pinned.iter().any(|p| p == id);
    if was == pin {
        return Some(Vec::new());
    }
    let moved = order.remove(from);
    if pin {
        let at = pinned.len();
        pinned.push(moved.clone());
        order.insert(at, moved);
    } else {
        pinned.retain(|p| p != &moved);
        let at = pinned.len();
        order.insert(at, moved);
    }
    Some(renumber(notes, &order, &pinned))
}

/// The position a note added at the end of the list gets.
pub fn end_position(notes: &[NoteMeta]) -> i64 {
    notes.iter().map(|n| n.position + 1).max().unwrap_or(0)
}

/// Makes room for a copy of note `after` right behind it; returns the
/// copy's position and the places that changed.
pub fn insert_after(notes: &mut [NoteMeta], after: &str) -> Option<(i64, bool, Vec<NoteOrder>)> {
    let order = display_order(notes);
    let at = order.iter().position(|x| x == after)?;
    let pinned: Vec<String> = notes
        .iter()
        .filter(|n| n.pinned)
        .map(|n| n.id.clone())
        .collect();
    let pin = pinned.iter().any(|p| p == after);
    // Everything behind the original moves one place down.
    let mut changed = Vec::new();
    for (i, id) in order.iter().enumerate() {
        let want = if i <= at { i as i64 } else { i as i64 + 1 };
        if let Some(n) = notes.iter_mut().find(|n| &n.id == id)
            && n.position != want
        {
            n.position = want;
            changed.push(NoteOrder {
                id: n.id.clone(),
                pinned: n.pinned,
                position: want,
            });
        }
    }
    Some((at as i64 + 1, pin, changed))
}

/// The note `step` places away from `current` in display order, going
/// round at either end; the first note when `current` is unknown.
pub fn step_note(notes: &[NoteMeta], current: Option<&str>, step: i64) -> Option<String> {
    let order = display_order(notes);
    if order.is_empty() {
        return None;
    }
    let len = order.len() as i64;
    let i = match current.and_then(|c| order.iter().position(|x| x == c)) {
        Some(i) => (i as i64 + step).rem_euclid(len),
        None => 0,
    };
    order.get(i as usize).cloned()
}

/// `title`, or `title (2)`, `title (3)`… whichever no note has yet.
pub fn unique_title<'a>(title: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let free = |t: &str| !taken.clone().any(|x| x.eq_ignore_ascii_case(t));
    if free(title) {
        return title.to_owned();
    }
    let base = title.trim_end();
    (2..)
        .map(|i| {
            let suffix = format!(" ({i})");
            let room = MAX_TITLE_CHARS.saturating_sub(suffix.chars().count());
            let cut: String = base.chars().take(room).collect();
            format!("{cut}{suffix}")
        })
        .find(|t| free(t))
        .unwrap_or_else(|| title.to_owned())
}

/// A title as stored: line breaks, tabs and other control characters
/// become spaces. Refused when longer than [`MAX_TITLE_CHARS`].
pub fn clean_title(title: &str) -> Result<String, String> {
    let clean: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if clean.chars().count() > MAX_TITLE_CHARS {
        return Err(format!(
            "A note's title can have at most {MAX_TITLE_CHARS} characters"
        ));
    }
    Ok(clean)
}

/// A note's text as stored: Windows line ends become `\n`. Refused when
/// longer than [`MAX_BODY_CHARS`] or holding a NUL character.
pub fn clean_body(body: &str) -> Result<String, String> {
    if body.contains('\0') {
        return Err("A note cannot contain NUL characters".to_owned());
    }
    let clean = body.replace("\r\n", "\n");
    if clean.chars().count() > MAX_BODY_CHARS {
        return Err(format!(
            "A note can have at most {MAX_BODY_CHARS} characters; split it into two notes"
        ));
    }
    Ok(clean)
}

/// Whether `url` may be opened from note text `body`: an `http` or
/// `https` address without spaces or control characters that the note
/// holds. The page never makes sotyware open an address that is not in
/// the user's own note (ADR-049's rule for addresses from the page).
pub fn link_allowed(body: &str, url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme = lower.starts_with("https://") || lower.starts_with("http://");
    let host = url.split_once("://").map_or("", |(_, rest)| rest);
    scheme
        && !host.is_empty()
        && !host.starts_with('/')
        && url.chars().count() <= MAX_LINK_CHARS
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        && body.contains(url)
}

// ------------------------------------------------------------ settings

/// Whether the overlay takes the mouse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// It takes clicks and the keyboard like any window.
    #[default]
    Interactive,
    /// Clicks go through it to the window below; it never takes the
    /// keyboard.
    Passive,
}

/// Where a new overlay opens on its monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
    /// At the left edge, centred vertically.
    CenterLeft,
    /// At the right edge, centred vertically.
    CenterRight,
    /// Centred on the line a third of the way across from the left, and
    /// vertically.
    MidCenterLeft,
    /// Centred on the line a third of the way across from the right, and
    /// vertically.
    MidCenterRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// What a new note starts with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NewNoteTemplate {
    #[default]
    Blank,
    /// A first, empty checklist item.
    Checklist,
}

impl NewNoteTemplate {
    pub fn body(self) -> &'static str {
        match self {
            Self::Blank => "",
            Self::Checklist => "- [ ] ",
        }
    }
}

/// Which note the overlay shows when it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenNote {
    /// The one shown last.
    #[default]
    Last,
    /// The first pinned note (else the first note).
    FirstPinned,
}

/// What a global shortcut of the overlay does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayAction {
    ToggleVisible,
    ToggleMode,
    NewNote,
    NextNote,
    PreviousNote,
}

impl OverlayAction {
    pub const ALL: [Self; 5] = [
        Self::ToggleVisible,
        Self::ToggleMode,
        Self::NewNote,
        Self::NextNote,
        Self::PreviousNote,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleVisible => "Show or hide the overlay",
            Self::ToggleMode => "Switch between interactive and passive",
            Self::NewNote => "New note",
            Self::NextNote => "Next note",
            Self::PreviousNote => "Previous note",
        }
    }

    /// The suggested chord: Ctrl+Shift with a function key, which games
    /// rarely use (they take plain function keys, Alt with F1–F10 is
    /// NVIDIA's, Shift+Tab Steam's), which never types a character with
    /// AltGr (Ctrl+Alt would) and which the autoclicker's suggestions (F6–F10,
    /// Shift+F11) leave free.
    pub fn suggestion(self) -> &'static str {
        match self {
            Self::ToggleVisible => "Ctrl+Shift+F1",
            Self::ToggleMode => "Ctrl+Shift+F2",
            Self::NewNote => "Ctrl+Shift+F3",
            Self::PreviousNote => "Ctrl+Shift+F4",
            Self::NextNote => "Ctrl+Shift+F5",
        }
    }
}

/// The overlay's settings: one JSON document in `notes.sqlite3`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlaySettings {
    pub format_version: u32,
    /// The background's opacity in percent ([`OPACITY_RANGE`]); the text
    /// stays opaque.
    pub opacity: u8,
    pub text_size: TextSize,
    /// The size a new overlay opens with, logical pixels.
    pub width: u32,
    pub height: u32,
    pub anchor: Anchor,
    /// Opens where it was last (else at `anchor`, at the default size).
    pub remember_placement: bool,
    /// The mode it starts in.
    pub default_mode: Mode,
    /// Shown when sotyware starts (while the feature is on).
    pub show_at_startup: bool,
    pub new_note: NewNoteTemplate,
    pub open_note: OpenNote,
    pub shortcuts: BTreeMap<OverlayAction, Binding>,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            opacity: 88,
            text_size: TextSize::Medium,
            width: 360,
            height: 440,
            anchor: Anchor::TopRight,
            remember_placement: true,
            default_mode: Mode::Interactive,
            show_at_startup: false,
            new_note: NewNoteTemplate::Blank,
            open_note: OpenNote::Last,
            shortcuts: OverlayAction::ALL
                .iter()
                .map(|&a| (a, Binding::on(a.suggestion())))
                .collect(),
        }
    }
}

impl OverlaySettings {
    /// Checks every value and returns the settings with every action
    /// present (a missing one has no shortcut) and a binding without a
    /// chord switched off. Two actions may not share a chord.
    pub fn validated(mut self) -> Result<Self, String> {
        if self.format_version != FORMAT_VERSION {
            return Err(format!(
                "settings of format {} cannot be used by this sotyware",
                self.format_version
            ));
        }
        let (lo, hi) = OPACITY_RANGE;
        if !(lo..=hi).contains(&self.opacity) {
            return Err(format!("The opacity must be from {lo} to {hi} %"));
        }
        if !(MIN_SIZE.0..=MAX_SIZE.0).contains(&self.width)
            || !(MIN_SIZE.1..=MAX_SIZE.1).contains(&self.height)
        {
            return Err(format!(
                "The size must be from {} × {} to {} × {} pixels",
                MIN_SIZE.0, MIN_SIZE.1, MAX_SIZE.0, MAX_SIZE.1
            ));
        }
        for a in OverlayAction::ALL {
            let b = self.shortcuts.entry(a).or_insert(Binding::NONE);
            if b.chord.is_none() {
                b.enabled = false;
            }
        }
        let mut used: Vec<(OverlayAction, Chord)> = Vec::new();
        for (&action, binding) in &self.shortcuts {
            let Some(chord) = binding.active() else {
                continue;
            };
            if let Some(p) = chord.problem() {
                return Err(format!("{}: {p}", action.label()));
            }
            if let Some((other, _)) = used.iter().find(|(_, c)| *c == chord) {
                return Err(format!(
                    "\"{}\" and \"{}\" have the same shortcut",
                    other.label(),
                    action.label()
                ));
            }
            used.push((action, chord));
        }
        Ok(self)
    }

    /// The shortcuts in use, in the order of [`OverlayAction::ALL`].
    pub fn active_shortcuts(&self) -> Vec<(OverlayAction, Chord)> {
        OverlayAction::ALL
            .iter()
            .filter_map(|a| {
                self.shortcuts
                    .get(a)
                    .and_then(|b| b.active())
                    .map(|c| (*a, c))
            })
            .collect()
    }
}

// --------------------------------------------------------------- state

/// What the overlay remembers between sessions besides its settings: one
/// JSON document in `notes.sqlite3`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayState {
    pub format_version: u32,
    pub collapsed: bool,
    /// The note shown last.
    pub active_note: Option<String>,
    /// Where the overlay was, expanded (its height is the expanded one
    /// while it is collapsed).
    pub placement: Option<Saved>,
}

impl OverlayState {
    pub fn new() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            ..Self::default()
        }
    }

    /// Reads a stored document; `None` for one this version cannot use.
    pub fn parse(text: &str) -> Option<Self> {
        let s: Self = serde_json::from_str(text).ok()?;
        (s.format_version == FORMAT_VERSION).then_some(s)
    }
}

/// The note the overlay opens with: the last one shown (if it still
/// exists) or, as `open` says, the first pinned one, else the first.
pub fn opening_note(notes: &[NoteMeta], last: Option<&str>, open: OpenNote) -> Option<String> {
    let order = display_order(notes);
    let exists = |id: &str| order.iter().any(|x| x == id);
    match open {
        OpenNote::Last => last
            .filter(|id| exists(id))
            .map(str::to_owned)
            .or_else(|| order.first().cloned()),
        // Pinned notes come first in the display order.
        OpenNote::FirstPinned => order.first().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, pinned: bool, position: i64) -> NoteMeta {
        NoteMeta {
            id: id.into(),
            title: id.to_uppercase(),
            pinned,
            position,
            revision: 1,
        }
    }

    fn sample() -> Vec<NoteMeta> {
        vec![
            meta("c", false, 2),
            meta("a", false, 0),
            meta("p", true, 7),
            meta("b", false, 1),
            meta("q", true, 3),
        ]
    }

    fn ids(order: &[NoteOrder]) -> Vec<&str> {
        order.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn pinned_notes_come_first_each_group_by_position() {
        assert_eq!(display_order(&sample()), ["q", "p", "a", "b", "c"]);
        assert!(display_order(&[]).is_empty());
        // Equal positions (an older database) still give one order.
        let tie = vec![meta("y", false, 0), meta("x", false, 0)];
        assert_eq!(display_order(&tie), ["x", "y"]);
    }

    #[test]
    fn moving_a_note_renumbers_and_stays_inside_its_group() {
        let mut notes = sample();
        let changed = move_note(&mut notes, "c", 2).unwrap();
        assert_eq!(display_order(&notes), ["q", "p", "c", "a", "b"]);
        // Positions are the display index now; only changes come back
        // ("c" was at 2 already).
        assert_eq!(ids(&changed), ["q", "p", "a", "b"]);
        let changed = move_note(&mut notes, "b", 3).unwrap();
        assert_eq!(display_order(&notes), ["q", "p", "c", "b", "a"]);
        assert_eq!(ids(&changed), ["b", "a"]);
        // An unpinned note cannot go before the pinned ones…
        move_note(&mut notes, "a", 0).unwrap();
        assert_eq!(display_order(&notes), ["q", "p", "a", "c", "b"]);
        // …nor a pinned one after them.
        move_note(&mut notes, "q", 99).unwrap();
        assert_eq!(display_order(&notes), ["p", "q", "a", "c", "b"]);
        // Beyond the end means the end.
        move_note(&mut notes, "a", 99).unwrap();
        assert_eq!(display_order(&notes), ["p", "q", "c", "b", "a"]);
        assert_eq!(move_note(&mut notes, "nope", 0), None);
        // Nothing to change: nothing to store.
        assert_eq!(move_note(&mut notes, "a", 4).unwrap(), Vec::new());
        let positions: Vec<i64> = display_order(&notes)
            .iter()
            .map(|id| notes.iter().find(|n| &n.id == id).unwrap().position)
            .collect();
        assert_eq!(positions, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn pinning_goes_to_the_end_of_the_pinned_and_unpinning_to_the_front_of_the_rest() {
        let mut notes = sample();
        let changed = set_pinned(&mut notes, "b", true).unwrap();
        assert_eq!(display_order(&notes), ["q", "p", "b", "a", "c"]);
        assert!(changed.iter().any(|o| o.id == "b" && o.pinned));
        set_pinned(&mut notes, "q", false).unwrap();
        assert_eq!(display_order(&notes), ["p", "b", "q", "a", "c"]);
        assert!(!notes.iter().find(|n| n.id == "q").unwrap().pinned);
        // Already so: nothing changes.
        assert_eq!(set_pinned(&mut notes, "p", true).unwrap(), Vec::new());
        assert_eq!(set_pinned(&mut notes, "nope", true), None);
        let o = orders(&notes);
        assert_eq!(ids(&o), ["p", "b", "q", "a", "c"]);
        assert_eq!(
            o.iter().map(|x| x.pinned).collect::<Vec<_>>(),
            [true, true, false, false, false]
        );
    }

    #[test]
    fn a_copy_goes_right_behind_its_original_in_its_group() {
        let mut notes = sample();
        let (position, pinned, changed) = insert_after(&mut notes, "q").unwrap();
        assert_eq!((position, pinned), (1, true));
        notes.push(meta("q2", true, position));
        assert_eq!(display_order(&notes), ["q", "q2", "p", "a", "b", "c"]);
        assert!(ids(&changed).contains(&"p"));
        let (position, pinned, _) = insert_after(&mut notes, "a").unwrap();
        assert_eq!((position, pinned), (4, false));
        notes.push(meta("a2", false, position));
        assert_eq!(display_order(&notes), ["q", "q2", "p", "a", "a2", "b", "c"]);
        assert_eq!(insert_after(&mut notes, "nope"), None);
        assert_eq!(end_position(&notes), 7);
        assert_eq!(end_position(&[]), 0);
    }

    #[test]
    fn stepping_goes_round_in_display_order() {
        let notes = sample();
        assert_eq!(step_note(&notes, Some("q"), 1).as_deref(), Some("p"));
        assert_eq!(step_note(&notes, Some("c"), 1).as_deref(), Some("q"));
        assert_eq!(step_note(&notes, Some("q"), -1).as_deref(), Some("c"));
        assert_eq!(step_note(&notes, Some("gone"), 1).as_deref(), Some("q"));
        assert_eq!(step_note(&notes, None, -1).as_deref(), Some("q"));
        assert_eq!(step_note(&[], None, 1), None);
        let one = vec![meta("x", false, 0)];
        assert_eq!(step_note(&one, Some("x"), 1).as_deref(), Some("x"));
    }

    #[test]
    fn titles_are_made_unique_and_kept_short() {
        let taken = ["New note", "New note (2)", "Boss"];
        assert_eq!(
            unique_title("New note", taken.iter().copied()),
            "New note (3)"
        );
        assert_eq!(
            unique_title("new NOTE", taken.iter().copied()),
            "new NOTE (3)"
        );
        assert_eq!(unique_title("Route", taken.iter().copied()), "Route");
        let long = "x".repeat(MAX_TITLE_CHARS);
        let taken = [long.as_str()];
        let u = unique_title(&long, taken.iter().copied());
        assert_eq!(u.chars().count(), MAX_TITLE_CHARS);
        assert!(u.ends_with(" (2)"));
    }

    #[test]
    fn titles_and_text_are_cleaned_and_bounded() {
        assert_eq!(clean_title("Boss\tstrategy\n").unwrap(), "Boss strategy ");
        assert_eq!(clean_title("").unwrap(), "");
        assert_eq!(clean_title("Ölmühle ✓").unwrap(), "Ölmühle ✓");
        assert!(clean_title(&"é".repeat(MAX_TITLE_CHARS)).is_ok());
        assert!(clean_title(&"é".repeat(MAX_TITLE_CHARS + 1)).is_err());
        assert_eq!(clean_body("a\r\nb\n").unwrap(), "a\nb\n");
        assert!(clean_body("nul\0").is_err());
        assert!(clean_body(&"ü".repeat(MAX_BODY_CHARS)).is_ok());
        assert!(clean_body(&"ü".repeat(MAX_BODY_CHARS + 1)).is_err());
    }

    #[test]
    fn only_web_links_the_note_holds_may_be_opened() {
        let body = "Guide: [map](https://example.com/map?a=1) and http://x.org/b";
        assert!(link_allowed(body, "https://example.com/map?a=1"));
        assert!(link_allowed(body, "http://x.org/b"));
        assert!(
            !link_allowed(body, "HTTPS://EXAMPLE.COM/MAP?A=1"),
            "only as written"
        );
        assert!(
            !link_allowed(body, "https://evil.example/"),
            "not in the note"
        );
        assert!(!link_allowed("file:///etc/passwd", "file:///etc/passwd"));
        assert!(!link_allowed("javascript:alert(1)", "javascript:alert(1)"));
        assert!(!link_allowed("https:///x", "https:///x"));
        assert!(!link_allowed("https://", "https://"));
        assert!(!link_allowed("https://a b", "https://a b"));
        let long = format!("https://a.example/{}", "x".repeat(MAX_LINK_CHARS));
        assert!(!link_allowed(&long, &long));
    }

    #[test]
    fn default_settings_are_valid_and_use_the_suggested_shortcuts() {
        let s = OverlaySettings::default().validated().unwrap();
        assert_eq!(s.opacity, 88);
        assert_eq!(s.default_mode, Mode::Interactive);
        assert!(!s.show_at_startup);
        assert!(s.remember_placement);
        let active: Vec<(OverlayAction, String)> = s
            .active_shortcuts()
            .into_iter()
            .map(|(a, c)| (a, c.to_string()))
            .collect();
        assert_eq!(
            active,
            [
                (OverlayAction::ToggleVisible, "Ctrl+Shift+F1".to_owned()),
                (OverlayAction::ToggleMode, "Ctrl+Shift+F2".to_owned()),
                (OverlayAction::NewNote, "Ctrl+Shift+F3".to_owned()),
                (OverlayAction::NextNote, "Ctrl+Shift+F5".to_owned()),
                (OverlayAction::PreviousNote, "Ctrl+Shift+F4".to_owned()),
            ]
        );
        // None is a chord the autoclicker suggests (ADR-050).
        for (_, c) in s.active_shortcuts() {
            for taken in ["F6", "F7", "F8", "F9", "F10", "Shift+F11"] {
                assert_ne!(c, Chord::parse(taken).unwrap());
            }
            assert_eq!(c.problem(), None);
        }
    }

    #[test]
    fn settings_out_of_range_or_with_clashing_shortcuts_are_refused() {
        let ok = OverlaySettings::default();
        let bad = |f: &dyn Fn(&mut OverlaySettings)| {
            let mut s = ok.clone();
            f(&mut s);
            s.validated().unwrap_err()
        };
        assert!(bad(&|s| s.opacity = 19).contains("opacity"));
        assert!(bad(&|s| s.opacity = 101).contains("opacity"));
        assert!(bad(&|s| s.width = 239).contains("size"));
        assert!(bad(&|s| s.height = 1601).contains("size"));
        assert!(bad(&|s| s.format_version = 2).contains("format"));
        let clash = bad(&|s| {
            s.shortcuts
                .insert(OverlayAction::NewNote, Binding::on("Ctrl+Shift+F1"));
        });
        assert!(clash.contains("same shortcut"), "{clash}");
        let problem = bad(&|s| {
            s.shortcuts
                .insert(OverlayAction::NewNote, Binding::on("KeyN"));
        });
        assert!(problem.contains("New note"), "{problem}");
        // The same chord twice is fine when one of them is off.
        let mut s = ok.clone();
        s.shortcuts.insert(
            OverlayAction::NewNote,
            Binding {
                chord: Chord::parse("Ctrl+Shift+F1").ok(),
                enabled: false,
            },
        );
        assert!(s.validated().is_ok());
    }

    #[test]
    fn missing_actions_get_no_shortcut_and_a_binding_without_chord_is_off() {
        let mut s = OverlaySettings::default();
        s.shortcuts.remove(&OverlayAction::NextNote);
        s.shortcuts.insert(
            OverlayAction::NewNote,
            Binding {
                chord: None,
                enabled: true,
            },
        );
        let v = s.validated().unwrap();
        assert_eq!(v.shortcuts[&OverlayAction::NextNote], Binding::NONE);
        assert!(!v.shortcuts[&OverlayAction::NewNote].enabled);
        assert_eq!(v.active_shortcuts().len(), 3);
    }

    #[test]
    fn the_page_receives_the_settings_in_this_shape() {
        let json = serde_json::to_value(OverlaySettings::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "formatVersion": 1,
                "opacity": 88,
                "textSize": "medium",
                "width": 360,
                "height": 440,
                "anchor": "top-right",
                "rememberPlacement": true,
                "defaultMode": "interactive",
                "showAtStartup": false,
                "newNote": "blank",
                "openNote": "last",
                "shortcuts": {
                    "toggle-visible": { "chord": "Ctrl+Shift+F1", "enabled": true },
                    "toggle-mode": { "chord": "Ctrl+Shift+F2", "enabled": true },
                    "new-note": { "chord": "Ctrl+Shift+F3", "enabled": true },
                    "next-note": { "chord": "Ctrl+Shift+F5", "enabled": true },
                    "previous-note": { "chord": "Ctrl+Shift+F4", "enabled": true },
                },
            })
        );
        // Unknown fields (a newer sotyware's) are refused, not dropped.
        let mut extra = json.clone();
        extra["glow"] = serde_json::json!(true);
        assert!(serde_json::from_value::<OverlaySettings>(extra).is_err());
        let back: OverlaySettings = serde_json::from_value(json).unwrap();
        assert_eq!(back, OverlaySettings::default());
    }

    #[test]
    fn the_places_have_these_names_on_the_page() {
        // `ANCHORS` in src/notesOverlay.ts lists the same names.
        let all = [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomLeft,
            Anchor::BottomRight,
            Anchor::Center,
            Anchor::CenterLeft,
            Anchor::CenterRight,
            Anchor::MidCenterLeft,
            Anchor::MidCenterRight,
        ];
        let names = serde_json::json!([
            "top-left",
            "top-right",
            "bottom-left",
            "bottom-right",
            "center",
            "center-left",
            "center-right",
            "mid-center-left",
            "mid-center-right",
        ]);
        assert_eq!(serde_json::to_value(all).unwrap(), names);
        assert_eq!(serde_json::from_value::<[Anchor; 9]>(names).unwrap(), all);
    }

    #[test]
    fn a_note_is_sent_in_this_shape() {
        let n = Note {
            id: "note-1".into(),
            title: "Boss".into(),
            body: "- [ ] heal".into(),
            pinned: true,
            position: 0,
            created_ms: 5,
            updated_ms: 6,
            revision: 2,
        };
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            serde_json::json!({
                "id": "note-1", "title": "Boss", "body": "- [ ] heal", "pinned": true,
                "position": 0, "createdMs": 5, "updatedMs": 6, "revision": 2,
            })
        );
        assert_eq!(n.meta().revision, 2);
    }

    #[test]
    fn the_state_is_parsed_strictly() {
        let s = OverlayState {
            collapsed: true,
            active_note: Some("note-1".into()),
            placement: Some(Saved::new(
                crate::window_state::Rect {
                    x: -100,
                    y: 20,
                    width: 400,
                    height: 500,
                },
                false,
                1.5,
            )),
            ..OverlayState::new()
        };
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(OverlayState::parse(&text), Some(s));
        assert_eq!(OverlayState::parse("{}"), None);
        assert_eq!(OverlayState::parse("not json"), None);
        assert_eq!(
            OverlayState::parse(
                r#"{"formatVersion":2,"collapsed":false,"activeNote":null,"placement":null}"#
            ),
            None
        );
    }

    #[test]
    fn the_opening_note_follows_the_setting() {
        let notes = sample();
        assert_eq!(
            opening_note(&notes, Some("b"), OpenNote::Last).as_deref(),
            Some("b")
        );
        assert_eq!(
            opening_note(&notes, Some("gone"), OpenNote::Last).as_deref(),
            Some("q")
        );
        assert_eq!(
            opening_note(&notes, Some("b"), OpenNote::FirstPinned).as_deref(),
            Some("q")
        );
        assert_eq!(opening_note(&[], Some("b"), OpenNote::Last), None);
        assert_eq!(NewNoteTemplate::Checklist.body(), "- [ ] ");
        assert_eq!(NewNoteTemplate::Blank.body(), "");
    }
}
