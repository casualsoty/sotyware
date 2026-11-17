//! Keys and shortcuts of the autoclicker (ADR-050).
//!
//! A key is named by its **physical position**, with the W3C UI Events
//! `code` names (`KeyA`, `Digit1`, `F6`, `ShiftLeft`, `ArrowUp`): the name a
//! browser reports as `KeyboardEvent.code`, independent of the keyboard
//! layout. One table maps each name to what the native layers need:
//! Windows' set-1 scan code (with the `0xE0` prefix of extended keys) and
//! its virtual-key code, and the X11 keycode (evdev + 8). A recorded macro
//! therefore replays the same physical keys on another layout, as the
//! hardware would.
//!
//! Keys that could switch the computer off or put it to sleep (Power, Sleep,
//! Wake) are not in the table: they are never recorded and can never be
//! replayed.
//!
//! [`Chord`] is a shortcut: modifiers plus one key, written
//! `Ctrl+Alt+Shift+Meta+<code>` (modifiers in that order).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What kind of key a table entry is; decides which shortcuts are allowed
/// and how Windows replays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// Ctrl, Alt, Shift or Meta (Windows key, ⌘, Super), left or right.
    Modifier(Modifier),
    /// A letter, digit or punctuation key: what it types depends on the
    /// layout, so Windows asks the layout for its virtual-key code.
    Layout,
    /// F1–F24.
    Function,
    /// Media and browser keys.
    Media,
    /// Pause and Scroll Lock: rarely typed, fine as shortcuts without a
    /// modifier.
    Quiet,
    /// Everything else (Enter, arrows, the numeric keypad, locks…).
    Other,
}

/// A modifier of a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

/// One row of the key table.
#[derive(Debug)]
pub struct KeyInfo {
    /// The W3C `code` name.
    pub code: &'static str,
    /// An English name for the UI when the platform cannot name the key.
    pub label: &'static str,
    /// Windows set-1 scan code; extended keys carry `0xE000`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub win_scan: u16,
    /// Windows virtual-key code (the US layout's for layout keys).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub win_vk: u8,
    /// X11 keycode (evdev code + 8).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub x11: u8,
    pub class: KeyClass,
}

macro_rules! keys {
    ($($code:literal, $label:literal, $scan:literal, $vk:literal, $x11:literal, $class:expr;)*) => {
        /// Every key sotyware records and replays.
        pub static KEYS: &[KeyInfo] = &[
            $(KeyInfo { code: $code, label: $label, win_scan: $scan, win_vk: $vk, x11: $x11, class: $class },)*
        ];
    };
}

use KeyClass::{Function, Layout, Media, Other, Quiet};
const CTRL: KeyClass = KeyClass::Modifier(Modifier::Ctrl);
const ALT: KeyClass = KeyClass::Modifier(Modifier::Alt);
const SHIFT: KeyClass = KeyClass::Modifier(Modifier::Shift);
const META: KeyClass = KeyClass::Modifier(Modifier::Meta);

