//! Connects the Editor to Claude Code Desktop: the Library's Extract panel opens a new Claude
//! Code session with the extraction prompt filled in, in the Clockwork folder (whose `.mcp.json`
//! offers the `clockwork` tools).
//!
//! This uses Claude Desktop's `claude://code/new` link. The link only prefills the composer, so
//! the user presses Send themselves; Claude Desktop also asks to confirm the folder.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Claude Desktop truncates the prompt at about 14,000 characters; stay below.
const MAX_PROMPT: usize = 12_000;

/// Whether Claude Desktop looks installed, i.e. something handles `claude://` links. Only a hint:
/// the check can be wrong, so callers must not block on it.
pub fn installed() -> bool {
    if cfg!(windows) {
        return Command::new("reg")
            .args(["query", r"HKCR\claude\shell\open\command"])
            .output()
            .is_ok_and(|o| o.status.success());
    }
    // Other systems can't be probed cheaply; opening the link reports failure instead.
    true
}

/// Prefer the portable release's MCP setup beside the executable. A normal Cargo build uses
/// the source workspace, whose MCP setup runs the agent through Cargo.
pub fn workspace_dir() -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."));
    std::env::current_exe().map_or_else(
        |_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        |exe| workspace_for(&exe, source),
    )
}

fn workspace_for(exe: &Path, source: &Path) -> PathBuf {
    let beside = exe.parent().unwrap_or_else(|| Path::new("."));
    if beside.join(".mcp.json").is_file() {
        return beside.to_path_buf();
    }
    if exe.starts_with(source.join("target")) && source.join(".mcp.json").is_file() {
        return source.to_path_buf();
    }
    // A relocated binary must never open a stale path from its build machine.
    beside.to_path_buf()
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn link(prompt: &str) -> String {
    let prompt: String = prompt.chars().take(MAX_PROMPT).collect();
    format!(
        "claude://code/new?q={}&folder={}",
        encode(&prompt),
        encode(&workspace_dir().to_string_lossy())
    )
}

/// Opens Claude Desktop with a new Code session holding `prompt`.
pub fn open_session(prompt: &str) -> Result<(), String> {
    let url = link(prompt);
    let result = if cfg!(windows) {
        // Not through a shell, so `&` in the link needs no quoting.
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&url).spawn()
    } else {
        Command::new("xdg-open").arg(&url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| format!("Could not open Claude Desktop: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_and_source_builds_use_their_own_mcp_workspace() {
        let root = std::env::temp_dir().join(format!(
            "clockwork-mcp-workspace-test-{}",
            std::process::id()
        ));
        let source = root.join("checkout");
        let portable = source.join("dist").join("Clockwork");
        let relocated = root.join("relocated");
        std::fs::create_dir_all(&portable).unwrap();
        std::fs::write(source.join(".mcp.json"), "{}").unwrap();
        std::fs::write(portable.join(".mcp.json"), "{}").unwrap();

        // The packaged config wins even when the build checkout still exists.
        assert_eq!(
            workspace_for(&portable.join("clockwork.exe"), &source),
            portable
        );
        assert_eq!(
            workspace_for(&source.join("target/debug/clockwork.exe"), &source),
            source
        );
        // Moving only the binary also avoids opening the old checkout.
        assert_eq!(
            workspace_for(&relocated.join("clockwork.exe"), &source),
            relocated
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn link_encodes_prompt_and_folder() {
        let url = link("Extract the hero & jump");
        assert!(url.starts_with("claude://code/new?q=Extract%20the%20hero%20%26%20jump&folder="));
        assert!(!url[..url.find("&folder").unwrap()].contains(' '));
    }

    #[test]
    fn long_prompts_are_cut() {
        assert!(link(&"a".repeat(50_000)).len() < 13_000 + 400);
    }
}
