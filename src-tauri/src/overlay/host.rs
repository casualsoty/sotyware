//! The overlay window through Tauri (ADR-052): [`TauriHost`] is the
//! [`OverlayHost`] of the app.
//!
//! The window is created at runtime, the first time the overlay is shown
//! (never declared in `tauri.conf.json`, whose only window is the
//! dashboard), and destroyed when the feature stops: while the feature is
//! off there is no overlay webview at all. It has no native frame (the
//! page draws its own title bar), a transparent background (the page
//! paints the panel at the chosen opacity), stays above other windows,
//! has no taskbar button, cannot be maximized or minimized and takes no
//! dropped files (HTML drag and drop reorders the tabs).
//!
//! Showing it never takes the keyboard unless asked. On Windows, after the
//! window is created, sotyware shows, hides and styles it itself, without
//! activating it, and changes none of tao's window flags (`focus.rs` says
//! why); should Windows activate it anyway, the keyboard goes straight
//! back to the window that had it (logged). Elsewhere it is made
//! unfocusable for the show and focusable again afterwards when
//! interactive.

use std::sync::Mutex;

use tauri::window::Color;
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use super::model::{COLLAPSED_HEIGHT, MAX_SIZE, MIN_SIZE, Mode};
use super::{Geometry, OVERLAY_LABEL, OVERLAY_PAGE, OverlayHost, WindowInit, placement};
use crate::window_state::{Placement, Rect, Screen};

/// The window's title (Alt+Tab, the accessibility tree).
#[cfg(not(feature = "test-profile"))]
const TITLE: &str = "sotyware notes";
/// A test-profile build (ADR-027) says so, like its tray tooltip.
#[cfg(feature = "test-profile")]
const TITLE: &str = "sotyware notes (test profile)";

pub struct TauriHost {
    app: AppHandle,
    /// A test-profile build keeps the webview's data in its profile, like
    /// the dashboard's (ADR-027).
    #[cfg_attr(not(feature = "test-profile"), allow(dead_code))]
    webview_dir: Option<std::path::PathBuf>,
    /// The window that had the keyboard before the overlay took it
    /// (Windows: its handle).
    #[cfg_attr(not(windows), allow(dead_code))]
    previous: Mutex<Option<isize>>,
}

impl TauriHost {
    pub fn new(app: AppHandle, webview_dir: Option<std::path::PathBuf>) -> Self {
        Self {
            app,
            webview_dir,
            previous: Mutex::new(None),
        }
    }

    fn window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(OVERLAY_LABEL)
    }

    fn require(&self) -> Result<WebviewWindow, String> {
        self.window()
            .ok_or_else(|| "the overlay window does not exist".to_owned())
    }

    #[cfg(windows)]
    fn handle(w: &WebviewWindow) -> Option<isize> {
        w.hwnd().ok().map(|h| h.0 as isize)
    }
}

fn screen_of(m: &tauri::Monitor) -> Screen {
    let w = m.work_area();
    Screen {
        work: Rect {
            x: w.position.x,
            y: w.position.y,
            width: w.size.width,
            height: w.size.height,
        },
        scale: m.scale_factor(),
    }
}

fn usable(s: &Screen) -> bool {
    s.work.width > 0 && s.work.height > 0 && s.scale.is_finite() && s.scale > 0.0
}

/// Position first (it may move the window to a monitor with another
/// scaling, which resizes it), then the exact size, then the position
/// again, as the dashboard does (`window_state::apply`).
fn apply(w: &WebviewWindow, p: Placement, collapsed: bool) -> Result<(), String> {
    let position = PhysicalPosition::new(p.x, p.y);
    w.set_position(position).map_err(|e| e.to_string())?;
    let scale = w.scale_factor().unwrap_or(1.0);
    let height = if collapsed {
        placement::collapsed_height(scale)
    } else {
        p.height
    };
    w.set_size(PhysicalSize::new(p.width, height))
        .map_err(|e| e.to_string())?;
    w.set_position(position).map_err(|e| e.to_string())
}

