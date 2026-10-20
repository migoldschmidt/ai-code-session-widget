pub fn resolve_focus_pid(
    start_pid: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
    has_window: impl Fn(u32) -> bool,
    max_depth: usize,
) -> Option<u32> {
    let mut current = start_pid;
    for _ in 0..=max_depth {
        if has_window(current) {
            return Some(current);
        }
        match parent_of(current) {
            Some(parent) => current = parent,
            None => return None,
        }
    }
    None
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn returns_start_pid_when_it_owns_a_window() {
        let result = resolve_focus_pid(100, |_| None, |pid| pid == 100, 5);
        assert_eq!(result, Some(100));
    }

    #[test]
    fn walks_up_to_parent_that_owns_a_window() {
        let parents: HashMap<u32, u32> = [(100, 50)].into_iter().collect();
        let windowed = [50];

        let result = resolve_focus_pid(
            100,
            |pid| parents.get(&pid).copied(),
            |pid| windowed.contains(&pid),
            5,
        );

        assert_eq!(result, Some(50));
    }

    #[test]
    fn returns_none_when_no_ancestor_has_a_window() {
        let parents: HashMap<u32, u32> = [(100, 50), (50, 10)].into_iter().collect();

        let result = resolve_focus_pid(100, |pid| parents.get(&pid).copied(), |_| false, 5);

        assert_eq!(result, None);
    }

    #[test]
    fn stops_at_max_depth_to_avoid_infinite_loop() {
        // A cycle (10 -> 20 -> 10 -> ...) would loop forever without a depth cap.
        let parents: HashMap<u32, u32> = [(10, 20), (20, 10)].into_iter().collect();

        let result = resolve_focus_pid(10, |pid| parents.get(&pid).copied(), |_| false, 3);

        assert_eq!(result, None);
    }
}

use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Once;
use std::thread;
use std::time::Duration;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, SetWindowRgn, RGN_DIFF,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, TreeScope_Descendants, UIA_InvokePatternId,
    UIA_SelectionItemPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EnumWindows, GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
    ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, LWA_ALPHA, MSG,
    SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, WM_CLOSE,
    WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

struct FindAllContext {
    target_pid: u32,
    found: Vec<HWND>,
}

unsafe extern "system" fn enum_proc_collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindAllContext);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.target_pid {
        ctx.found.push(hwnd);
    }
    BOOL(1) // keep going, collect every match
}

/// Every visible top-level window owned by `pid` — there can be more than
/// one (e.g. several separate Windows Terminal windows all hosted by the
/// same WindowsTerminal.exe process).
fn all_windows_for_pid(pid: u32) -> Vec<HWND> {
    let mut ctx = FindAllContext {
        target_pid: pid,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc_collect), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.found
}

unsafe extern "system" fn enum_proc_collect_hidden(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindAllContext);
    if IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.target_pid {
        ctx.found.push(hwnd);
    }
    BOOL(1)
}

/// The most plausible "main window" among `pid`'s *hidden* top-level windows:
/// it has a title and is at least 200x200 — the largest such one wins. This
/// is what an app that was closed to the tray (Claude Desktop) leaves behind.
fn largest_hidden_titled_window(pid: u32) -> Option<HWND> {
    let mut ctx = FindAllContext {
        target_pid: pid,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc_collect_hidden),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }
    let mut best: Option<(i64, HWND)> = None;
    for hwnd in ctx.found {
        if unsafe { GetWindowTextLengthW(hwnd) } <= 0 {
            continue;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            continue;
        }
        let width = i64::from(rect.right - rect.left);
        let height = i64::from(rect.bottom - rect.top);
        if width < 200 || height < 200 {
            continue;
        }
        let area = width * height;
        if best.map_or(true, |(best_area, _)| area > best_area) {
            best = Some((area, hwnd));
        }
    }
    best.map(|(_, hwnd)| hwnd)
}

/// Fallback when no process in the session's ancestry has a visible window:
/// walk the same chain (stopping at the shell) and show the hidden main
/// window of the nearest ancestor that has one.
fn restore_hidden_window_in_chain(
    start_pid: u32,
    parent_of: impl Fn(u32) -> Option<u32>,
    is_shell: impl Fn(u32) -> bool,
    max_depth: usize,
) -> Option<HWND> {
    let mut current = start_pid;
    for _ in 0..=max_depth {
        if is_shell(current) {
            return None;
        }
        if let Some(hwnd) = largest_hidden_titled_window(current) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            return Some(hwnd);
        }
        current = parent_of(current)?;
    }
    None
}

