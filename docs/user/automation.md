# Autoclicker and macros

sotyware can click for you, record what you do with the mouse and the
keyboard, and replay it, from shortcuts that work in every program. This
guide explains how. (How it works inside: [automation.md](../architecture/automation.md).)

**Where it works:** Windows, and Linux desktops that run X11 (Xorg). Not on
Wayland desktops (most new Linux installations: sign in with an "Xorg" or
"X11" session instead) and not yet on macOS. There you can still edit,
import and export macros and profiles.

## Turn it on

Open **Autoclicker** in the sidebar. It is off until you turn it on:
**Turn on** (or Settings › Features › Autoclicker). From then on, while
sotyware runs, its shortcuts work in every program. Turning it off stops
anything that runs and gives the keys back to other programs. Nothing ever
starts by itself, also not after a restart.

## Click automatically

On the **Clicker** tab:

- **Mouse button**, **single or double** click, **speed** in clicks per
  second or milliseconds between clicks (at most 1,000 per second; many
  programs miss clicks faster than about 100 per second).
- **Repeat** until you stop it, or a number of clicks.
- **Where**: where the pointer is, or at a fixed position. **Pick…** reads
  the pointer's position after 3 seconds: move it to the spot meanwhile.
- **Advanced** adds: back and forward buttons, random intervals (between
  two values or ± a percentage), a time limit, a start delay, pauses
  between batches of clicks, a hold time, the gap of a double-click,
  several positions in turn or at random, random points in an area, and a
  spread around each position.

Changes are saved to the active profile as you make them. **Start
autoclicker** waits 3 seconds first (Settings › Safety), so you can point
where it should click. Or press **F6** in any program to start, and again
to stop.

## Record a macro

On the **Recorder** tab, **Start recording** (after the countdown, switch
to the program), or press **F9** in the program. sotyware records your
clicks, pointer movements, scrolling and key presses, with their timing.
Press **F10** (or **Stop recording** here) when you are done, then give the
macro a name and **Save macro**, or **Discard** it.

- Clicks on sotyware's own window and keys typed into it are not recorded,
  nor are the shortcut keys you press to pause or stop.
- **Everything you type while recording is saved in the macro, including
  passwords**, as plain text on this computer. Pause before typing
  anything secret.
- **Recording options** choose what is recorded, how many pointer
  movements are kept, keys and buttons never to record, and whether the
  macro replays at the same place on the screen, relative to where the
  pointer starts, or relative to the program's window.

## Edit a macro

On the **Macros** tab, open a macro (its name or the pencil). The editor
shows every action with its delay and time from the start, and a timeline.
You can:

- change any delay, position, button, key or shortcut;
- add actions (click, mouse button, move, scroll, key, shortcut, wait, a
  pause point where playback waits for you), move them up and down,
  duplicate and delete them, also several at once;
- set or scale the delays of the selected actions, or remove every pointer
  movement (their time is kept);
- **Preview**: a marker runs along the timeline at the playback speed;
  nothing is clicked or typed;
- set the macro's **shortcut**, its **program** (playback can warn or stop
  when another program is in front), and its **playback**: how many times,
  speed, delays, random variation, positions, a time limit, and **repeat
  only part of the macro** (before it plays once, then the part repeats,
  then the rest plays once).

**Save** keeps the changes; **Revert** drops them.

## Play a macro

**Play** in the library or the editor, the macro's own shortcut, or
**Shift+F11** for the profile's macro (Profiles tab). The Overview shows
the repetition, the action, the progress and the time left; **F7** pauses
and resumes; **Stop** or **F8** stops.

## Profiles

A profile keeps the autoclicker's settings, the shortcuts and a macro to
play (with its own playback settings if you like). Switch profiles at the
top of the page; the shortcuts change at once. **Start with it** makes a
profile the one sotyware starts with. Export and import profiles and
macros as `.sotyware.json` files to move them to another computer; imported
macro shortcuts are switched off until you turn them on.

## Shortcuts

Settings tab. Click a field and press the keys; Backspace clears it.
Suggested: F6 start/stop the autoclicker, F7 pause/resume, **F8 emergency
stop** (every profile), F9 start recording, F10 stop recording, Shift+F11
play the profile's macro. A shortcut cannot be a letter or other typing key
alone (add Ctrl, Alt or the Windows key), and a key another program already
uses cannot be taken: the tab shows which shortcuts work.

## Staying in control

- **F8** (the emergency stop) stops everything, from any program.
- **Push the pointer into a corner of the screen** to stop the autoclicker
  or a macro (Settings › Safety). A position in a corner that the
  autoclicker or a macro clicks itself does not stop it; push into
  another corner then, or press F8.
- The autoclicker never clicks sotyware's own window, so its **Stop**
  button always works. A click it skips there does not count towards a
  number of clicks: the page shows it as skipped, and the autoclicker goes
  on until it has clicked elsewhere that often or you stop it.
- Stopping, pausing or quitting releases every key and mouse button a
  macro held down.
- Quitting sotyware (tray › Quit) stops everything.

## What it cannot do

- Click or type into programs running as administrator, UAC prompts or the
  lock screen (unless sotyware also runs as administrator).
- Work on Wayland or macOS (yet).
- Replay Power, Sleep or Wake keys (never recorded).

More: [limitations](../status/limitations.md#autoclicker-adr-050).