impl OverlayHost for TauriHost {
    fn geometry(&self) -> Geometry {
        Geometry {
            screens: self
                .app
                .available_monitors()
                .unwrap_or_default()
                .iter()
                .map(screen_of)
                .filter(usable)
                .collect(),
            primary: self
                .app
                .primary_monitor()
                .ok()
                .flatten()
                .map(|m| screen_of(&m))
                .filter(usable),
            cursor: self
                .app
                .cursor_position()
                .ok()
                .map(|p| (p.x, p.y))
                .filter(|(x, y)| x.is_finite() && y.is_finite()),
        }
    }

    fn exists(&self) -> bool {
        self.window().is_some()
    }

    fn create(&self, init: &WindowInit) -> Result<(), String> {
        let min_height = if init.collapsed {
            COLLAPSED_HEIGHT
        } else {
            MIN_SIZE.1
        };
        let mut builder = WebviewWindowBuilder::new(
            &self.app,
            OVERLAY_LABEL,
            WebviewUrl::App(OVERLAY_PAGE.into()),
        )
        .title(TITLE)
        .decorations(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .focused(false)
        // On Windows tao's flags stay as created; `focus::set_mode` gives
        // the passive mode its styles.
        .focusable(cfg!(windows) || init.mode == Mode::Interactive)
        .visible(false)
        .inner_size(f64::from(MIN_SIZE.0), f64::from(min_height))
        .min_inner_size(f64::from(MIN_SIZE.0), f64::from(min_height))
        .disable_drag_drop_handler();
        if init.collapsed {
            builder = builder.max_inner_size(f64::from(MAX_SIZE.0), f64::from(COLLAPSED_HEIGHT));
        }
        #[cfg(feature = "test-profile")]
        if let Some(dir) = &self.webview_dir {
            builder = builder.data_directory(dir.clone());
        }
        let w = builder.build().map_err(|e| e.to_string())?;
        apply(&w, init.placement, init.collapsed)?;
        #[cfg(windows)]
        {
            let h = Self::handle(&w).ok_or("the overlay window has no handle")?;
            super::focus::set_mode(h, init.mode == Mode::Passive);
        }
        #[cfg(not(windows))]
        if init.mode == Mode::Passive {
            w.set_ignore_cursor_events(true)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn bounds(&self) -> Option<(Rect, f64)> {
        let w = self.window()?;
        let (position, size) = (w.outer_position().ok()?, w.inner_size().ok()?);
        Some((
            Rect {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            },
            w.scale_factor().unwrap_or(1.0),
        ))
    }

    fn place(&self, placement: Placement, collapsed: bool) -> Result<(), String> {
        apply(&self.require()?, placement, collapsed)
    }

    #[cfg(windows)]
    fn show(&self, mode: Mode, activate: bool) -> Result<(), String> {
        let w = self.require()?;
        let h = Self::handle(&w).ok_or("the overlay window has no handle")?;
        // Tauri's setters (the place just given) reach the event loop as
        // posted messages, which a `SetWindowPos` from this thread would
        // overtake: shown first, the window would appear where it was and
        // then jump. A getter waits for its answer behind them.
        let _ = w.outer_position();
        let before = super::focus::foreground();
        // The mode's styles first, then shown at the top of the topmost
        // windows (above a game that made itself topmost after it),
        // neither of which activates it.
        super::focus::set_mode(h, mode == Mode::Passive);
        super::focus::show(h);
        if !activate
            && let Some(before) = before
            && before != h
            && super::focus::foreground() == Some(h)
        {
            eprintln!("sotyware: the overlay took the keyboard when shown; giving it back");
            if !super::focus::activate(before) {
                eprintln!("sotyware: Windows did not give the keyboard back");
            }
        }
        if activate {
            self.take_focus()?;
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn show(&self, mode: Mode, activate: bool) -> Result<(), String> {
        let w = self.require()?;
        if w.is_minimized().unwrap_or(false)
            && let Err(e) = w.unminimize()
        {
            eprintln!("sotyware: cannot restore the overlay: {e}");
        }
        if !activate {
            w.set_focusable(false).map_err(|e| e.to_string())?;
        }
        w.show().map_err(|e| e.to_string())?;
        if mode == Mode::Interactive {
            w.set_focusable(true).map_err(|e| e.to_string())?;
        }
        if activate {
            self.take_focus()?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn hide(&self) -> Result<(), String> {
        let w = self.require()?;
        let h = Self::handle(&w).ok_or("the overlay window has no handle")?;
        super::focus::hide(h);
        Ok(())
    }

    #[cfg(not(windows))]
    fn hide(&self) -> Result<(), String> {
        self.require()?.hide().map_err(|e| e.to_string())
    }

    fn destroy(&self) {
        if let Some(w) = self.window()
            && let Err(e) = w.destroy()
        {
            eprintln!("sotyware: cannot close the overlay window: {e}");
        }
        *self.previous.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[cfg(windows)]
    fn set_mode(&self, mode: Mode) -> Result<(), String> {
        let w = self.require()?;
        let h = Self::handle(&w).ok_or("the overlay window has no handle")?;
        super::focus::set_mode(h, mode == Mode::Passive);
        Ok(())
    }

    #[cfg(not(windows))]
    fn set_mode(&self, mode: Mode) -> Result<(), String> {
        let w = self.require()?;
        w.set_ignore_cursor_events(mode == Mode::Passive)
            .map_err(|e| e.to_string())?;
        w.set_focusable(mode == Mode::Interactive)
            .map_err(|e| e.to_string())
    }

    #[cfg(windows)]
    fn take_focus(&self) -> Result<(), String> {
        let w = self.require()?;
        let own = Self::handle(&w).ok_or("the overlay window has no handle")?;
        if let Some(fg) = super::focus::foreground()
            && fg != own
            && !super::focus::is_own_process(fg)
        {
            *self.previous.lock().unwrap_or_else(|e| e.into_inner()) = Some(fg);
        }
        // Allowed right after the user's input reached sotyware (the
        // shortcut that asked for it); otherwise the overlay stays without
        // the keyboard rather than making up a key press.
        if !super::focus::activate(own) {
            eprintln!("sotyware: Windows did not give the overlay the keyboard");
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn take_focus(&self) -> Result<(), String> {
        self.require()?.set_focus().map_err(|e| e.to_string())
    }

    fn give_focus_back(&self) {
        #[cfg(windows)]
        {
            let Some(own) = self.window().as_ref().and_then(Self::handle) else {
                return;
            };
            if super::focus::foreground() != Some(own) {
                return;
            }
            let previous = self
                .previous
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .filter(|&p| super::focus::is_window(p));
            match previous.or_else(|| super::focus::next_below(own)) {
                Some(p) => {
                    if !super::focus::activate(p) {
                        eprintln!("sotyware: Windows did not give the keyboard back");
                    }
                }
                None => eprintln!("sotyware: no window to give the keyboard back to"),
            }
        }
        // Elsewhere the window manager decides; the page drops the
        // keyboard in passive mode (it focuses nothing and edits nothing).
    }

    /// Collapsed, the height is pinned by the size limits, not by making
    /// the window non-resizable: GTK sizes a non-resizable window to its
    /// content (the webview asks for 200 px), and the width stays
    /// adjustable.
    fn set_collapsed(&self, collapsed: bool, expanded_height: u32) -> Result<(), String> {
        let w = self.require()?;
        let scale = w.scale_factor().unwrap_or(1.0);
        let width = w.inner_size().map_err(|e| e.to_string())?.width;
        if collapsed {
            w.set_min_size(Some(LogicalSize::new(
                f64::from(MIN_SIZE.0),
                f64::from(COLLAPSED_HEIGHT),
            )))
            .map_err(|e| e.to_string())?;
            w.set_max_size(Some(LogicalSize::new(
                f64::from(MAX_SIZE.0),
                f64::from(COLLAPSED_HEIGHT),
            )))
            .map_err(|e| e.to_string())?;
            w.set_size(PhysicalSize::new(width, placement::collapsed_height(scale)))
                .map_err(|e| e.to_string())
        } else {
            let min = (f64::from(MIN_SIZE.1) * scale).round() as u32;
            w.set_max_size(None::<LogicalSize<f64>>)
                .map_err(|e| e.to_string())?;
            w.set_size(PhysicalSize::new(width, expanded_height.max(min)))
                .map_err(|e| e.to_string())?;
            w.set_min_size(Some(LogicalSize::new(
                f64::from(MIN_SIZE.0),
                f64::from(MIN_SIZE.1),
            )))
            .map_err(|e| e.to_string())
        }
    }

    fn show_dashboard(&self) {
        crate::show_dashboard(&self.app);
    }

    fn open_link(&self, url: &str) -> Result<(), String> {
        crate::github_page::open_in_browser(url)
    }
}