fn window_for_pid(pid: u32) -> Option<HWND> {
    all_windows_for_pid(pid).into_iter().next()
}

pub fn pid_has_window(pid: u32) -> bool {
    window_for_pid(pid).is_some()
}

/// Like `pid_has_window`, but only counts a window that's actually
/// visibly sized on screen — filters out degenerate proxy windows that
/// technically pass `IsWindowVisible` while never being meant to be seen or
/// interacted with. Notably ConPTY's hidden, 0x0 `PseudoConsoleWindow`,
/// which Windows Terminal keeps one of per pane purely so legacy console
/// APIs have something to resolve.
///
/// Used to resolve the real, on-screen window that will end up hosting a
/// pane — as opposed to `pid_has_window`/`resolve_focus_pid`, which stop
/// climbing the process tree as soon as *any* pid owns a window at all,
/// and for a ConPTY-hosted pane that's almost always the pane's own private
/// hosting process (conhost/OpenConsole), not the terminal emulator (e.g.
/// WindowsTerminal.exe) whose window is what actually becomes visible.
fn pid_has_real_window(pid: u32) -> bool {
    all_windows_for_pid(pid)
        .into_iter()
        .any(|hwnd| visual_window_rect(hwnd).is_some())
}

/// Best-effort check of whether any UI Automation element under `hwnd`
/// (e.g. a Windows Terminal tab title, or visible pane text) contains
/// `hint` (case-insensitive). Used only to disambiguate between several
/// windows that share one process id — Win32 has no direct "which window
/// hosts this child process" API for apps like Windows Terminal that
/// multiplex several windows/panes through one process via ConPTY, so this
/// reads the window's own accessibility tree instead. Returns false (never
/// panics) if UI Automation is unavailable or the walk fails for any reason.
fn window_text_matches_hint(automation: &IUIAutomation, hwnd: HWND, hint_lower: &str) -> bool {
    unsafe {
        let Ok(element) = automation.ElementFromHandle(hwnd) else {
            return false;
        };
        let Ok(condition) = automation.CreateTrueCondition() else {
            return false;
        };
        let Ok(all) = element.FindAll(TreeScope_Descendants, &condition) else {
            return false;
        };
        let Ok(count) = all.Length() else {
            return false;
        };
        for i in 0..count {
            let Ok(item) = all.GetElement(i) else {
                continue;
            };
            let Ok(name) = item.CurrentName() else {
                continue;
            };
            if name.to_string().to_lowercase().contains(hint_lower) {
                return true;
            }
        }
    }
    false
}

