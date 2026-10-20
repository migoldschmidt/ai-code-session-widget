use crate::sessions::SessionFile;
use sysinfo::{Pid, System};

/// Pid check alone (upstream); the session list uses `is_session_alive`.
#[allow(dead_code)]
pub fn is_pid_alive(sys: &System, pid: u32) -> bool {
    sys.process(Pid::from_u32(pid)).is_some()
}

/// Seconds the recorded and the actual process start may differ (rounding,
/// clock resolution).
const START_TOLERANCE_SECS: u64 = 5;

/// Fork: the pid lives AND, if the session file recorded when its process
/// started (`procStart`), the process under that pid started then too.
/// Otherwise the pid was handed to another program after the session ended
/// without cleaning up (crash, reboot), and the session is gone.
pub fn is_session_alive(sys: &System, pid: u32, proc_start: Option<u64>) -> bool {
    match sys.process(Pid::from_u32(pid)) {
        None => false,
        Some(process) => proc_start.map_or(true, |recorded| same_start(recorded, process.start_time())),
    }
}

fn same_start(recorded: u64, actual: u64) -> bool {
    // sysinfo reports 0 when it can't read the start time: don't hide the session then.
    actual == 0 || recorded.abs_diff(actual) <= START_TOLERANCE_SECS
}

pub fn filter_alive(
    sessions: Vec<SessionFile>,
    is_alive: impl Fn(u32, Option<u64>) -> bool,
) -> Vec<SessionFile> {
    sessions.into_iter().filter(|s| is_alive(s.pid, s.proc_start)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(pid: u32) -> SessionFile {
        SessionFile {
            pid,
            session_id: "s".into(),
            cwd: "C:\\x".into(),
            name: "n".into(),
            hook_status: None,
            status_updated_at: None,
            host_session_id: None,
            proc_start: None,
        }
    }

    #[test]
    fn filter_alive_keeps_only_pids_reported_alive() {
        let sessions = vec![make_session(1), make_session(2), make_session(3)];

        let result = filter_alive(sessions, |pid, _| pid == 2);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 2);
    }

    #[test]
    fn filter_alive_returns_empty_when_none_alive() {
        let sessions = vec![make_session(1), make_session(2)];

        let result = filter_alive(sessions, |_, _| false);

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn current_process_pid_is_reported_alive() {
        let mut sys = System::new_all();
        sys.refresh_all();
        let current_pid = std::process::id();

        assert!(is_pid_alive(&sys, current_pid));
    }

    #[test]
    fn current_process_with_its_real_start_time_is_alive() {
        let mut sys = System::new_all();
        sys.refresh_all();
        let pid = std::process::id();
        let started = sys.process(Pid::from_u32(pid)).expect("own process").start_time();

        assert!(is_session_alive(&sys, pid, Some(started)));
        assert!(is_session_alive(&sys, pid, None));
    }

    #[test]
    fn reused_pid_with_another_start_time_is_not_alive() {
        let mut sys = System::new_all();
        sys.refresh_all();
        let pid = std::process::id();
        let started = sys.process(Pid::from_u32(pid)).expect("own process").start_time();

        // The session's process started an hour before the one now holding the pid.
        assert!(!is_session_alive(&sys, pid, Some(started - 3600)));
    }

    #[test]
    fn start_times_match_within_tolerance() {
        assert!(same_start(1000, 1003));
        assert!(!same_start(1000, 1010));
        assert!(same_start(1000, 0));
    }

    #[test]
    fn implausible_pid_is_reported_not_alive() {
        let mut sys = System::new_all();
        sys.refresh_all();

        assert!(!is_pid_alive(&sys, u32::MAX));
    }
}
