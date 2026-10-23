//! External programs the agent can use (git, Ghidra, decompilers, …).
//!
//! They live in their own directory, one folder per tool: `<data>/tools/<tool>/`, or
//! `CLOCKWORK_TOOLS_DIR`. A tool found there wins over one on the system PATH, so a project can
//! pin the versions it uses.

use std::path::{Path, PathBuf};

use clockwork_library::store;

/// A program the agent knows how to use.
pub struct Known {
    pub name: &'static str,
    /// Executable names to look for (Windows extensions are tried automatically).
    pub executables: &'static [&'static str],
    pub purpose: &'static str,
}

pub const KNOWN: &[Known] = &[
    Known {
        name: "git",
        executables: &["git"],
        purpose: "Clones linked decompilation repositories (decomp_sync).",
    },
    Known {
        name: "ghidra",
        executables: &["analyzeHeadless", "support/analyzeHeadless"],
        purpose: "Disassembles and decompiles games without a decomp (not wired up yet).",
    },
];

/// Where external tools live.
pub fn tools_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLOCKWORK_TOOLS_DIR") {
        return dir.into();
    }
    data_dir().join("tools")
}

/// Clockwork's data folder (shared with the editor).
pub fn data_dir() -> PathBuf {
    store::data_dir().unwrap_or_else(|| std::env::temp_dir().join("clockwork"))
}

/// Finds a known tool: first in its folder under [`tools_dir`] (also one level deeper, for
/// archives that unpack into a versioned folder), then on PATH.
pub fn find(name: &str) -> Option<PathBuf> {
    let known = KNOWN.iter().find(|k| k.name == name)?;
    let folder = tools_dir().join(name);
    let mut roots = vec![folder.clone(), folder.join("bin")];
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                roots.push(entry.path());
                roots.push(entry.path().join("bin"));
            }
        }
    }
    for root in &roots {
        for exe in known.executables {
            if let Some(path) = executable(&root.join(exe)) {
                return Some(path);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .flat_map(|dir| known.executables.iter().map(move |exe| dir.join(exe)))
        .find_map(|candidate| executable(&candidate))
}

fn executable(path: &Path) -> Option<PathBuf> {
    let candidates = [
        path.to_path_buf(),
        path.with_extension("exe"),
        path.with_extension("bat"),
        path.with_extension("cmd"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}