/// Picks the right window among several candidates sharing one process id
/// by searching each one's UI Automation tree for `hint` (e.g. the
/// session's project folder name). Falls back to the first candidate if
/// there's only one, if UI Automation can't be initialized, or if none of
/// them match — a wrong-but-present window beats a silent no-op.
fn find_best_window(pid: u32, hint: &str) -> Option<HWND> {
    let candidates = all_windows_for_pid(pid);
    if candidates.len() <= 1 || hint.trim().is_empty() {
        return candidates.into_iter().next();
    }

    let hint_lower = hint.to_lowercase();
    let automation: Option<IUIAutomation> = unsafe {
        // COINIT_APARTMENTTHREADED can legitimately return an "already
        // initialized" error on a thread that's used this before — that's
        // fine, CoCreateInstance below still works either way.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    };

    if let Some(automation) = &automation {
        for hwnd in &candidates {
            if window_text_matches_hint(automation, *hwnd, &hint_lower) {
                return Some(*hwnd);
            }
        }
    }

    candidates.into_iter().next()
}

/// Finds the visible top-level window owned by `pid` (or, failing that, by
/// the nearest process ancestor that owns one) and brings it to the
/// foreground. If that owner has several windows open (common for apps
/// like Windows Terminal that host multiple windows from one process),
/// `hint` (typically the session's project folder name) is used to pick
/// the right one via UI Automation — see `find_best_window`.
///
/// Fork additions:
/// - The Windows shell (explorer.exe) is never a focus target and the walk
///   stops there. Without that, a session whose app is closed to the tray
///   (Claude Desktop) climbed on to explorer.exe and surfaced a random
///   Explorer or helper window. Instead the app's hidden main window is
///   shown again (`restore_hidden_window_in_chain`).
/// - With `sidebar_title`, the session is also selected in the app's own
///   list via UI Automation after focusing (`select_in_sidebar`).
///
/// Returns a short outcome code for the frontend's console.
pub fn focus_pid(pid: u32, hint: &str, sidebar_title: Option<&str>) -> String {
    use sysinfo::{Pid, System};

    // `new_all()` already performs a full refresh at construction — calling
    // `refresh_all()` again right after scanned the entire process table
    // twice for no reason.
    let sys = System::new_all();

    let parent_of = |p: u32| {
        sys.process(Pid::from_u32(p))
            .and_then(|proc| proc.parent())
            .map(|pp| pp.as_u32())
    };
    let is_shell = |p: u32| {
        sys.process(Pid::from_u32(p))
            .is_some_and(|proc| is_shell_process_name(&proc.name().to_string_lossy()))
    };
    // Same tree walk, but it never climbs past (or lands on) the shell.
    let parent_below_shell = |p: u32| if is_shell(p) { None } else { parent_of(p) };
    let has_window_not_shell = |p: u32| !is_shell(p) && pid_has_window(p);
    let has_real_window_not_shell = |p: u32| !is_shell(p) && pid_has_real_window(p);

    let Some(target_pid) = resolve_focus_pid(pid, parent_below_shell, has_window_not_shell, 10)
    else {
        let Some(hidden) = restore_hidden_window_in_chain(pid, parent_below_shell, is_shell, 10)
        else {
            return "no-window".to_string();
        };
        unsafe {
            force_foreground(hidden);
        }
        highlight_window(hidden);
        let mut outcome = "restored-hidden-window".to_string();
        if let Some(title) = sidebar_title {
            outcome.push_str(&format!(" sidebar={}", select_in_sidebar(hidden, title).code()));
        }
        return outcome;
    };

    let Some(hwnd) = find_best_window(target_pid, hint) else {
        return "no-window".to_string();
    };

    // `hwnd` itself can be a hidden, zero-size proxy window — notably
    // ConPTY's "PseudoConsoleWindow", which Windows Terminal keeps one of
    // per pane purely so legacy console APIs have something to resolve, and
    // which is what makes `force_foreground` below land on the right actual
    // terminal window/tab in the first place (each pane has its own such
    // window, one-to-one, so there's nothing to disambiguate at the pane
    // level — no UI Automation search needed there).
    //
    // What we highlight afterward can't just be "whatever's foreground now"
    // read back after the fact, though: `resolve_focus_pid` above stops
    // climbing the process tree as soon as *any* pid owns a window, and for
    // a ConPTY-hosted pane that's this same degenerate proxy pid, not the
    // terminal emulator's — so a foreground window belonging to that pid
    // was never coming, and re-reading ambient desktop state afterward (an
    // OS-level global that anything can briefly touch) to guess what
    // happened was exactly what made this flaky: it could catch a stale,
    // unrelated, or mid-transition window depending on timing, no matter
    // how that guess was gated. Resolve the *real* on-screen window we
    // expect this to surface as up front instead, by climbing the same
    // process tree again with `pid_has_real_window`, which requires an
    // actually visibly-sized window rather than any window at all — for a
    // ConPTY-hosted pane that lands on the terminal emulator itself (e.g.
    // WindowsTerminal.exe). There's then nothing left to reverse-engineer:
    // we already know which window to highlight before we've even asked
    // Windows to focus anything.
    let real_pid = resolve_focus_pid(target_pid, parent_below_shell, has_real_window_not_shell, 10)
        .unwrap_or(target_pid);
    let expected_hwnd = find_best_window(real_pid, hint).unwrap_or(hwnd);

    unsafe {
        force_foreground(hwnd);
    }
    highlight_window(expected_hwnd);

    let mut outcome = "focused".to_string();
    if let Some(title) = sidebar_title {
        outcome.push_str(&format!(" sidebar={}", select_in_sidebar(expected_hwnd, title).code()));
    }
    outcome
}

fn is_shell_process_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("explorer.exe")
}

// ---------------------------------------------------------------------------
// Sidebar selection via UI Automation (fork, opt-in)
// ---------------------------------------------------------------------------

