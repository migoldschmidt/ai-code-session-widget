//! Read-only adapter for local Codex rollouts and Windows writer locks.
//! These are internal Codex formats; see docs/codex-sessions.md for limits.
use crate::{model::SessionInfo, status::SessionStatus};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct CodexSessions {
    entries: HashMap<String, Rollout>,
}

struct Rollout {
    path: PathBuf,
    offset: u64,
    pid: u32,
    /// When the writer-pid lookup (an expensive Restart Manager round trip)
    /// last ran and came up empty — gates retries below so a rollout with no
    /// resolvable owner doesn't pay that cost on every single poll.
    pid_lookup_failed_at: Option<Instant>,
    state: RolloutState,
}

/// How long to wait before retrying a failed writer-pid lookup for the same
/// rollout. Only gates *repeated* failures (pid stuck at 0) — a pid that
/// was previously resolved and then died is retried immediately.
const PID_LOOKUP_BACKOFF: Duration = Duration::from_secs(10);

struct RolloutState {
    id: String,
    cwd: String,
    child: bool,
    status: SessionStatus,
    activity: Option<String>,
    pending_input: HashMap<String, String>,
}

impl Default for RolloutState {
    fn default() -> Self {
        Self {
            id: String::new(),
            cwd: String::new(),
            child: false,
            status: SessionStatus::Waiting,
            activity: None,
            pending_input: HashMap::new(),
        }
    }
}

impl RolloutState {
    fn apply(&mut self, value: Value) {
        let p = &value["payload"];
        let kind = p["type"].as_str().unwrap_or("");
        match value["type"].as_str().unwrap_or("") {
            "session_meta" => {
                self.id = p["id"].as_str().unwrap_or("").into();
                self.cwd = p["cwd"].as_str().unwrap_or("").into();
                self.child = p["source"].get("subagent").is_some();
            }
            "turn_context" => {
                if let Some(cwd) = p["cwd"].as_str() {
                    self.cwd = cwd.into();
                }
            }
            "event_msg" => match kind {
                "task_started" | "turn_started" | "user_message" => {
                    self.pending_input.clear();
                    self.status = SessionStatus::Working;
                    self.activity = None;
                }
                "task_complete" | "task_completed" | "turn_complete" | "turn_aborted" => {
                    self.pending_input.clear();
                    self.status = SessionStatus::Waiting;
                    self.activity = p["last_agent_message"]
                        .as_str()
                        .filter(|s| s.trim_end().ends_with('?'))
                        .map(|_| "Ended with a question".into());
                }
                _ => {}
            },
            "response_item" => match kind {
                "function_call" | "custom_tool_call" => {
                    let name = p["name"].as_str().unwrap_or("tool");
                    let short_name = name.rsplit('.').next().unwrap_or(name);
                    let label = match short_name {
                        "exec" | "exec_command" | "shell" | "shell_command" | "write_stdin" => {
                            "Running a command".into()
                        }
                        "apply_patch" => "Editing files".into(),
                        "web" | "web__run" => "Searching the web".into(),
                        _ => format!("Using {short_name}"),
                    };
                    // A pending ordinary tool is still working, not evidence of approval.
                    if short_name == "request_user_input" {
                        if let Some(id) = p["call_id"].as_str() {
                            self.pending_input
                                .insert(id.into(), "Asked a question".into());
                        }
                    }
                    self.status = if self.pending_input.is_empty() {
                        SessionStatus::Working
                    } else {
                        SessionStatus::NeedsInput
                    };
                    self.activity = self.pending_input.values().next().cloned().or(Some(label));
                }
                "function_call_output" | "custom_tool_call_output" => {
                    if let Some(id) = p["call_id"].as_str() {
                        self.pending_input.remove(id);
                    }
                    self.status = if self.pending_input.is_empty() {
                        SessionStatus::Working
                    } else {
                        SessionStatus::NeedsInput
                    };
                    self.activity = self.pending_input.values().next().cloned();
                }
                "message" if p["role"] == "assistant" && p["phase"] == "final_answer" => {
                    self.pending_input.clear();
                    self.status = SessionStatus::Waiting;
                    self.activity = p["content"]
                        .as_array()
                        .and_then(|items| items.last())
                        .and_then(|item| item["text"].as_str())
                        .filter(|text| text.trim_end().ends_with('?'))
                        .map(|_| "Ended with a question".into());
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl Rollout {
    fn refresh(&mut self) {
        let Ok(mut file) = File::open(&self.path) else {
            return;
        };
        let Ok(meta) = file.metadata() else { return };
        if meta.len() < self.offset {
            self.offset = 0;
            self.state = RolloutState::default();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let Ok(n) = reader.read_until(b'\n', &mut line) else {
                break;
            };
            // Retry incomplete JSONL writes on the next poll.
            if n == 0 || line.last() != Some(&b'\n') {
                break;
            }
            self.offset += n as u64;
            if let Ok(value) = serde_json::from_slice(&line) {
                self.state.apply(value);
            }
        }
    }
}

/// Recursively hunts `dir` for rollout files matching any of `suffixes`
/// (precomputed once per `collect_with` call, not per file visited — the
/// naive version reallocated a `-{id}.jsonl` pattern string for every
/// (file, wanted-id) pair). Stops descending as soon as every id has been
/// found, since a typical call is only missing one or two rollouts out of
/// however many sessions exist on disk.
fn find_rollouts(
    dir: &Path,
    suffixes: &[(String, &str)],
    found: &mut HashMap<String, PathBuf>,
    depth: usize,
) {
    if depth > 4 || found.len() >= suffixes.len() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if found.len() >= suffixes.len() {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            find_rollouts(&entry.path(), suffixes, found, depth + 1);
        } else if kind.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("rollout-") {
                continue;
            }
            for (suffix, id) in suffixes {
                if name.ends_with(suffix.as_str()) {
                    found.insert((*id).to_string(), entry.path());
                    break;
                }
            }
        }
    }
}

// Codex locks even an empty lock file. Windows returns ERROR_LOCK_VIOLATION
// on a read while the writer holds it; file existence alone is not liveness.
fn writer_is_active(path: &Path) -> bool {
    let result = File::open(path).and_then(|mut f| f.read(&mut [0u8; 1]));
    matches!(result, Err(e) if e.raw_os_error() == Some(33))
}

fn writer_pid(path: &Path, is_codex: &impl Fn(u32) -> bool) -> u32 {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS},
            System::RestartManager::*,
        },
    };
    // Only query owners. Never ask Restart Manager to stop or restart anything.
    unsafe {
        let mut handle = 0;
        let mut key = [0u16; 33];
        if RmStartSession(&mut handle, 0, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
            return 0;
        }
        struct Session(u32);
        impl Drop for Session {
            fn drop(&mut self) {
                unsafe {
                    let _ = RmEndSession(self.0);
                }
            }
        }
        let _session = Session(handle);
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if RmRegisterResources(handle, Some(&[PCWSTR(name.as_ptr())]), None, None) != ERROR_SUCCESS
        {
            return 0;
        }
        let (mut needed, mut count, mut reasons) = (0, 0, 0);
        if RmGetList(handle, &mut needed, &mut count, None, &mut reasons) != ERROR_MORE_DATA {
            return 0;
        }
        for _ in 0..3 {
            let mut owners = vec![RM_PROCESS_INFO::default(); needed as usize];
            count = needed;
            let result = RmGetList(
                handle,
                &mut needed,
                &mut count,
                Some(owners.as_mut_ptr()),
                &mut reasons,
            );
            if result == ERROR_MORE_DATA {
                continue;
            }
            if result != ERROR_SUCCESS {
                return 0;
            }
            return owners
                .iter()
                .take(count as usize)
                .map(|p| p.Process.dwProcessId)
                .find(|pid| is_codex(*pid))
                .unwrap_or(0);
        }
        0
    }
}