keys! {
    "KeyA", "A", 0x1E, 0x41, 38, Layout;
    "KeyB", "B", 0x30, 0x42, 56, Layout;
    "KeyC", "C", 0x2E, 0x43, 54, Layout;
    "KeyD", "D", 0x20, 0x44, 40, Layout;
    "KeyE", "E", 0x12, 0x45, 26, Layout;
    "KeyF", "F", 0x21, 0x46, 41, Layout;
    "KeyG", "G", 0x22, 0x47, 42, Layout;
    "KeyH", "H", 0x23, 0x48, 43, Layout;
    "KeyI", "I", 0x17, 0x49, 31, Layout;
    "KeyJ", "J", 0x24, 0x4A, 44, Layout;
    "KeyK", "K", 0x25, 0x4B, 45, Layout;
    "KeyL", "L", 0x26, 0x4C, 46, Layout;
    "KeyM", "M", 0x32, 0x4D, 58, Layout;
    "KeyN", "N", 0x31, 0x4E, 57, Layout;
    "KeyO", "O", 0x18, 0x4F, 32, Layout;
    "KeyP", "P", 0x19, 0x50, 33, Layout;
    "KeyQ", "Q", 0x10, 0x51, 24, Layout;
    "KeyR", "R", 0x13, 0x52, 27, Layout;
    "KeyS", "S", 0x1F, 0x53, 39, Layout;
    "KeyT", "T", 0x14, 0x54, 28, Layout;
    "KeyU", "U", 0x16, 0x55, 30, Layout;
    "KeyV", "V", 0x2F, 0x56, 55, Layout;
    "KeyW", "W", 0x11, 0x57, 25, Layout;
    "KeyX", "X", 0x2D, 0x58, 53, Layout;
    "KeyY", "Y", 0x15, 0x59, 29, Layout;
    "KeyZ", "Z", 0x2C, 0x5A, 52, Layout;
    "Digit1", "1", 0x02, 0x31, 10, Layout;
    "Digit2", "2", 0x03, 0x32, 11, Layout;
    "Digit3", "3", 0x04, 0x33, 12, Layout;
    "Digit4", "4", 0x05, 0x34, 13, Layout;
    "Digit5", "5", 0x06, 0x35, 14, Layout;
    "Digit6", "6", 0x07, 0x36, 15, Layout;
    "Digit7", "7", 0x08, 0x37, 16, Layout;
    "Digit8", "8", 0x09, 0x38, 17, Layout;
    "Digit9", "9", 0x0A, 0x39, 18, Layout;
    "Digit0", "0", 0x0B, 0x30, 19, Layout;
    "Minus", "-", 0x0C, 0xBD, 20, Layout;
    "Equal", "=", 0x0D, 0xBB, 21, Layout;
    "BracketLeft", "[", 0x1A, 0xDB, 34, Layout;
    "BracketRight", "]", 0x1B, 0xDD, 35, Layout;
    "Backslash", "\\", 0x2B, 0xDC, 51, Layout;
    "Semicolon", ";", 0x27, 0xBA, 47, Layout;
    "Quote", "'", 0x28, 0xDE, 48, Layout;
    "Backquote", "`", 0x29, 0xC0, 49, Layout;
    "Comma", ",", 0x33, 0xBC, 59, Layout;
    "Period", ".", 0x34, 0xBE, 60, Layout;
    "Slash", "/", 0x35, 0xBF, 61, Layout;
    "IntlBackslash", "\\ (ISO)", 0x56, 0xE2, 94, Layout;
    "Escape", "Esc", 0x01, 0x1B, 9, Other;
    "Backspace", "Backspace", 0x0E, 0x08, 22, Other;
    "Tab", "Tab", 0x0F, 0x09, 23, Other;
    "Enter", "Enter", 0x1C, 0x0D, 36, Other;
    "Space", "Space", 0x39, 0x20, 65, Other;
    "ControlLeft", "Left Ctrl", 0x1D, 0xA2, 37, CTRL;
    "ControlRight", "Right Ctrl", 0xE01D, 0xA3, 105, CTRL;
    "ShiftLeft", "Left Shift", 0x2A, 0xA0, 50, SHIFT;
    "ShiftRight", "Right Shift", 0x36, 0xA1, 62, SHIFT;
    "AltLeft", "Left Alt", 0x38, 0xA4, 64, ALT;
    "AltRight", "Right Alt", 0xE038, 0xA5, 108, ALT;
    "MetaLeft", "Left Meta", 0xE05B, 0x5B, 133, META;
    "MetaRight", "Right Meta", 0xE05C, 0x5C, 134, META;
    "CapsLock", "Caps Lock", 0x3A, 0x14, 66, Other;
    "NumLock", "Num Lock", 0xE045, 0x90, 77, Other;
    "ScrollLock", "Scroll Lock", 0x46, 0x91, 78, Quiet;
    "F1", "F1", 0x3B, 0x70, 67, Function;
    "F2", "F2", 0x3C, 0x71, 68, Function;
    "F3", "F3", 0x3D, 0x72, 69, Function;
    "F4", "F4", 0x3E, 0x73, 70, Function;
    "F5", "F5", 0x3F, 0x74, 71, Function;
    "F6", "F6", 0x40, 0x75, 72, Function;
    "F7", "F7", 0x41, 0x76, 73, Function;
    "F8", "F8", 0x42, 0x77, 74, Function;
    "F9", "F9", 0x43, 0x78, 75, Function;
    "F10", "F10", 0x44, 0x79, 76, Function;
    "F11", "F11", 0x57, 0x7A, 95, Function;
    "F12", "F12", 0x58, 0x7B, 96, Function;
    "F13", "F13", 0x64, 0x7C, 191, Function;
    "F14", "F14", 0x65, 0x7D, 192, Function;
    "F15", "F15", 0x66, 0x7E, 193, Function;
    "F16", "F16", 0x67, 0x7F, 194, Function;
    "F17", "F17", 0x68, 0x80, 195, Function;
    "F18", "F18", 0x69, 0x81, 196, Function;
    "F19", "F19", 0x6A, 0x82, 197, Function;
    "F20", "F20", 0x6B, 0x83, 198, Function;
    "F21", "F21", 0x6C, 0x84, 199, Function;
    "F22", "F22", 0x6D, 0x85, 200, Function;
    "F23", "F23", 0x6E, 0x86, 201, Function;
    "F24", "F24", 0x76, 0x87, 202, Function;
    "PrintScreen", "Print Screen", 0xE037, 0x2C, 107, Other;
    "Pause", "Pause", 0x45, 0x13, 127, Quiet;
    "ContextMenu", "Menu", 0xE05D, 0x5D, 135, Other;
    "Insert", "Insert", 0xE052, 0x2D, 118, Other;
    "Delete", "Delete", 0xE053, 0x2E, 119, Other;
    "Home", "Home", 0xE047, 0x24, 110, Other;
    "End", "End", 0xE04F, 0x23, 115, Other;
    "PageUp", "Page Up", 0xE049, 0x21, 112, Other;
    "PageDown", "Page Down", 0xE051, 0x22, 117, Other;
    "ArrowUp", "Up", 0xE048, 0x26, 111, Other;
    "ArrowDown", "Down", 0xE050, 0x28, 116, Other;
    "ArrowLeft", "Left", 0xE04B, 0x25, 113, Other;
    "ArrowRight", "Right", 0xE04D, 0x27, 114, Other;
    "Numpad0", "Num 0", 0x52, 0x60, 90, Other;
    "Numpad1", "Num 1", 0x4F, 0x61, 87, Other;
    "Numpad2", "Num 2", 0x50, 0x62, 88, Other;
    "Numpad3", "Num 3", 0x51, 0x63, 89, Other;
    "Numpad4", "Num 4", 0x4B, 0x64, 83, Other;
    "Numpad5", "Num 5", 0x4C, 0x65, 84, Other;
    "Numpad6", "Num 6", 0x4D, 0x66, 85, Other;
    "Numpad7", "Num 7", 0x47, 0x67, 79, Other;
    "Numpad8", "Num 8", 0x48, 0x68, 80, Other;
    "Numpad9", "Num 9", 0x49, 0x69, 81, Other;
    "NumpadMultiply", "Num *", 0x37, 0x6A, 63, Other;
    "NumpadAdd", "Num +", 0x4E, 0x6B, 86, Other;
    "NumpadSubtract", "Num -", 0x4A, 0x6D, 82, Other;
    "NumpadDecimal", "Num .", 0x53, 0x6E, 91, Other;
    "NumpadDivide", "Num /", 0xE035, 0x6F, 106, Other;
    "NumpadEnter", "Num Enter", 0xE01C, 0x0D, 104, Other;
    "AudioVolumeMute", "Mute", 0xE020, 0xAD, 121, Media;
    "AudioVolumeDown", "Volume Down", 0xE02E, 0xAE, 122, Media;
    "AudioVolumeUp", "Volume Up", 0xE030, 0xAF, 123, Media;
    "MediaTrackNext", "Next Track", 0xE019, 0xB0, 171, Media;
    "MediaTrackPrevious", "Previous Track", 0xE010, 0xB1, 173, Media;
    "MediaStop", "Stop", 0xE024, 0xB2, 174, Media;
    "MediaPlayPause", "Play/Pause", 0xE022, 0xB3, 172, Media;
    "BrowserBack", "Browser Back", 0xE06A, 0xA6, 166, Media;
    "BrowserForward", "Browser Forward", 0xE069, 0xA7, 167, Media;
    "BrowserRefresh", "Browser Refresh", 0xE067, 0xA8, 181, Media;
    "BrowserHome", "Browser Home", 0xE032, 0xAC, 180, Media;
    "BrowserSearch", "Browser Search", 0xE065, 0xAA, 225, Media;
    "BrowserFavorites", "Browser Favorites", 0xE066, 0xAB, 164, Media;
    "LaunchMail", "Mail", 0xE06C, 0xB4, 163, Media;
}

