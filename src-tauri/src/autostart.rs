//! Autostart via a shortcut in the user's Startup folder (fork). Plain file
//! in `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` — visible and
//! removable in Explorer and Task Manager, no registry entry.
//!
//! The shortcut is written in-process through the Shell's own link object
//! (IShellLinkW + IPersistFile), the same way Explorer creates one. Version
//! 0.3.0 started a hidden `powershell.exe -ExecutionPolicy Bypass` for this;
//! behaviour-based antivirus treated that as malware persistence and
//! removed the app. No child process is started any more.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::com::ComApartment;

/// Characters Windows doesn't allow in file names become `_`.
fn shortcut_file_name(product_name: &str) -> String {
    let cleaned: String = product_name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "AI Code Session Widget.lnk".to_string()
    } else {
        format!("{cleaned}.lnk")
    }
}

fn startup_dir() -> Option<PathBuf> {
    // dirs::config_dir() is %APPDATA% (Roaming) on Windows.
    Some(
        dirs::config_dir()?
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"),
    )
}

fn shortcut_path(product_name: &str) -> Option<PathBuf> {
    Some(startup_dir()?.join(shortcut_file_name(product_name)))
}

pub fn is_enabled(product_name: &str) -> bool {
    shortcut_path(product_name).is_some_and(|p| p.exists())
}

/// Creates or removes the shortcut and returns the new state. Creating is
/// refused in debug builds: `tauri dev` binaries need the Vite dev server and
/// would open an empty window at logon.
pub fn set_enabled(product_name: &str, enabled: bool) -> Result<bool, String> {
    let lnk = shortcut_path(product_name).ok_or("Startup folder not found")?;

    if !enabled {
        if lnk.exists() {
            std::fs::remove_file(&lnk).map_err(|e| format!("Could not remove {}: {e}", lnk.display()))?;
        }
        return Ok(false);
    }

    if cfg!(debug_assertions) {
        return Err(
            "Autostart only works with the installed app (npm run tauri build), not with tauri dev."
                .to_string(),
        );
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let work_dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    if let Some(dir) = lnk.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    write_shortcut(&lnk, &exe, &work_dir)?;
    if lnk.exists() {
        Ok(true)
    } else {
        Err(format!("Creating {} failed", lnk.display()))
    }
}

/// NUL-terminated UTF-16 for the Win32 calls below.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Writes a `.lnk` at `lnk` that starts `target` in `work_dir`.
fn write_shortcut(lnk: &Path, target: &Path, work_dir: &Path) -> Result<(), String> {
    let lnk_w = wide(lnk.as_os_str());
    let target_w = wide(target.as_os_str());
    let dir_w = wide(work_dir.as_os_str());

    // Dropped last, after the COM objects below.
    let _com = ComApartment::enter();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Shell link object unavailable: {e}"))?;
        link.SetPath(PCWSTR(target_w.as_ptr()))
            .map_err(|e| format!("Could not set the shortcut target: {e}"))?;
        link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))
            .map_err(|e| format!("Could not set the working directory: {e}"))?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(PCWSTR(lnk_w.as_ptr()), true)
            .map_err(|e| format!("Could not write {}: {e}", lnk.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_name_is_a_valid_file_name() {
        assert_eq!(shortcut_file_name("AI Code Session Widget"), "AI Code Session Widget.lnk");
        assert_eq!(shortcut_file_name("A:B/C"), "A_B_C.lnk");
        assert_eq!(shortcut_file_name("   "), "AI Code Session Widget.lnk");
    }

    #[test]
    fn shortcut_lives_in_the_startup_folder() {
        let path = shortcut_path("AI Code Session Widget").expect("APPDATA is set on Windows");
        let text = path.to_string_lossy().to_lowercase();
        assert!(text.ends_with("start menu\\programs\\startup\\ai code session widget.lnk"), "{text}");
    }

    /// Writes into a temp folder, never into the real Startup folder.
    #[test]
    fn writes_a_shell_link_without_a_child_process() {
        let dir = tempfile::tempdir().expect("temp dir");
        let lnk = dir.path().join("Test Widget.lnk");
        let exe = std::env::current_exe().expect("test binary path");
        let work_dir = exe.parent().expect("test binary folder");

        write_shortcut(&lnk, &exe, work_dir).expect("shortcut written");

        let bytes = std::fs::read(&lnk).expect("shortcut readable");
        // [MS-SHLLINK]: HeaderSize 0x4C, then LinkCLSID 00021401-0000-0000-C000-000000000046.
        assert_eq!(&bytes[..4], &[0x4C, 0, 0, 0]);
        assert_eq!(
            &bytes[4..20],
            &[0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46]
        );
        // The target's file name is stored, as ANSI path and/or UTF-16 long name.
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        let utf16: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(contains(name.as_bytes()) || contains(&utf16), "{name} not found in the shortcut");
    }
}
