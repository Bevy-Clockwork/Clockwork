//! Imports ROMs from a folder and prints what the Library makes of them.
//!
//! cargo run -p clockwork_library --example scan -- <folder> [name filter]

use clockwork_library::{Library, ROM_EXTENSIONS, boxart::BoxArtSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = args.next().expect("usage: scan <folder> [name filter]");
    let filter = args.next().unwrap_or_default().to_lowercase();

    let mut library = Library::new().with_platforms(clockwork_platforms::all());
    let mut names = BoxArtSource::new(std::env::temp_dir().join("clockwork-scan-databases"));

    let mut paths: Vec<_> = std::fs::read_dir(&folder)
        .expect("could not read folder")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            (ext == "zip" || ROM_EXTENSIONS.contains(&ext.as_str()))
                && p.to_string_lossy().to_lowercase().contains(&filter)
        })
        .collect();
    paths.sort();

    for path in paths {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        match library.import(&path) {
            Ok(entry) => {
                let entry = entry.clone();
                let name = match names.identify(&entry) {
                    Ok(Some(name)) => name,
                    Ok(None) => "(not in database)".into(),
                    Err(e) => format!("(lookup failed: {e})"),
                };
                println!(
                    "{file}\n    {} {} {:?} sha1 {}\n    -> {name}",
                    entry.info.platform, entry.info.game_code, entry.info.title, entry.sha1
                );
            }
            Err(e) => println!("{file}\n    error: {e}"),
        }
    }
}
