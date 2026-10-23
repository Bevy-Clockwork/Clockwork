//! Working with a game's linked decompilation: a local shallow clone in `<data>/decomps/`,
//! searched and read from disk (no API limits).

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::RegexBuilder;

use crate::external;

/// Files larger than this are skipped when searching.
const MAX_SEARCH_FILE: u64 = 4 * 1024 * 1024;
const TEXT_EXTENSIONS: &[&str] = &[
    "c", "h", "s", "inc", "json", "txt", "md", "ld", "mk", "py", "cpp", "hpp", "asm", "yaml",
    "yml", "toml", "csv", "xml", "sha1", "ron",
];

/// Where a repository's clone lives.
pub fn local_path(url: &str) -> PathBuf {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let name: String = url
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("__")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    external::data_dir().join("decomps").join(name)
}

/// Clones the repository (shallow) or updates an existing clone.
pub fn sync(url: &str) -> Result<String, String> {
    let git = external::find("git")
        .ok_or("git was not found in the tools folder or on PATH (see list_external_tools)")?;
    let path = local_path(url);
    let output = if path.join(".git").is_dir() {
        Command::new(&git)
            .arg("-C")
            .arg(&path)
            .args(["pull", "--ff-only", "--depth", "1"])
            .output()
    } else {
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        Command::new(&git)
            // `--` keeps a URL that starts with '-' from being read as an option.
            .args(["clone", "--depth", "1", "--", url])
            .arg(&path)
            .output()
    }
    .map_err(|e| format!("could not run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let files = walk(&path).len();
    Ok(format!(
        "{} is ready at {} ({files} files)",
        url,
        path.display()
    ))
}

/// A path inside the clone; refuses anything that would leave it.
fn inside(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "The decomp isn't cloned yet; run decomp_sync first".to_string())?;
    let path = root
        .join(relative.trim_start_matches(['/', '\\']))
        .canonicalize()
        .map_err(|_| format!("\"{relative}\" doesn't exist in the decomp"))?;
    if !path.starts_with(&root) {
        return Err("Paths must stay inside the decomp".into());
    }
    Ok(path)
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn relative(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Lists a directory of the clone.
pub fn list(root: &Path, dir: &str) -> Result<String, String> {
    let path = inside(root, dir)?;
    let mut entries: Vec<_> = std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_name() != ".git")
        .map(|e| {
            let is_dir = e.path().is_dir();
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            (is_dir, e.file_name().to_string_lossy().to_string(), size)
        })
        .collect();
    entries.sort();
    entries.sort_by_key(|(is_dir, _, _)| !is_dir);
    let mut out = format!("{}/\n", relative(root, &path));
    for (is_dir, name, size) in entries {
        if is_dir {
            let _ = writeln!(out, "  {name}/");
        } else {
            let _ = writeln!(out, "  {name}  ({size} bytes)");
        }
    }
    Ok(out)
}

/// Searches text files for a regular expression; `path_filter` limits to paths containing it.
pub fn search(root: &Path, pattern: &str, path_filter: &str, max: usize) -> Result<String, String> {
    let root = inside(root, "")?;
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("Invalid pattern: {e}"))?;
    let mut out = String::new();
    let mut hits = 0;
    for file in walk(&root) {
        let rel = relative(&root, &file);
        let is_text = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if !is_text
            || !rel.contains(path_filter)
            || file.metadata().map(|m| m.len()).unwrap_or(0) > MAX_SEARCH_FILE
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if regex.is_match(line) {
                let line = if line.len() > 200 {
                    &line[..line.floor_char_boundary(200)]
                } else {
                    line
                };
                let _ = writeln!(out, "{rel}:{}: {}", n + 1, line.trim());
                hits += 1;
                if hits >= max {
                    let _ = writeln!(out, "… stopped after {max} matches; narrow the search");
                    return Ok(out);
                }
            }
        }
    }
    if hits == 0 {
        out.push_str("No matches.");
    }
    Ok(out)
}

/// Reads lines `start..start + count` (1-based) of a file in the clone.
pub fn read(root: &Path, file: &str, start: usize, count: usize) -> Result<String, String> {
    let path = inside(root, file)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Can't read as text: {e}"))?;
    let total = text.lines().count();
    let start = start.max(1);
    let mut out = format!(
        "{} (lines {start}–{} of {total})\n",
        relative(root, &path),
        (start + count - 1).min(total)
    );
    for (n, line) in text.lines().enumerate().skip(start - 1).take(count) {
        let _ = writeln!(out, "{:>6}  {line}", n + 1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_folder_names() {
        let path = local_path("https://github.com/team/game");
        assert!(path.ends_with("decomps/team__game") || path.ends_with("decomps\\team__game"));
        // The same repository, however its URL is written, shares one clone.
        for same in [
            "https://github.com/team/game.git",
            "https://github.com/team/game/",
        ] {
            assert_eq!(local_path(same).file_name(), path.file_name(), "{same}");
        }
    }

    #[test]
    fn stays_inside_the_clone() {
        let root = std::env::temp_dir().join("clockwork-decomp-test");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.c"), "int gFoo = 1;\nint gBar = 2;\n").unwrap();
        assert!(read(&root, "src/a.c", 2, 5).unwrap().contains("gBar"));
        assert!(
            search(&root, "gfoo", "src", 10)
                .unwrap()
                .contains("src/a.c:1")
        );
        assert!(read(&root, "../outside.txt", 1, 1).is_err());
        assert!(list(&root, "").unwrap().contains("src/"));
    }
}