impl CodexSessions {
    pub fn collect(&mut self, home: &Path, is_codex: impl Fn(u32) -> bool) -> Vec<SessionInfo> {
        self.collect_with(
            home,
            writer_is_active,
            |path| writer_pid(path, &is_codex),
            &is_codex,
        )
    }

    fn collect_with(
        &mut self,
        home: &Path,
        active: impl Fn(&Path) -> bool,
        owner: impl Fn(&Path) -> u32,
        alive: impl Fn(u32) -> bool,
    ) -> Vec<SessionInfo> {
        let locks = home.join("thread-writer-locks");
        let mut ids = Vec::new();
        if let Ok(files) = fs::read_dir(&locks) {
            for entry in files.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("lock") {
                    continue;
                }
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if id.len() == 36
                    && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                    && active(&path)
                {
                    ids.push(id);
                }
            }
        }
        self.entries.retain(|id, _| ids.contains(id));
        let missing: Vec<_> = ids
            .iter()
            .filter(|id| !self.entries.contains_key(*id))
            .cloned()
            .collect();
        let mut paths = HashMap::new();
        if !missing.is_empty() {
            let suffixes: Vec<(String, &str)> = missing
                .iter()
                .map(|id| (format!("-{id}.jsonl"), id.as_str()))
                .collect();
            find_rollouts(&home.join("sessions"), &suffixes, &mut paths, 0);
        }
        for (id, path) in paths {
            self.entries.insert(
                id,
                Rollout {
                    path,
                    offset: 0,
                    pid: 0,
                    pid_lookup_failed_at: None,
                    state: RolloutState::default(),
                },
            );
        }
        let mut result = Vec::new();
        for (id, rollout) in &mut self.entries {
            if rollout.pid == 0 || !alive(rollout.pid) {
                let should_retry = rollout.pid != 0
                    || rollout
                        .pid_lookup_failed_at
                        .is_none_or(|t| t.elapsed() >= PID_LOOKUP_BACKOFF);
                if should_retry {
                    rollout.pid = owner(&locks.join(format!("{id}.lock")));
                    rollout.pid_lookup_failed_at =
                        if rollout.pid == 0 { Some(Instant::now()) } else { None };
                }
            }
            rollout.refresh();
            let state = &rollout.state;
            if state.id != *id || state.cwd.is_empty() || state.child {
                continue;
            }
            result.push(SessionInfo {
                provider: "codex",
                pid: rollout.pid,
                session_id: id.clone(),
                name: format!("Codex {}", &id[..8]),
                cwd: state.cwd.clone(),
                status: state.status,
                activity: state.activity.clone(),
                status_updated_at: None,
                host_session_id: None,
                background_commands: 0,
            });
        }
        result.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::tempdir;

