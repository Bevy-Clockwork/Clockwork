//! Knowledge packs: notes about specific games, written by people or by earlier extractions.
//! They live in `knowledge/` of each content folder.

use std::path::PathBuf;

fn roots() -> Vec<PathBuf> {
    clockwork_library::content::knowledge_dirs()
        .into_iter()
        .filter(|d| d.is_dir())
        .collect()
}

/// All knowledge files, relative to their knowledge folder.
pub fn list() -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for root in roots() {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(rel) = path.strip_prefix(&root) {
                    files.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Reads a file from the first knowledge folder that has it.
pub fn read(file: &str) -> Result<String, String> {
    for root in roots() {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        let Ok(path) = root.join(file).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            return Err("Paths must stay inside the knowledge folder".into());
        }
        return std::fs::read_to_string(path).map_err(|e| e.to_string());
    }
    Err(format!("No knowledge file \"{file}\"; see list_knowledge"))
}
