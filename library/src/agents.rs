//! Running agent servers announce themselves here so the editor can show them.
//!
//! Each `clockwork-mcp` process keeps `<data>/agents/<pid>.ron` up to date while it runs:
//! refreshed every few seconds, updated when a client connects or calls a tool, removed on exit.
//! A file that hasn't been refreshed for a while belongs to a server that has gone away.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::store;

/// How often servers refresh their file.
pub const HEARTBEAT: Duration = Duration::from_secs(3);
/// After this long without a refresh a server counts as gone.
pub const STALE: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub pid: u32,
    /// `"stdio"` or `"http <address>"`.
    pub transport: String,
    /// The connected client's name and version, from the MCP handshake.
    #[serde(default)]
    pub client: Option<String>,
    /// Unix seconds.
    pub started: u64,
    /// Unix seconds of the last refresh.
    pub last_seen: u64,
    #[serde(default)]
    pub tool_calls: u64,
    /// The latest tool called, e.g. "run_recipe".
    #[serde(default)]
    pub last_tool: Option<String>,
    /// Unix seconds of the latest tool call.
    #[serde(default)]
    pub last_tool_at: Option<u64>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn dir() -> Option<PathBuf> {
    store::data_dir().map(|d| d.join("agents"))
}

/// Writes a status file atomically (write, then rename), so readers never see half a file.
pub fn write(path: &Path, status: &AgentStatus) {
    let Ok(text) = ron::ser::to_string_pretty(status, Default::default()) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Agent servers that refreshed their file recently, newest first. Files of servers that
/// stopped without cleaning up are removed.
pub fn running() -> Vec<AgentStatus> {
    let Some(entries) = dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    let now = now();
    let mut found: Vec<AgentStatus> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ron"))
        .filter_map(|e| {
            let status: AgentStatus =
                ron::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            if now.saturating_sub(status.last_seen) > STALE.as_secs() {
                let _ = std::fs::remove_file(e.path());
                return None;
            }
            Some(status)
        })
        .collect();
    found.sort_by_key(|s| std::cmp::Reverse(s.started));
    found
}
