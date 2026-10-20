use crate::pid::filter_alive;
use crate::sessions::{project_dir_for_cwd, read_sessions_dir};
use crate::status::{compute_status, extract_activity, read_tail_lines, SessionStatus};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub provider: &'static str,
    pub pid: u32,
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub status: SessionStatus,
    pub activity: Option<String>,
    /// Raw `statusUpdatedAt` from the session file (Claude only), see sessions.rs.
    pub status_updated_at: Option<serde_json::Value>,
    /// `hostSessionId` from the session file (Claude Desktop sessions only).
    pub host_session_id: Option<String>,
    /// Shells still running below the session process (fork, background.rs);
    /// filled in by commands.rs, 0 for Codex.
    pub background_commands: u32,
}

pub fn build_session_list(
    sessions_dir: &Path,
    projects_dir: &Path,
    is_alive: impl Fn(u32, Option<u64>) -> bool,
) -> Vec<SessionInfo> {
    let sessions = read_sessions_dir(sessions_dir);
    let alive = filter_alive(sessions, is_alive);

    alive
        .into_iter()
        .map(|s| {
            let tail = match resolve_transcript_path(projects_dir, &s.cwd, &s.session_id) {
                Some(transcript_path) => read_tail_lines(&transcript_path, 5),
                None => Vec::new(),
            };
            let status = compute_status(s.hook_status.as_deref(), &tail);
            let activity = extract_activity(status, &tail);
            SessionInfo {
                provider: "claude",
                pid: s.pid,
                session_id: s.session_id,
                name: s.name,
                cwd: s.cwd,
                status,
                activity,
                status_updated_at: s.status_updated_at,
                host_session_id: s.host_session_id,
                background_commands: 0,
            }
        })
        .collect()
}

/// Caches the fallback-scan result of `resolve_transcript_path`, keyed by
/// session id, so a session whose project dir name doesn't match the derived
/// scheme doesn't pay for a full `read_dir` of `projects_dir` on every 2s
/// poll for its entire lifetime.
static FALLBACK_TRANSCRIPT_PATHS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

/// Resolves the transcript file path for a session. Tries the fast path derived
/// directly from `cwd` first; if that doesn't exist (e.g. the derivation scheme
/// doesn't match how Claude Code actually named the project dir), falls back to
/// scanning `projects_dir` for any subdirectory containing `<session_id>.jsonl`.
fn resolve_transcript_path(projects_dir: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    let derived = projects_dir
        .join(project_dir_for_cwd(cwd))
        .join(format!("{session_id}.jsonl"));
    if derived.exists() {
        return Some(derived);
    }

    let cache = FALLBACK_TRANSCRIPT_PATHS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(session_id) {
        if cached.exists() {
            return Some(cached.clone());
        }
    }

    let entries = fs::read_dir(projects_dir).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if candidate.exists() {
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_id.to_string(), candidate.clone());
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn builds_session_info_with_status_from_transcript() {
        let root = tempdir().unwrap();
        let sessions_dir = root.path().join("sessions");
        let projects_dir = root.path().join("projects");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::create_dir_all(&projects_dir).unwrap();

        fs::write(
            sessions_dir.join("111.json"),
            r#"{"pid":111,"sessionId":"sess-1","cwd":"C:\\Projects\\foo","name":"foo-1a","status":"idle"}"#,
        )
        .unwrap();

        let project_subdir = projects_dir.join("C--Projects-foo");
        fs::create_dir_all(&project_subdir).unwrap();
        fs::write(
            project_subdir.join("sess-1.jsonl"),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let result = build_session_list(&sessions_dir, &projects_dir, |pid, _| pid == 111);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 111);
        assert_eq!(result[0].session_id, "sess-1");
        assert_eq!(result[0].name, "foo-1a");
        assert_eq!(result[0].status, SessionStatus::Waiting);
    }

    #[test]
    fn missing_transcript_file_defaults_to_waiting_without_panic() {
        // No status field (not idle) AND no transcript found anywhere —
        // no evidence of activity, so this should read as Waiting rather
        // than Working (see status::compute_status's empty-tail case).
        let root = tempdir().unwrap();
        let sessions_dir = root.path().join("sessions");
        let projects_dir = root.path().join("projects");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::create_dir_all(&projects_dir).unwrap();

        fs::write(
            sessions_dir.join("222.json"),
            r#"{"pid":222,"sessionId":"sess-2","cwd":"C:\\Projects\\bar","name":"bar-1a"}"#,
        )
        .unwrap();

        let result = build_session_list(&sessions_dir, &projects_dir, |pid, _| pid == 222);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, SessionStatus::Waiting);
    }

    #[test]
    fn dead_pid_is_excluded() {
        let root = tempdir().unwrap();
        let sessions_dir = root.path().join("sessions");
        let projects_dir = root.path().join("projects");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::create_dir_all(&projects_dir).unwrap();

        fs::write(
            sessions_dir.join("333.json"),
            r#"{"pid":333,"sessionId":"sess-3","cwd":"C:\\Projects\\baz","name":"baz-1a"}"#,
        )
        .unwrap();

        let result = build_session_list(&sessions_dir, &projects_dir, |_, _| false);

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn falls_back_to_scanning_project_dirs_when_derived_dir_does_not_exist() {
        let root = tempdir().unwrap();
        let sessions_dir = root.path().join("sessions");
        let projects_dir = root.path().join("projects");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::create_dir_all(&projects_dir).unwrap();

        // cwd derives to "C--Projects-missing", which will NOT exist on disk.
        fs::write(
            sessions_dir.join("444.json"),
            r#"{"pid":444,"sessionId":"sess-4","cwd":"C:\\Projects\\missing","name":"missing-1a","status":"idle"}"#,
        )
        .unwrap();

        // The transcript actually lives under a differently-named project dir.
        let actual_project_subdir = projects_dir.join("some-other-dir-name");
        fs::create_dir_all(&actual_project_subdir).unwrap();
        fs::write(
            actual_project_subdir.join("sess-4.jsonl"),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash"}]}}
"#,
        )
        .unwrap();

        let result = build_session_list(&sessions_dir, &projects_dir, |pid, _| pid == 444);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, SessionStatus::NeedsInput);
    }
}
