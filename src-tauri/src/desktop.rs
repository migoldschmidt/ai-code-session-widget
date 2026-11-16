//! Window layer (fork): always on top, normal, or pinned to the desktop.
//!
//! "On desktop" keeps the widget like a desktop gadget: below the other
//! windows, without a taskbar button (windowLayer.ts hides it), and still
//! there after Win+D. "Always on top" and "normal" are minimized by Win+D
//! like any window and come back with the second Win+D.
//!
//! The Win+D part is the owner trick desktop gadget tools use: a window
//! owned by the desktop window (`Progman`) is not hidden by "Show desktop"
//! and can never drop below its owner, so it stays visible and clickable
//! above the icons. Only for the desktop layer: tested with Win+D, an owned
//! topmost window was minimized anyway and lost topmost on the way back.
//!
//! "Below the other windows" is not tauri's always-on-bottom: that forces
//! the window to the bottom on every z-order change, and since an owned
//! window must stay above its owner it dragged the desktop down again each
//! time Win+D raised it, so "Show desktop" never happened. Instead the widget
//! is pushed to the bottom when it gets pinned and whenever another window
//! becomes the foreground window (`EVENT_SYSTEM_FOREGROUND`): that covers
//! clicking away from the widget and the second Win+D, which drops the
//! desktop but leaves the widget up among the windows. Nothing moves when
//! the desktop itself comes to the foreground (first Win+D) or while it is
//! shown — then a watcher thread waits, read-only, for it to drop (the
//! second Win+D raises no event) — or when the widget is in place already.
//! A timer that moved it every 2 s hit the moment Win+D raised the desktop
//! and pulled it down.
//!
//! Bringing a pinned widget up for a while (tray click, dragging) is
//! `raise_pinned`: a plain SetWindowPos(HWND_TOPMOST) from the command
//! thread. tauri's set_always_on_top goes through the main thread, which
//! Windows keeps busy in its own loop while a window is dragged, so the
//! widget only came up after the drag. For the same reason the end of a drag
//! is read from the mouse buttons (`raise_pinned_for_drag`), not from window
//! events.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Once;
use std::thread;
use std::time::Duration;

use tauri::WebviewWindow;
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetClassNameW, GetWindow, GetWindowTextLengthW, IsIconic, IsWindowVisible,
    SetWindowLongPtrW, SetWindowPos, EVENT_SYSTEM_FOREGROUND, GWLP_HWNDPARENT, GW_HWNDNEXT,
    HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WINEVENT_OUTOFCONTEXT,
};

/// The widget window while it is pinned (raw handle, 0 otherwise); read by
/// the foreground hook.
static PINNED_HWND: AtomicIsize = AtomicIsize::new(0);
static HOOK_INSTALLED: Once = Once::new();
/// A watcher thread (see `push_to_bottom_when_possible`) is running.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// Counts raises and lowerings; a drag watcher only lowers the raise it made.
static RAISE_GENERATION: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowLayer {
    Top,
    Normal,
    Desktop,
}

impl WindowLayer {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "top" => Some(Self::Top),
            "normal" => Some(Self::Normal),
            "desktop" => Some(Self::Desktop),
            _ => None,
        }
    }
}

pub fn apply(window: &WebviewWindow, layer: WindowLayer) -> Result<(), String> {
    let on_desktop = layer == WindowLayer::Desktop;
    let hwnd = raw_hwnd(window)?;
    set_desktop_owner(hwnd, on_desktop)?;
    window.set_always_on_top(layer == WindowLayer::Top).map_err(|e| e.to_string())?;
    // Clears the flag of builds that used it (see the module docs).
    window.set_always_on_bottom(false).map_err(|e| e.to_string())?;
    PINNED_HWND.store(if on_desktop { hwnd.0 as isize } else { 0 }, Ordering::Relaxed);
    if on_desktop {
        // Ends a raise_pinned that is still on (tauri does not know of it).
        set_topmost(hwnd, false);
        let _ = push_to_bottom(hwnd);
    }
    Ok(())
}

/// Pinned widget up above all windows (`raised`) or back onto the desktop.
/// Keeps the desktop owner; while raised the foreground hook leaves it alone.
pub fn raise_pinned(window: &WebviewWindow, raised: bool) -> Result<(), String> {
    let hwnd = raw_hwnd(window)?;
    RAISE_GENERATION.fetch_add(1, Ordering::Relaxed);
    if raised {
        raise(hwnd);
    } else {
        lower(hwnd);
    }
    Ok(())
}

/// Mouse pressed on a drag region of the pinned widget: up until shortly
/// after the button is released (`DRAG_SETTLE`), checked every `DRAG_POLL`
/// off the UI thread.
const DRAG_POLL: Duration = Duration::from_millis(100);
const DRAG_SETTLE: Duration = Duration::from_millis(400);
/// Gives up after this long with a button held (e.g. a missed release).
const DRAG_LIMIT: u32 = 10 * 60 * 1000 / 100;

pub fn raise_pinned_for_drag(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = raw_hwnd(window)?;
    let generation = RAISE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    raise(hwnd);
    let hwnd_raw = hwnd.0 as isize;
    thread::spawn(move || {
        for _ in 0..DRAG_LIMIT {
            if !mouse_button_down() {
                break;
            }
            thread::sleep(DRAG_POLL);
        }
        thread::sleep(DRAG_SETTLE);
        // Another raise or lowering since (tray click, layer change): theirs.
        if RAISE_GENERATION.load(Ordering::Relaxed) == generation {
            lower(HWND(hwnd_raw as _));
        }
    });
    Ok(())
}

