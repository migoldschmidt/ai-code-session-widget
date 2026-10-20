use crate::codex::CodexSessions;
use crate::model::build_session_list;
use crate::pid::is_session_alive;
use crate::usage::{
    find_runaway_orphans, sum_session_usage, total_disk_bytes, ProcSample, ResourceUsage,
    SessionsPayload,
};
use std::sync::{Mutex, OnceLock};
use sysinfo::{ProcessesToUpdate, System};
#[allow(unused_imports)] // package_info() lives on the Manager trait
use tauri::Manager;

static CODEX: OnceLock<Mutex<CodexSessions>> = OnceLock::new();
static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();

#[tauri::command]
pub async fn get_sessions() -> SessionsPayload {
    tauri::async_runtime::spawn_blocking(collect_sessions)
        .await
        .unwrap_or(SessionsPayload {
            sessions: Vec::new(),
            usage: ResourceUsage::default(),
        })
}

fn collect_sessions() -> SessionsPayload {
    let Some(home) = dirs::home_dir() else {
        return SessionsPayload {
            sessions: Vec::new(),
            usage: ResourceUsage::default(),
        };
    };
    let sessions_dir = home.join(".claude").join("sessions");
    let projects_dir = home.join(".claude").join("projects");

    // Kept alive across polls (rather than recreated each call) so sysinfo
    // has a prior sample to diff against — per-process CPU% is a delta since
    // the last refresh, and is meaningless (reads as 0) on a brand-new
    // System with nothing to compare to.
    let mut sys = SYSTEM.get_or_init(|| Mutex::new(System::new_all())).lock().unwrap_or_else(|e| e.into_inner());
    sys.refresh_cpu_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.refresh_memory();

    let mut sessions =
        build_session_list(&sessions_dir, &projects_dir, |pid, started| {
            is_session_alive(&sys, pid, started)
        });
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    // Recovers from a poisoned lock the same way SYSTEM does above, rather
    // than silently dropping every Codex session forever the first time a
    // panic (anywhere this mutex is held) poisons it.
    let mut codex = CODEX
        .get_or_init(|| Mutex::new(CodexSessions::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    sessions.extend(codex.collect(&codex_home, |pid| {
        sys.process(sysinfo::Pid::from_u32(pid)).is_some_and(|p| {
            let name = p.name().to_string_lossy();
            name.eq_ignore_ascii_case("codex.exe") || name.eq_ignore_ascii_case("codex")
        })
    }));

    let procs: Vec<ProcSample> = sys
        .processes()
        .values()
        .map(|p| {
            let disk = p.disk_usage();
            ProcSample {
                pid: p.pid().as_u32(),
                parent: p.parent().map(|pp| pp.as_u32()),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: p.cpu_usage(),
                memory_bytes: p.memory(),
                disk_bytes: disk.read_bytes + disk.written_bytes,
            }
        })
        .collect();
    for session in sessions.iter_mut().filter(|s| s.provider == "claude") {
        session.background_commands = crate::background::count_background_shells(&procs, session.pid);
    }
    let root_pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
    let totals = sum_session_usage(&procs, &root_pids, sys.cpus().len());

    let usage = ResourceUsage {
        session_cpu_percent: totals.cpu_percent,
        total_cpu_percent: sys.global_cpu_usage(),
        session_memory_bytes: totals.memory_bytes,
        total_memory_bytes: sys.total_memory(),
        session_disk_bytes: totals.disk_bytes,
        total_disk_bytes: total_disk_bytes(&procs),
        orphaned_processes: find_runaway_orphans(&procs),
    };

    SessionsPayload { sessions, usage }
}

/// `sidebar_title`: when set (setting "Select in sidebar"), the session is
/// also selected in the host app's list via UI Automation after focusing.
/// Returns a short outcome code (e.g. "focused sidebar=selected") that the
/// frontend logs to the console.
#[tauri::command]
pub async fn focus_session(pid: u32, hint: String, sidebar_title: Option<String>) -> String {
    // Walking the process tree and hunting for/highlighting a window is
    // blocking work (Win32 calls, a brief poll-and-sleep) — running it
    // synchronously on the command thread froze the whole UI for the
    // duration of every click.
    tauri::async_runtime::spawn_blocking(move || {
        crate::focus::focus_pid(pid, &hint, sidebar_title.as_deref())
    })
    .await
    .unwrap_or_else(|_| "internal-error".to_string())
}

/// Opens a Claude Desktop Code session via
/// `claude://claude.ai/epitaxy/<hostSessionId>` (see deeplink.rs). The id is
/// validated before anything runs.
#[tauri::command]
pub fn open_desktop_session(host_session_id: String) -> Result<(), String> {
    let url = crate::deeplink::desktop_session_url(&host_session_id)
        .ok_or_else(|| format!("Not a valid Claude Desktop session id: {host_session_id:?}"))?;
    crate::deeplink::open(&url)
}

#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> bool {
    crate::autostart::is_enabled(&app.package_info().name)
}

/// Async + spawn_blocking: writing the shortcut goes through COM and the file
/// system (and antivirus may scan it), and sync commands run on the main
/// thread, which would freeze the widget meanwhile.
#[tauri::command]
pub async fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let name = app.package_info().name.clone();
    tauri::async_runtime::spawn_blocking(move || crate::autostart::set_enabled(&name, enabled))
        .await
        .unwrap_or_else(|_| Err("Internal error while changing autostart".to_string()))
}

#[tauri::command]
pub async fn kill_orphan_process(pid: u32, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || kill_process_by_pid_and_name(pid, &name))
        .await
        .unwrap_or_else(|_| Err("Internal error while terminating the process".into()))
}

// Re-checks the pid immediately before killing and refuses unless its name
// still matches what the frontend showed the user — Windows recycles pids,
// so the two-second-old snapshot the confirm dialog was built from could by
// now belong to an unrelated process.
fn kill_process_by_pid_and_name(pid: u32, expected_name: &str) -> Result<(), String> {
    let mut sys = SYSTEM
        .get_or_init(|| Mutex::new(System::new_all()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    sys.refresh_processes(ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);

    let Some(process) = sys.process(sysinfo::Pid::from_u32(pid)) else {
        return Err("That process has already exited".into());
    };
    let actual_name = process.name().to_string_lossy();
    if !actual_name.eq_ignore_ascii_case(expected_name) {
        return Err(format!(
            "PID {pid} is now a different process ({actual_name}) — refusing to end it"
        ));
    }
    if process.kill() {
        Ok(())
    } else {
        Err("Failed to terminate the process".into())
    }
}

#[tauri::command]
pub fn set_always_on_top(window: tauri::Window, enabled: bool) {
    let _ = window.set_always_on_top(enabled);
}

#[tauri::command]
pub fn close_app(window: tauri::Window) {
    let _ = window.close();
}