/// How often the sidebar search is attempted. Electron/Chromium only builds
/// its accessibility tree once a UI Automation client shows up, so the first
/// walk can come back (nearly) empty.
const SIDEBAR_ATTEMPTS: u32 = 6;
const SIDEBAR_RETRY_MS: u64 = 400;

/// Accessible names of Claude Desktop's mode switch entry for Code. Verified
/// with scripts/dump-claude-ui.ps1 against Claude Desktop 2.42304 (German UI): two
/// RadioButtons, "Chat und Cowork" and "Code". While Code is not the active
/// view, Desktop appends a status badge after a comma ("Code, wartet auf
/// deine Eingabe"), so only the part before the first comma is compared.
const CODE_MODE_NAMES: &[&str] = &["Code", "Claude Code"];

/// Result of `select_in_sidebar`: what happened to the session entry, and
/// whether the app had to be switched to its Code view first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarResult {
    pub outcome: SidebarOutcome,
    pub switched_to_code: bool,
}

impl SidebarResult {
    pub fn code(self) -> String {
        if self.switched_to_code {
            format!("{}+switched-to-code", self.outcome.code())
        } else {
            self.outcome.code().to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarOutcome {
    /// Exactly one clickable element carried the title and it was invoked.
    Selected,
    /// No clickable element with that title, even after retrying.
    NotFound,
    /// Several clickable elements carry the title — nothing was clicked.
    Ambiguous,
    /// The only match refused both Invoke and Select.
    Failed,
    /// UI Automation could not be initialized.
    Unavailable,
}

impl SidebarOutcome {
    pub fn code(self) -> &'static str {
        match self {
            SidebarOutcome::Selected => "selected",
            SidebarOutcome::NotFound => "not-found",
            SidebarOutcome::Ambiguous => "ambiguous",
            SidebarOutcome::Failed => "failed",
            SidebarOutcome::Unavailable => "unavailable",
        }
    }
}

/// Collapses whitespace and lowercases, so "  Rework  login form"
/// and "rework login form" compare equal.
fn normalize_title(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleMatch {
    No,
    /// Title plus a space-separated prefix/suffix, e.g. a status word
    /// ("Inaktiv <title>") or extra info ("<title> Gestartet von …").
    Partial,
    Exact,
}

/// Exact (normalized) match first. A partial match only counts when the
/// title is set off by a space at the start or end — never "<title>, …",
/// which is how Desktop labels the header's rename button ("<title>,
/// Sitzung umbenennen"), and never somewhere in the middle (message text).
fn title_match(element_name: &str, wanted_normalized: &str) -> TitleMatch {
    if wanted_normalized.is_empty() {
        return TitleMatch::No;
    }
    let name = normalize_title(element_name);
    if name == wanted_normalized {
        TitleMatch::Exact
    } else if name.starts_with(&format!("{wanted_normalized} ")) || name.ends_with(&format!(" {wanted_normalized}")) {
        TitleMatch::Partial
    } else {
        TitleMatch::No
    }
}

fn is_code_mode_name(element_name: &str) -> bool {
    let before_badge = element_name.split(',').next().unwrap_or("");
    let name = normalize_title(before_badge);
    CODE_MODE_NAMES.iter().any(|candidate| normalize_title(candidate) == name)
}

/// Looks for exactly one clickable element (Invoke or SelectionItem
/// pattern) under `hwnd` whose name is the session title, and activates it.
/// If the title is ambiguous nothing is clicked — the window has already
/// been focused by the caller, which is the safe fallback.
///
/// The sidebar only lists Code sessions while Claude Desktop shows its Code
/// view, so the mode switch (Chat / Cowork / Code) is checked first:
/// - its Code entry is a selectable tab/radio item that is not selected →
///   Chat or Cowork is showing → activate Code, then search again;
/// - its state can't be read (plain button) → switch only if the title is
///   still missing on the second attempt (the first may just have woken up
///   Electron's accessibility tree).
/// The switch is tried at most once per click.
pub fn select_in_sidebar(hwnd: HWND, title: &str) -> SidebarResult {
    let mut result = SidebarResult {
        outcome: SidebarOutcome::NotFound,
        switched_to_code: false,
    };
    let wanted = normalize_title(title);
    if wanted.is_empty() {
        return result;
    }

    let automation: Option<IUIAutomation> = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    };
    let Some(automation) = automation else {
        result.outcome = SidebarOutcome::Unavailable;
        return result;
    };

    let title_or_code_mode =
        |name: &str| title_match(name, &wanted) != TitleMatch::No || is_code_mode_name(name);
    let mut switch_tried = false;
    for attempt in 0..SIDEBAR_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(SIDEBAR_RETRY_MS));
        }
        let found = unsafe { clickable_elements_where(&automation, hwnd, &title_or_code_mode) };
        let groups = unsafe { classify(found, &wanted) };