/// A key of [`KEYS`]. Serialized as its `code` name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u8);

impl Key {
    /// The key named `code`, if sotyware knows it.
    pub fn from_code(code: &str) -> Option<Self> {
        KEYS.iter()
            .position(|k| k.code == code)
            .and_then(|i| u8::try_from(i).ok())
            .map(Self)
    }

    pub fn info(self) -> &'static KeyInfo {
        &KEYS[usize::from(self.0)]
    }

    pub fn code(self) -> &'static str {
        self.info().code
    }

    /// The English fallback name.
    pub fn label(self) -> &'static str {
        self.info().label
    }

    pub fn class(self) -> KeyClass {
        self.info().class
    }

    pub fn modifier(self) -> Option<Modifier> {
        match self.class() {
            KeyClass::Modifier(m) => Some(m),
            _ => None,
        }
    }

    /// The key Windows reports with this scan code (`0xE0` prefix for
    /// extended keys) and virtual-key code. The scan code decides (it is the
    /// physical key); a few keys share one and are told apart by `vk`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn from_windows(vk: u32, scan: u32, extended: bool) -> Option<Self> {
        const VK_CANCEL: u32 = 0x03; // Ctrl+Pause (Break)
        const VK_PAUSE: u32 = 0x13;
        const VK_SNAPSHOT: u32 = 0x2C;
        const VK_NUMLOCK: u32 = 0x90;
        match vk {
            VK_PAUSE | VK_CANCEL => return Self::from_code("Pause"),
            VK_NUMLOCK => return Self::from_code("NumLock"),
            VK_SNAPSHOT => return Self::from_code("PrintScreen"),
            _ => {}
        }
        let wanted = (scan & 0xFF) as u16 | if extended { 0xE000 } else { 0 };
        if scan & 0xFF != 0
            && let Some(i) = KEYS.iter().position(|k| k.win_scan == wanted)
        {
            return u8::try_from(i).ok().map(Self);
        }
        // No usable scan code (some software injects virtual keys only):
        // the virtual-key code of a key whose code does not depend on the
        // layout.
        KEYS.iter()
            .position(|k| u32::from(k.win_vk) == vk && k.class != KeyClass::Layout)
            .and_then(|i| u8::try_from(i).ok())
            .map(Self)
    }

    /// The key with X11 keycode `keycode`.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn from_x11(keycode: u32) -> Option<Self> {
        KEYS.iter()
            .position(|k| u32::from(k.x11) == keycode)
            .and_then(|i| u8::try_from(i).ok())
            .map(Self)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let code = String::deserialize(d)?;
        Self::from_code(&code)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown key '{code}'")))
    }
}

