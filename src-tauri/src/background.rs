//! Background commands of a Claude Code session (fork).
//!
//! Claude Code runs shell commands (Bash/PowerShell tool) as child processes
//! of its own process. When the session is idle but such a shell is still
//! alive, a command keeps running in the background (dev server, watcher,
//! long build started with `run_in_background`). The widget shows that
//! instead of a plain "Idle".

use crate::usage::ProcSample;
use std::collections::HashMap;

/// Shell executables the Bash/PowerShell tools run in. `cmd.exe` is left out
/// on purpose: Claude Code starts `npx` MCP servers through `cmd /c`, which
/// would make every session with such a server look busy.
const SHELL_NAMES: &[&str] = &["bash", "bash.exe", "sh", "sh.exe", "zsh", "pwsh", "pwsh.exe", "powershell.exe"];

/// How far below the session process a shell may sit (Desktop wraps some
/// commands in an extra launcher process).
const MAX_DEPTH: usize = 4;

fn is_shell(name: &str) -> bool {
    SHELL_NAMES.iter().any(|shell| name.eq_ignore_ascii_case(shell))
}

/// Number of top-level shells running below `session_pid` (a shell started
/// by another counted shell is not counted again).
pub fn count_background_shells(procs: &[ProcSample], session_pid: u32) -> u32 {
    let by_pid: HashMap<u32, &ProcSample> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut count = 0;
    for proc in procs.iter().filter(|p| is_shell(&p.name)) {
        let mut current = proc.parent;
        for _ in 0..MAX_DEPTH {
            let Some(parent_pid) = current else { break };
            if parent_pid == session_pid {
                count += 1;
                break;
            }
            match by_pid.get(&parent_pid) {
                // Nested shell: its outer shell is the one that counts.
                Some(parent) if is_shell(&parent.name) => break,
                Some(parent) => current = parent.parent,
                None => break,
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, name: &str) -> ProcSample {
        ProcSample { pid, parent, name: name.into(), cpu_percent: 0.0, memory_bytes: 0, disk_bytes: 0 }
    }

    #[test]
    fn counts_shells_below_the_session() {
        let procs = vec![
            proc(10, None, "claude.exe"),
            proc(11, Some(10), "bash.exe"), // background dev server shell
            proc(12, Some(11), "node.exe"),
            proc(13, Some(10), "pwsh.exe"),
            proc(20, None, "bash.exe"), // someone else's shell
        ];
        assert_eq!(count_background_shells(&procs, 10), 2);
    }

    #[test]
    fn ignores_mcp_servers_and_nested_shells() {
        let procs = vec![
            proc(10, None, "claude.exe"),
            proc(11, Some(10), "cmd.exe"), // `cmd /c npx …` MCP server
            proc(12, Some(11), "node.exe"),
            proc(13, Some(10), "node.exe"), // MCP server started directly
            proc(14, Some(10), "BASH.EXE"),
            proc(15, Some(14), "bash.exe"), // subshell of 14
        ];
        assert_eq!(count_background_shells(&procs, 10), 1);
    }

    #[test]
    fn finds_shells_behind_a_launcher_but_not_too_deep() {
        let procs = vec![
            proc(10, None, "claude.exe"),
            proc(11, Some(10), "launcher.exe"),
            proc(12, Some(11), "bash.exe"),
            proc(21, Some(10), "a.exe"),
            proc(22, Some(21), "b.exe"),
            proc(23, Some(22), "c.exe"),
            proc(24, Some(23), "d.exe"),
            proc(25, Some(24), "bash.exe"),
        ];
        assert_eq!(count_background_shells(&procs, 10), 1);
    }
}