        if !switch_tried {
            if let Some(entry) = unsafe { pick_code_entry(&groups.code_mode) } {
                let no_title_yet = groups.exact.is_empty() && groups.partial.is_empty();
                let should_switch = match unsafe { is_selected(&entry) } {
                    Some(selected) => !selected,
                    None => no_title_yet && attempt >= 1,
                };
                if should_switch {
                    switch_tried = true;
                    result.switched_to_code = unsafe { activate_element(&entry) };
                    continue; // let the view change, then search again
                }
            }
        }

        // Exact names win; partial ones only count if there is no exact one.
        let candidates = if groups.exact.is_empty() { &groups.partial } else { &groups.exact };
        match candidates.len() {
            0 => {}
            1 => {
                result.outcome = if unsafe { activate_element(&candidates[0]) } {
                    SidebarOutcome::Selected
                } else {
                    SidebarOutcome::Failed
                };
                return result;
            }
            _ => {
                result.outcome = SidebarOutcome::Ambiguous;
                return result;
            }
        }
    }
    result
}

struct ElementGroups {
    exact: Vec<IUIAutomationElement>,
    partial: Vec<IUIAutomationElement>,
    code_mode: Vec<IUIAutomationElement>,
}

/// Sorts the scan results into exact title matches, partial title matches
/// and mode-switch entries.
unsafe fn classify(found: Vec<IUIAutomationElement>, wanted_normalized: &str) -> ElementGroups {
    let mut groups = ElementGroups {
        exact: Vec::new(),
        partial: Vec::new(),
        code_mode: Vec::new(),
    };
    for item in found {
        let name = item.CurrentName().map(|n| n.to_string()).unwrap_or_default();
        match title_match(&name, wanted_normalized) {
            TitleMatch::Exact => groups.exact.push(item),
            TitleMatch::Partial => groups.partial.push(item),
            TitleMatch::No => {
                if is_code_mode_name(&name) {
                    groups.code_mode.push(item);
                }
            }
        }
    }
    groups
}

/// The Code entry of the mode switch: the only selectable (tab / radio) one
/// if there is exactly one, else the only clickable one; `None` if unclear.
unsafe fn pick_code_entry(entries: &[IUIAutomationElement]) -> Option<IUIAutomationElement> {
    let mut selectable = Vec::new();
    for item in entries {
        if item.GetCurrentPattern(UIA_SelectionItemPatternId).is_ok() {
            selectable.push(item.clone());
        }
    }
    if selectable.len() == 1 {
        return selectable.pop();
    }
    if entries.len() == 1 {
        return Some(entries[0].clone());
    }
    None
}

/// Selection state of a tab/radio-like element; `None` if it has none.
unsafe fn is_selected(item: &IUIAutomationElement) -> Option<bool> {
    let unknown = item.GetCurrentPattern(UIA_SelectionItemPatternId).ok()?;
    let pattern = unknown.cast::<IUIAutomationSelectionItemPattern>().ok()?;
    pattern.CurrentIsSelected().ok().map(|selected| selected.as_bool())
}

unsafe fn supports_pattern_invoke_or_select(item: &IUIAutomationElement) -> bool {
    item.GetCurrentPattern(UIA_InvokePatternId).is_ok()
        || item.GetCurrentPattern(UIA_SelectionItemPatternId).is_ok()
}