/// A shortcut: modifiers plus one key that is not a modifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: Key,
}

/// The modifiers held, as a set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Mods {
    pub fn set(&mut self, m: Modifier, down: bool) {
        match m {
            Modifier::Ctrl => self.ctrl = down,
            Modifier::Alt => self.alt = down,
            Modifier::Shift => self.shift = down,
            Modifier::Meta => self.meta = down,
        }
    }

    pub fn has(self, m: Modifier) -> bool {
        match m {
            Modifier::Ctrl => self.ctrl,
            Modifier::Alt => self.alt,
            Modifier::Shift => self.shift,
            Modifier::Meta => self.meta,
        }
    }
}

impl Chord {
    pub fn mods(self) -> Mods {
        Mods {
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            meta: self.meta,
        }
    }

    /// Parses `Ctrl+Alt+Shift+Meta+<code>`; modifiers in any order, each at
    /// most once.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut chord_mods = Mods::default();
        let parts: Vec<&str> = text.split('+').collect();
        let Some((last, mods)) = parts.split_last() else {
            return Err("empty shortcut".to_owned());
        };
        for m in mods {
            let modifier = match *m {
                "Ctrl" => Modifier::Ctrl,
                "Alt" => Modifier::Alt,
                "Shift" => Modifier::Shift,
                "Meta" => Modifier::Meta,
                other => return Err(format!("unknown modifier '{other}' in '{text}'")),
            };
            if chord_mods.has(modifier) {
                return Err(format!("'{m}' is twice in '{text}'"));
            }
            chord_mods.set(modifier, true);
        }
        let key =
            Key::from_code(last).ok_or_else(|| format!("unknown key '{last}' in '{text}'"))?;
        if key.modifier().is_some() {
            return Err(format!(
                "'{text}' ends with a modifier; a shortcut needs one more key"
            ));
        }
        Ok(Self {
            ctrl: chord_mods.ctrl,
            alt: chord_mods.alt,
            shift: chord_mods.shift,
            meta: chord_mods.meta,
            key,
        })
    }

    /// Why this shortcut cannot be used, if it cannot: a shortcut must not
    /// take a key away from typing (a letter, digit, Space, Enter… needs
    /// Ctrl, Alt or Meta), and a few combinations belong to the system.
    pub fn problem(self) -> Option<String> {
        let typing_safe = matches!(
            self.key.class(),
            KeyClass::Function | KeyClass::Media | KeyClass::Quiet
        );
        if !typing_safe && !(self.ctrl || self.alt || self.meta) {
            return Some(format!(
                "{} alone would stop that key from working everywhere; add Ctrl, Alt or the Windows key",
                self.label_with(|k| k.label().to_owned())
            ));
        }
        let code = self.key.code();
        let only = |ctrl, alt, shift, meta| {
            self.ctrl == ctrl && self.alt == alt && self.shift == shift && self.meta == meta
        };
        if code == "F12" && only(false, false, false, false) {
            return Some("F12 alone is reserved by Windows for debuggers".to_owned());
        }
        let reserved = (code == "Delete" && self.ctrl && self.alt)
            || (code == "Escape" && (self.ctrl || self.alt))
            || (code == "Tab" && self.alt)
            || (code == "F4" && only(false, true, false, false))
            || (code == "KeyL" && only(false, false, false, true));
        if reserved {
            return Some(format!(
                "{} belongs to the system",
                self.label_with(|k| k.label().to_owned())
            ));
        }
        None
    }

    /// For the UI: `Ctrl + Alt + P`, naming the key with `name`.
    pub fn label_with(self, name: impl Fn(Key) -> String) -> String {
        let mut out = String::new();
        for (on, m) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, "Meta"),
        ] {
            if on {
                out.push_str(m);
                out.push_str(" + ");
            }
        }
        out.push_str(&name(self.key));
        out
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, m) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.meta, "Meta+"),
        ] {
            if on {
                f.write_str(m)?;
            }
        }
        f.write_str(self.key.code())
    }
}