    const ID: &str = "01a08af3-a337-7af2-988c-1443812acb73";

    #[test]
    fn lifecycle_and_questions_do_not_confuse_commands_with_approvals() {
        let mut s = RolloutState::default();
        s.apply(json!({"type":"event_msg","payload":{"type":"task_started"}}));
        assert_eq!(s.status, SessionStatus::Working);
        s.apply(json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"a"}}));
        assert_eq!(s.status, SessionStatus::Working);
        s.apply(json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"q"}}));
        assert_eq!(s.status, SessionStatus::NeedsInput);
        s.apply(
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"a"}}),
        );
        assert_eq!(s.status, SessionStatus::NeedsInput);
        s.apply(
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"q"}}),
        );
        assert_eq!(s.status, SessionStatus::Working);
        s.apply(json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Continue?"}}));
        assert_eq!(s.status, SessionStatus::Waiting);
        assert_eq!(s.activity.as_deref(), Some("Ended with a question"));
        s.apply(json!({"type":"event_msg","payload":{"type":"task_started"}}));
        s.apply(json!({"type":"event_msg","payload":{"type":"turn_aborted"}}));
        assert_eq!(s.status, SessionStatus::Waiting);
        assert!(s.activity.is_none());
    }

    #[test]
    fn active_locks_only_and_incremental_partial_writes() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("thread-writer-locks")).unwrap();
        fs::write(
            root.path().join(format!("thread-writer-locks/{ID}.lock")),
            "",
        )
        .unwrap();
        let dir = root.path().join("sessions/2026/09/10");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-09-10T12-00-00-{ID}.jsonl"));
        let meta = json!({"type":"session_meta","payload":{"id":ID,"cwd":"C:\\Projects\\example","source":"cli"}});
        fs::write(&path, format!("{meta}\n{{broken}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n")).unwrap();
        let mut cache = CodexSessions::default();
        assert!(cache
            .collect_with(root.path(), |_| false, |_| 42, |_| true)
            .is_empty());
        let first = cache.collect_with(root.path(), |_| true, |_| 42, |_| true);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, SessionStatus::Working);
        assert_eq!(first[0].provider, "codex");
        assert_eq!(first[0].pid, 42);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, "{{\"type\":\"event_msg\",\"payload\":").unwrap();
        assert_eq!(
            cache.collect_with(root.path(), |_| true, |_| 42, |_| true)[0].status,
            SessionStatus::Working
        );
        writeln!(f, "{{\"type\":\"task_complete\"}}}}").unwrap();
        assert_eq!(
            cache.collect_with(root.path(), |_| true, |_| 42, |_| true)[0].status,
            SessionStatus::Waiting
        );
        fs::write(&path, format!("{meta}\n")).unwrap();
        assert_eq!(
            cache.collect_with(root.path(), |_| true, |_| 42, |_| true)[0].status,
            SessionStatus::Waiting
        );
        assert!(cache
            .collect_with(root.path(), |_| false, |_| 42, |_| true)
            .is_empty());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn missing_home_and_unlocked_files_are_not_sessions() {
        let root = tempdir().unwrap();
        assert!(CodexSessions::default()
            .collect(root.path(), |_| true)
            .is_empty());
        let path = root.path().join("old.lock");
        fs::write(&path, "").unwrap();
        assert!(!writer_is_active(&path));
        assert!(!writer_is_active(&root.path().join("missing.lock")));
    }

    #[test]
    fn subagents_are_identified_and_final_answer_is_idle() {
        let mut s = RolloutState::default();
        s.apply(
            json!({"type":"session_meta","payload":{"source":{"subagent":{"thread_spawn":{}}}}}),
        );
        assert!(s.child);
        s.apply(json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary"}}));
        s.apply(json!({"type":"event_msg","payload":{"type":"task_started"}}));
        s.apply(json!({"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"text":"Done."}]}}));
        assert_eq!(s.status, SessionStatus::Waiting);
    }

    #[test]
    #[ignore = "Read-only diagnostic: requires running local Codex sessions"]
    fn inspect_local_sessions() {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".codex"));
        let sys = sysinfo::System::new_all();
        let sessions = CodexSessions::default().collect(&home, |pid| {
            sys.process(sysinfo::Pid::from_u32(pid))
                .is_some_and(|p| p.name().eq_ignore_ascii_case("codex.exe"))
        });
        assert!(!sessions.is_empty(), "No active Codex writer locks found");
        assert!(
            sessions.iter().all(|s| s.pid > 0),
            "Could not resolve a Codex writer process"
        );
    }
}