/// Every clickable (Invoke or SelectionItem) descendant of `hwnd` whose
/// accessible name satisfies `name_matches`.
unsafe fn clickable_elements_where(
    automation: &IUIAutomation,
    hwnd: HWND,
    name_matches: &dyn Fn(&str) -> bool,
) -> Vec<IUIAutomationElement> {
    let mut found = Vec::new();
    let Ok(root) = automation.ElementFromHandle(hwnd) else {
        return found;
    };
    let Ok(condition) = automation.CreateTrueCondition() else {
        return found;
    };
    let Ok(all) = root.FindAll(TreeScope_Descendants, &condition) else {
        return found;
    };
    let Ok(count) = all.Length() else {
        return found;
    };
    for i in 0..count {
        let Ok(item) = all.GetElement(i) else {
            continue;
        };
        let Ok(name) = item.CurrentName() else {
            continue;
        };
        if name_matches(&name.to_string()) && supports_pattern_invoke_or_select(&item) {
            found.push(item);
        }
    }
    found
}

/// Invoke (buttons, links) first, then SelectionItem (list/tree items).
unsafe fn activate_element(item: &IUIAutomationElement) -> bool {
    if let Ok(unknown) = item.GetCurrentPattern(UIA_InvokePatternId) {
        if let Ok(pattern) = unknown.cast::<IUIAutomationInvokePattern>() {
            if pattern.Invoke().is_ok() {
                return true;
            }
        }
    }
    if let Ok(unknown) = item.GetCurrentPattern(UIA_SelectionItemPatternId) {
        if let Ok(pattern) = unknown.cast::<IUIAutomationSelectionItemPattern>() {
            if pattern.Select().is_ok() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod fork_tests {
    use super::*;

    #[test]
    fn explorer_is_the_shell_case_insensitively() {
        assert!(is_shell_process_name("explorer.exe"));
        assert!(is_shell_process_name("Explorer.EXE"));
        assert!(!is_shell_process_name("Claude.exe"));
        assert!(!is_shell_process_name("WindowsTerminal.exe"));
    }

    // Name patterns as seen in a scripts/dump-claude-ui.ps1 run against
    // Claude Desktop 2.42304 (German UI); the titles themselves are examples.
    // The German words are Desktop's real labels: "Gestartet von" (started
    // by), "Inaktiv" (inactive), "Sitzung umbenennen" (rename session),
    // "Nachrichtenaktionen für … hat geantwortet" (message actions for …).
    #[test]
    fn titles_match_exactly_after_normalizing_whitespace_and_case() {
        let wanted = normalize_title("Rework login form");
        assert_eq!(title_match("Rework login form", &wanted), TitleMatch::Exact);
        assert_eq!(title_match("  rework   LOGIN form ", &wanted), TitleMatch::Exact);
        assert_eq!(title_match("Login", &wanted), TitleMatch::No);
    }

    #[test]
    fn partial_title_needs_a_space_not_a_comma() {
        let wanted = normalize_title("Write changelog for v2.0");
        assert_eq!(
            title_match("Write changelog for v2.0 Gestartet von Claude aus Projekt X", &wanted),
            TitleMatch::Partial
        );
        let chat = normalize_title("Summarize notes");
        assert_eq!(title_match("Inaktiv Summarize notes", &chat), TitleMatch::Partial);
        // The header's rename button must never count as the sidebar entry.
        let wanted = normalize_title("Rework login form");
        assert_eq!(title_match("Rework login form, Sitzung umbenennen", &wanted), TitleMatch::No);
        assert_eq!(title_match("Nachrichtenaktionen für Rework login form hat geantwortet", &wanted), TitleMatch::No);
    }

    #[test]
    fn code_mode_entry_is_recognized_by_name() {
        assert!(is_code_mode_name("Code"));
        assert!(is_code_mode_name(" code "));
        assert!(is_code_mode_name("Code, wartet auf deine Eingabe"));
        assert!(is_code_mode_name("Claude Code"));
        assert!(!is_code_mode_name("Chat und Cowork"));
        assert!(!is_code_mode_name("Cowork"));
        assert!(!is_code_mode_name("Copy code"));
        assert!(!is_code_mode_name("In Claude Code öffnen"));
    }

    #[test]
    fn sidebar_result_codes() {
        let plain = SidebarResult { outcome: SidebarOutcome::Selected, switched_to_code: false };
        let switched = SidebarResult { outcome: SidebarOutcome::NotFound, switched_to_code: true };
        assert_eq!(plain.code(), "selected");
        assert_eq!(switched.code(), "not-found+switched-to-code");
    }

    #[test]
    fn empty_title_never_matches() {
        assert_eq!(title_match("", ""), TitleMatch::No);
        assert_eq!(title_match("anything", &normalize_title("   ")), TitleMatch::No);
    }

    #[test]
    fn stopping_at_the_shell_ends_the_walk() {
        // 100 (session) -> 50 (app, no visible window) -> 10 (shell, has windows)
        use std::collections::HashMap;
        let parents: HashMap<u32, u32> = [(100, 50), (50, 10), (10, 1)].into_iter().collect();
        let is_shell = |p: u32| p == 10;
        let parent_below_shell = |p: u32| if is_shell(p) { None } else { parents.get(&p).copied() };
        let has_window_not_shell = |p: u32| !is_shell(p) && (p == 10 || p == 1);

        assert_eq!(resolve_focus_pid(100, parent_below_shell, has_window_not_shell, 10), None);
    }
}

/// How long the highlight overlay stays on screen — long enough to spot at
/// a glance among several open terminal windows, short enough to not linger
/// and look stuck.
const HIGHLIGHT_DURATION_MS: u32 = 3000;

/// Frame thickness of the highlight overlay, in pixels.
const HIGHLIGHT_THICKNESS: i32 = 6;

// COLORREF is 0x00BBGGRR, not RGB — this is a vivid orange (RGB 255,140,0),
// chosen to stand out against both light and dark terminal themes.
const HIGHLIGHT_COLOR: COLORREF = COLORREF(0x00_00_8C_FF);

const OVERLAY_CLASS_NAME: windows::core::PCWSTR = w!("AiSessionWidgetFocusHighlightOverlay");

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // WM_TIMER is the overlay's own natural expiry; WM_CLOSE is how a
        // newer overlay (see ACTIVE_OVERLAY_HWND) asks an older, still-alive
        // one to get out of the way immediately instead of leaving two
        // overlays alive at once.
        WM_TIMER | WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// HWND (as its raw pointer value) of whichever highlight overlay is
/// currently showing, or 0 if none. Only one overlay is ever meant to be on
/// screen at a time — see `highlight_window`.
static ACTIVE_OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

fn ensure_overlay_class_registered() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wndproc),
            hInstance: module.into(),
            hbrBackground: CreateSolidBrush(HIGHLIGHT_COLOR),
            lpszClassName: OVERLAY_CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

/// `GetWindowRect` reports a window's *legacy* bounds, which for a
/// DWM-composed window include an invisible resize-border margin — several
/// pixels wide, and pushed even further off-screen for a maximized window —
/// that isn't part of what's actually drawn on screen. Positioning an
/// overlay from that rect lands its frame partly or entirely in that
/// invisible margin. `DWMWA_EXTENDED_FRAME_BOUNDS` reports the real visible
/// bounds instead; fall back to `GetWindowRect` if DWM can't answer (no
/// worse than before), and treat a degenerate (zero-area) result from
/// either as no answer at all.
fn visual_window_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut dwm_rect = RECT::default();
        let dwm_ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut dwm_rect as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
        .is_ok()
            && dwm_rect.right > dwm_rect.left
            && dwm_rect.bottom > dwm_rect.top;
        if dwm_ok {
            return Some(dwm_rect);
        }

        let mut gwr_rect = RECT::default();
        let gwr_ok = GetWindowRect(hwnd, &mut gwr_rect).is_ok()
            && gwr_rect.right > gwr_rect.left
            && gwr_rect.bottom > gwr_rect.top;
        gwr_ok.then_some(gwr_rect)
    }
}

