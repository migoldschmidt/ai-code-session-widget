//! Opens a Claude Desktop Code session directly (fork). Claude Desktop routes
//! `claude://claude.ai/<path>` to its internal page `<path>`, and the Code
//! tab's page for a session is `epitaxy/<hostSessionId>` — so
//! `claude://claude.ai/epitaxy/local_<uuid>` brings Desktop forward and
//! switches to that session, even from the Chat/Cowork view or the tray.
//!
//! Not documented (the Help Center only lists `claude://code/new` for Code),
//! verified by hand on Claude Desktop 2.42304. If a Desktop update breaks
//! it, the click still focuses the window and the optional UI Automation
//! sidebar selection remains as fallback.

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::com::ComApartment;

/// Builds the link, or `None` if `host_session_id` contains anything but
/// ASCII letters, digits, `-` and `_` (it looks like `local_<uuid>`). The id
/// ends up in a URL handed to the shell, so nothing else is let through.
pub fn desktop_session_url(host_session_id: &str) -> Option<String> {
    let id = host_session_id;
    let valid = !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| format!("claude://claude.ai/epitaxy/{id}"))
}

/// Hands the URL to the shell, which starts the registered `claude:`
/// protocol handler (same as `Start-Process <url>`). In-process via
/// `ShellExecuteW`: no child process and no command line. 0.3.0 started
/// `explorer.exe <url>`, a pattern behaviour-based antivirus rates as
/// suspicious (indirect execution through explorer).
pub fn open(url: &str) -> Result<(), String> {
    let url_w: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let _com = ComApartment::enter();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with a value above 32, errors as 0..=32.
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Could not open {url} (ShellExecute error {code})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_epitaxy_url_for_desktop_ids() {
        assert_eq!(
            desktop_session_url("local_0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d").as_deref(),
            Some("claude://claude.ai/epitaxy/local_0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d")
        );
    }

    #[test]
    fn rejects_anything_that_could_reach_a_command_line() {
        let too_long = "x".repeat(129);
        for bad in ["", "a b", "a&b", "a\"b", "../x", "x/y", "a|b", "a?b", "ä", too_long.as_str()] {
            assert_eq!(desktop_session_url(bad), None, "{bad:?}");
        }
    }
}