impl fmt::Debug for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str) -> Key {
        Key::from_code(code).unwrap()
    }

    #[test]
    fn every_code_scan_code_and_x11_keycode_is_unique() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert_ne!(a.code, b.code);
                assert_ne!(a.win_scan, b.win_scan, "{} and {}", a.code, b.code);
                assert_ne!(a.x11, b.x11, "{} and {}", a.code, b.code);
            }
        }
        assert!(KEYS.len() < 256);
    }

    #[test]
    fn keys_round_trip_through_their_codes_and_native_numbers() {
        for k in KEYS {
            let key = Key::from_code(k.code).unwrap();
            assert_eq!(key.code(), k.code);
            assert_eq!(Key::from_x11(u32::from(k.x11)), Some(key), "{}", k.code);
            let ext = k.win_scan & 0xE000 != 0;
            assert_eq!(
                Key::from_windows(u32::from(k.win_vk), u32::from(k.win_scan & 0xFF), ext),
                Some(key),
                "{}",
                k.code
            );
        }
        assert_eq!(
            Key::from_code("Power"),
            None,
            "power keys are never replayed"
        );
        assert_eq!(Key::from_code("keya"), None, "codes are case-sensitive");
    }

    #[test]
    fn windows_numbers_of_shared_scan_codes_name_the_right_key() {
        // NumLock and Pause share scan 0x45; Ctrl+Pause reports VK_CANCEL.
        assert_eq!(Key::from_windows(0x90, 0x45, true), Some(key("NumLock")));
        assert_eq!(Key::from_windows(0x13, 0x45, false), Some(key("Pause")));
        assert_eq!(Key::from_windows(0x03, 0x46, true), Some(key("Pause")));
        // The numeric keypad without NumLock reports navigation keys' VKs:
        // the scan code keeps the physical key.
        assert_eq!(Key::from_windows(0x24, 0x47, false), Some(key("Numpad7")));
        assert_eq!(Key::from_windows(0x24, 0x47, true), Some(key("Home")));
        // Enter and the keypad's Enter differ by the extended flag.
        assert_eq!(
            Key::from_windows(0x0D, 0x1C, true),
            Some(key("NumpadEnter"))
        );
        // The fake left Ctrl of AltGr carries 0x21D.
        assert_eq!(
            Key::from_windows(0xA2, 0x21D, false),
            Some(key("ControlLeft"))
        );
        // Injected without a scan code: only layout-independent keys.
        assert_eq!(Key::from_windows(0x75, 0, false), Some(key("F6")));
        assert_eq!(Key::from_windows(0x41, 0, false), None);
        // VK_PACKET (Unicode injection) is not a key.
        assert_eq!(Key::from_windows(0xE7, 0, false), None);
    }

    #[test]
    fn chords_parse_and_print_in_one_canonical_form() {
        let c = Chord::parse("Shift+Ctrl+KeyP").unwrap();
        assert!(c.ctrl && c.shift && !c.alt && !c.meta);
        assert_eq!(c.to_string(), "Ctrl+Shift+KeyP");
        assert_eq!(Chord::parse("F6").unwrap().to_string(), "F6");
        assert_eq!(
            serde_json::to_value(Chord::parse("Meta+Alt+Digit1").unwrap()).unwrap(),
            serde_json::json!("Alt+Meta+Digit1")
        );
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+KeyA",
            "Hyper+KeyA",
            "Ctrl+ShiftLeft",
            "Ctrl+Power",
        ] {
            assert!(Chord::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_shortcut_never_takes_a_key_away_from_typing() {
        let problem = |t: &str| Chord::parse(t).unwrap().problem();
        assert!(problem("KeyA").is_some());
        assert!(problem("Shift+KeyA").is_some(), "Shift+A types a capital A");
        assert!(problem("Space").is_some());
        assert!(problem("Numpad1").is_some());
        assert!(problem("Ctrl+KeyA").is_none());
        assert!(problem("Alt+Space").is_none());
        assert!(problem("F6").is_none());
        assert!(problem("Shift+F11").is_none());
        assert!(problem("Pause").is_none());
        assert!(problem("MediaPlayPause").is_none());
        // The system's own.
        assert!(problem("F12").is_some());
        assert!(problem("Ctrl+F12").is_none());
        assert!(problem("Ctrl+Alt+Delete").is_some());
        assert!(problem("Alt+Tab").is_some());
        assert!(problem("Alt+F4").is_some());
        assert!(problem("Ctrl+Shift+Escape").is_some());
        assert!(problem("Meta+KeyL").is_some());
    }

    #[test]
    fn labels_name_modifiers_then_the_key() {
        let c = Chord::parse("Ctrl+Alt+KeyP").unwrap();
        assert_eq!(c.label_with(|k| k.label().to_owned()), "Ctrl + Alt + P");
    }
}