/// Briefly draws a thick colored frame directly over `hwnd`'s on-screen
/// bounds — a hollow rectangle (via `SetWindowRgn`) so the window's own
/// content stays visible underneath — then destroys itself after
/// `HIGHLIGHT_DURATION_MS`.
///
/// This exists instead of the "obvious" native approach
/// (`DWMWA_BORDER_COLOR`, which lets DWM color a window's real border): that
/// call succeeds but is invisible in practice — apps like Windows Terminal
/// can overwrite it with their own focus-color logic, and maximized windows
/// hide their real border a few pixels off-screen. An owned topmost overlay
/// sidesteps both: it draws on top of whatever's there and follows the
/// window's actual screen rect regardless of maximize state.
///
/// Runs on its own thread with its own message loop, since the overlay
/// needs to pump `WM_TIMER`/`WM_DESTROY` independently of the caller.
fn highlight_window(hwnd: HWND) {
    // A window that just got raised from minimized/background — or, for a
    // ConPTY-hosted pane, is only now being brought forward as a side
    // effect of focusing its pane's proxy window a moment ago (see
    // `focus_pid`) — can briefly report a degenerate rect while DWM/the
    // terminal emulator finishes the transition. Retry for up to ~300ms
    // rather than silently giving up on the first miss; this returns as
    // soon as a real rect shows up, so the common case pays none of it.
    const RECT_RETRY_ATTEMPTS: u32 = 15;
    let mut rect = None;
    for attempt in 0..RECT_RETRY_ATTEMPTS {
        rect = visual_window_rect(hwnd);
        if rect.is_some() || attempt == RECT_RETRY_ATTEMPTS - 1 {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let Some(rect) = rect else {
        return;
    };

    thread::spawn(move || unsafe {
        ensure_overlay_class_registered();

        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };

        let Ok(overlay) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            OVERLAY_CLASS_NAME,
            w!(""),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            module,
            None,
        ) else {
            return;
        };

        // If a previous highlight is still on screen (e.g. a session was
        // clicked again before the last one's 3s timer ran out), ask it to
        // close right away rather than leaving two overlays alive at once —
        // DestroyWindow can only be called by the thread that created the
        // window, so this asks its own thread to do it via WM_CLOSE.
        let previous = ACTIVE_OVERLAY_HWND.swap(overlay.0 as isize, Ordering::SeqCst);
        if previous != 0 {
            let _ = PostMessageW(HWND(previous as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
        }

        // Punch out the interior so only a `HIGHLIGHT_THICKNESS`-wide frame
        // around the edge is actually drawn — the window underneath stays
        // fully visible and click-through (WS_EX_TRANSPARENT) inside it.
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let outer = CreateRectRgn(0, 0, width, height);
        let inner = CreateRectRgn(
            HIGHLIGHT_THICKNESS,
            HIGHLIGHT_THICKNESS,
            width - HIGHLIGHT_THICKNESS,
            height - HIGHLIGHT_THICKNESS,
        );
        CombineRgn(outer, outer, inner, RGN_DIFF);
        // SetWindowRgn takes ownership of `outer` (must not be deleted after
        // a successful call) — but `inner` was only a scratch input to
        // CombineRgn and is never handed to Windows, so it has to be deleted
        // here or it leaks a GDI region handle on every single highlight.
        let _ = DeleteObject(inner);
        SetWindowRgn(overlay, outer, BOOL(1));

        let _ = SetLayeredWindowAttributes(overlay, COLORREF(0), 255, LWA_ALPHA);
        let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
        // Belt-and-suspenders: WS_EX_TOPMOST at creation time doesn't always
        // win against other topmost surfaces already on screen (our own
        // always-on-top widget included) — explicitly reassert top-of-band.
        let _ = SetWindowPos(
            overlay,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        SetTimer(overlay, 1, HIGHLIGHT_DURATION_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Clear our slot, but only if a newer overlay hasn't already
        // claimed it (it would have, via the swap above, if we were closed
        // by WM_CLOSE rather than our own timer).
        let _ = ACTIVE_OVERLAY_HWND.compare_exchange(
            overlay.0 as isize,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    });
}

/// Windows normally refuses SetForegroundWindow from a background process
/// (the "foreground lock") unless the calling thread's input state is
/// attached to the currently-focused thread's — this is the standard
/// workaround: briefly attach, steal focus, detach. Without it,
/// SetForegroundWindow fails silently (no error, window just never comes
/// forward), which is exactly what made this a no-op for windows owned by
/// a background process like a Windows Terminal instance the user hadn't
/// just clicked into.
unsafe fn force_foreground(hwnd: HWND) {
    if IsIconic(hwnd).as_bool() {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }

    let foreground = GetForegroundWindow();
    let current_thread = GetCurrentThreadId();
    let foreground_thread = GetWindowThreadProcessId(foreground, None);

    let attached = foreground_thread != 0
        && foreground_thread != current_thread
        && AttachThreadInput(current_thread, foreground_thread, true).as_bool();

    let _ = BringWindowToTop(hwnd);
    let _ = SetForegroundWindow(hwnd);

    if attached {
        let _ = AttachThreadInput(current_thread, foreground_thread, false);
    }
}
