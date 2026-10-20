use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionFile {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    /// The hook's own status word — "idle", "busy", "waiting" (with a
    /// `waitingFor` reason, e.g. for an AskUserQuestion prompt), or absent
    /// entirely for a session whose entrypoint doesn't emit it. Kept as the
    /// raw string rather than collapsed to a bool so `status::compute_status`
    /// can trust "waiting" as its own unambiguous signal.
    pub hook_status: Option<String>,
    /// `statusUpdatedAt` exactly as Claude Code wrote it. Its format isn't
    /// documented (number or string), so it is passed through untouched and
    /// interpreted by the frontend (sessionOrder.ts::toEpochMs), which uses
    /// it to seed "time in current state" for sorting.
    pub status_updated_at: Option<serde_json::Value>,
    /// `hostSessionId`: the hosting app's id for this session ("local_…",
    /// set for Claude Desktop's Code tab). Opens the session directly via
    /// `claude://claude.ai/epitaxy/<id>` (deeplink.rs) and tells Desktop
    /// sessions from terminal ones.
    pub host_session_id: Option<String>,
    /// `procStart` converted to Unix seconds: when the session's process
    /// started. Lets pid.rs tell the session's own process from an unrelated
    /// one that got the same pid after a crash (fork).
    pub proc_start: Option<u64>,
}

#[derive(Deserialize)]
struct RawSessionFile {
    pid: u32,
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    name: String,
    #[serde(default)]
    status: Option<String>,
    // Both kept as raw JSON values so an unexpected type can never make the
    // whole file fail to parse (which would hide the session).
    #[serde(default, rename = "statusUpdatedAt")]
    status_updated_at: Option<serde_json::Value>,
    #[serde(default, rename = "hostSessionId")]
    host_session_id: Option<serde_json::Value>,
    #[serde(default, rename = "procStart")]
    proc_start: Option<serde_json::Value>,
}

/// Seconds between 1601-01-01 (Windows FILETIME epoch) and 1970-01-01.
const FILETIME_UNIX_OFFSET_SECS: u64 = 11_644_473_600;

/// `procStart` as Claude Code writes it on Windows: a FILETIME (100 ns since
/// 1601) as string or number, e.g. "134368033234460409". Anything else, or a
/// value before 1970, gives None (then only the pid is checked, as before).
pub fn proc_start_unix_secs(value: &serde_json::Value) -> Option<u64> {
    let ticks = match value {
        serde_json::Value::String(text) => text.trim().parse::<u64>().ok()?,
        serde_json::Value::Number(number) => number.as_u64()?,
        _ => return None,
    };
    (ticks / 10_000_000).checked_sub(FILETIME_UNIX_OFFSET_SECS).filter(|secs| *secs > 0)
}

pub fn project_dir_for_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|c| match c {
            ':' | '\\' | ' ' => '-',
            other => other,
        })
        .collect()
}

pub fn read_sessions_dir(dir: &Path) -> Vec<SessionFile> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(raw) = serde_json::from_str::<RawSessionFile>(&contents) else {
            continue;
        };
        sessions.push(SessionFile {
            pid: raw.pid,
            session_id: raw.session_id,
            cwd: raw.cwd,
            name: raw.name,
            hook_status: raw.status,
            status_updated_at: raw.status_updated_at,
            host_session_id: raw
                .host_session_id
                .and_then(|v| v.as_str().map(str::to_string))
                .filter(|s| !s.is_empty()),
            proc_start: raw.proc_start.as_ref().and_then(proc_start_unix_secs),
        });
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_start_filetime_becomes_unix_seconds() {
        // Value from a real Desktop session file (18.10.2026).
        let secs = proc_start_unix_secs(&serde_json::json!("134368033234460409")).unwrap();
        assert_eq!(secs, 1_792_329_723);
        assert_eq!(proc_start_unix_secs(&serde_json::json!(134368033234460409u64)), Some(secs));
        for bad in [serde_json::json!("x"), serde_json::json!(null), serde_json::json!(true), serde_json::json!(5)] {
            assert_eq!(proc_start_unix_secs(&bad), None, "{bad}");
        }
    }
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_file(dir: &Path, filename: &str, contents: &str) {
        let mut f = File::create(dir.join(filename)).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn parses_valid_session_file() {
        let dir = tempdir().unwrap();
        write_file(
            dir.path(),
            "1234.json",
            r#"{"pid":1234,"sessionId":"abc-123","cwd":"C:\\Projects\\foo","name":"foo-1a","status":"idle"}"#,
        );

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 1234);
        assert_eq!(result[0].session_id, "abc-123");
        assert_eq!(result[0].cwd, "C:\\Projects\\foo");
        assert_eq!(result[0].name, "foo-1a");
        assert_eq!(result[0].hook_status.as_deref(), Some("idle"));
        assert_eq!(result[0].status_updated_at, None);
        assert_eq!(result[0].host_session_id, None);
    }

    #[test]
    fn reads_desktop_fields_when_present() {
        let dir = tempdir().unwrap();
        write_file(
            dir.path(),
            "1234.json",
            r#"{"pid":1234,"sessionId":"abc","cwd":"C:\\x","name":"Dashboard","status":"idle","statusUpdatedAt":1789636143000,"hostSessionId":"local_42"}"#,
        );

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status_updated_at, Some(serde_json::json!(1789636143000u64)));
        assert_eq!(result[0].host_session_id.as_deref(), Some("local_42"));
    }

    #[test]
    fn unexpected_types_in_desktop_fields_do_not_hide_the_session() {
        let dir = tempdir().unwrap();
        write_file(
            dir.path(),
            "1234.json",
            r#"{"pid":1234,"sessionId":"abc","cwd":"C:\\x","name":"n","statusUpdatedAt":"2026-10-18T18:54:06Z","hostSessionId":42}"#,
        );

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status_updated_at, Some(serde_json::json!("2026-10-18T18:54:06Z")));
        assert_eq!(result[0].host_session_id, None);
    }

    #[test]
    fn treats_missing_status_as_none() {
        let dir = tempdir().unwrap();
        write_file(
            dir.path(),
            "1234.json",
            r#"{"pid":1234,"sessionId":"abc-123","cwd":"C:\\Projects\\foo","name":"foo-1a"}"#,
        );

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].hook_status, None);
    }

    #[test]
    fn skips_malformed_json_without_panicking() {
        let dir = tempdir().unwrap();
        write_file(dir.path(), "1234.json", "{not valid json");
        write_file(
            dir.path(),
            "5678.json",
            r#"{"pid":5678,"sessionId":"ok","cwd":"C:\\x","name":"n"}"#,
        );

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 5678);
    }

    #[test]
    fn ignores_non_json_files() {
        let dir = tempdir().unwrap();
        write_file(dir.path(), "1234.key", "some binary-ish content");

        let result = read_sessions_dir(dir.path());

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn missing_directory_returns_empty_list() {
        let result = read_sessions_dir(Path::new("C:\\does\\not\\exist\\at\\all"));
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn derives_project_dir_name_from_cwd() {
        assert_eq!(
            project_dir_for_cwd("C:\\Projects\\claude-widget"),
            "C--Projects-claude-widget"
        );
        // Covers a path with spaces as well as the drive colon and separators.
        assert_eq!(
            project_dir_for_cwd("C:\\Work\\Sites\\Platform\\v10\\Acme Corp\\example-site"),
            "C--Work-Sites-Platform-v10-Acme-Corp-example-site"
        );
    }
}