fn mouse_button_down() -> bool {
    // High bit: down right now. Both buttons, in case they are swapped.
    let down = |vk: i32| unsafe { GetAsyncKeyState(vk) } < 0;
    down(VK_LBUTTON.0 as i32) || down(VK_RBUTTON.0 as i32)
}

fn raise(hwnd: HWND) {
    PINNED_HWND.store(0, Ordering::Relaxed); // the foreground hook leaves it up
    set_topmost(hwnd, true);
}

fn lower(hwnd: HWND) {
    set_topmost(hwnd, false);
    PINNED_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    if !push_to_bottom(hwnd) {
        push_to_bottom_when_possible(hwnd.0 as isize);
    }
}

fn set_topmost(hwnd: HWND, on: bool) {
    let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    let _ = unsafe { SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
}

/// Puts the pinned widget below all windows, just above the desktop (its
/// owner). Does nothing when it is there already. Returns false, without
/// moving anything, while "Show desktop" has raised the desktop above the
/// windows: moving the widget down would take the desktop along and end it.
/// That check comes first, because while the desktop is shown the widget
/// sits right above it and would pass for "in place".
fn push_to_bottom(hwnd: HWND) -> bool {
    let Ok(progman) = (unsafe { FindWindowW(w!("Progman"), None) }) else {
        return true;
    };
    if desktop_is_shown(progman) {
        return false;
    }
    let in_place = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.is_ok_and(|next| next == progman);
    if in_place {
        return true;
    }
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    true
}

/// While the desktop is shown (or about to be: the first Win+D brings it to
/// the foreground), waits for it to drop and then moves the widget down. The
/// second Win+D raises no foreground event of its own, so this has to watch.
/// Off the UI thread, and read-only until the desktop is down again: only
/// z-order reads every 300 ms, which leave "Show desktop" alone.
const WATCH_INTERVAL: Duration = Duration::from_millis(300);
/// Stop watching after this long anyway (desktop left shown, nobody there).
const WATCH_LIMIT: u32 = 2 * 60 * 60 * 1000 / 300;

fn push_to_bottom_when_possible(hwnd_raw: isize) {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    thread::spawn(move || {
        for _ in 0..WATCH_LIMIT {
            thread::sleep(WATCH_INTERVAL);
            // Unpinned (or a new window handle) meanwhile, or done: stop.
            if PINNED_HWND.load(Ordering::Relaxed) != hwnd_raw || push_to_bottom(HWND(hwnd_raw as _)) {
                break;
            }
        }
        WATCHING.store(false, Ordering::Relaxed);
    });
}

/// Must run on the thread with the message loop (tauri's main thread, from
/// `setup` in lib.rs): out-of-context events are delivered there. Installed
/// from a command it never fired — commands run on other threads.
pub fn install_foreground_hook() {
    HOOK_INSTALLED.call_once(|| {
        // Lives as long as the app; if it fails, the pinned widget only
        // stays where Win+D or a click left it until it is pinned again.
        unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
        }
    });
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let pinned = PINNED_HWND.load(Ordering::Relaxed);
    if pinned == 0 || hwnd.0 as isize == pinned {
        return;
    }
    // The desktop came forward (first Win+D, or a click on it): never move
    // now, that would pull the desktop down mid "Show desktop".
    if is_desktop_window(hwnd) || !push_to_bottom(HWND(pinned as _)) {
        push_to_bottom_when_possible(pinned);
    }
}

/// The desktop windows Win+D brings to the foreground.
fn is_desktop_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 32];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    let class = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
    class == "Progman" || class == "WorkerW"
}

/// True when an open (visible, not minimized, titled) window sits below the
/// desktop window in the z-order — i.e. Win+D has raised the desktop.
fn desktop_is_shown(progman: HWND) -> bool {
    let mut next = unsafe { GetWindow(progman, GW_HWNDNEXT) };
    while let Ok(hwnd) = next {
        if hwnd.is_invalid() {
            break;
        }
        let open = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() && GetWindowTextLengthW(hwnd) > 0 };
        if open {
            return true;
        }
        next = unsafe { GetWindow(hwnd, GW_HWNDNEXT) };
    }
    false
}

fn set_desktop_owner(hwnd: HWND, enabled: bool) -> Result<(), String> {
    let owner = if enabled {
        unsafe { FindWindowW(w!("Progman"), None) }
            .map_err(|_| "Desktop window (Progman) not found".to_string())?
    } else {
        HWND::default()
    };
    unsafe { SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize) };
    Ok(())
}

// tauri links its own windows crate version, so the handle is passed on as
// the raw pointer.
fn raw_hwnd(window: &WebviewWindow) -> Result<HWND, String> {
    Ok(HWND(window.hwnd().map_err(|e| e.to_string())?.0 as _))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_layers() {
        assert_eq!(WindowLayer::parse("top"), Some(WindowLayer::Top));
        assert_eq!(WindowLayer::parse("normal"), Some(WindowLayer::Normal));
        assert_eq!(WindowLayer::parse("desktop"), Some(WindowLayer::Desktop));
    }

    #[test]
    fn rejects_unknown_layers() {
        assert_eq!(WindowLayer::parse(""), None);
        assert_eq!(WindowLayer::parse("Desktop"), None);
    }
}
