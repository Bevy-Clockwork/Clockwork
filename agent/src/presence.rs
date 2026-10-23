//! Announces this server to the editor (see `clockwork_library::agents`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clockwork_library::agents::{self, AgentStatus, HEARTBEAT};

#[derive(Clone)]
pub struct Presence {
    status: Arc<Mutex<AgentStatus>>,
    path: Option<PathBuf>,
}

impl Presence {
    /// Starts announcing this process and keeps refreshing in the background.
    pub fn start(transport: impl Into<String>) -> Self {
        let pid = std::process::id();
        let presence = Self {
            status: Arc::new(Mutex::new(AgentStatus {
                pid,
                transport: transport.into(),
                client: None,
                started: agents::now(),
                last_seen: agents::now(),
                tool_calls: 0,
                last_tool: None,
                last_tool_at: None,
            })),
            path: agents::dir().map(|d| d.join(format!("{pid}.ron"))),
        };
        presence.write();
        let heartbeat = presence.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(HEARTBEAT);
                heartbeat.write();
            }
        });
        presence
    }

    pub fn set_client(&self, client: String) {
        self.status.lock().unwrap().client = Some(client);
        self.write();
    }

    pub fn tool_called(&self, tool: &str) {
        {
            let mut status = self.status.lock().unwrap();
            status.tool_calls += 1;
            status.last_tool = Some(tool.to_string());
            status.last_tool_at = Some(agents::now());
        }
        self.write();
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut status = self.status.lock().unwrap();
        status.last_seen = agents::now();
        agents::write(path, &status);
    }

    /// Removes the file; call when the server stops.
    pub fn stop(&self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}
