//! Box art from [libretro-thumbnails](https://github.com/libretro-thumbnails).
//!
//! Thumbnails are named after the game's database name, so a game is first looked up in the
//! No-Intro (cartridge) or Redump (disc) database from [libretro-database](https://github.com/libretro/libretro-database):
//! by SHA-1, then by the header's serial (game code) for patched or unknown dumps. Databases are
//! downloaded once and cached. Only the ROM's hash and serial are used; the ROM never leaves the
//! machine.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::LibraryEntry;

const DAT_BASE: &str =
    "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat";
const THUMBNAIL_BASE: &str = "https://thumbnails.libretro.com";

/// The libretro system name for a Clockwork platform id.
pub fn system_name(platform: &str) -> Option<&'static str> {
    match platform {
        "gba" => Some("Nintendo - Game Boy Advance"),
        "n64" => Some("Nintendo - Nintendo 64"),
        "snes" => Some("Nintendo - Super Nintendo Entertainment System"),
        "nds" => Some("Nintendo - Nintendo DS"),
        "ps1" => Some("Sony - PlayStation"),
        "ps2" => Some("Sony - PlayStation 2"),
        _ => None,
    }
}

/// A parsed No-Intro or Redump database (clrmamepro DAT format).
#[derive(Debug, Default)]
pub struct Dat {
    by_sha1: HashMap<String, String>,
    by_serial: HashMap<String, String>,
}

impl Dat {
    pub fn parse(text: &str) -> Self {
        let mut dat = Self::default();
        let mut in_game = false;
        let mut name: Option<String> = None;

        for line in text.lines().map(str::trim) {
            if line.starts_with("game (") {
                in_game = true;
                name = None;
            } else if line == ")" {
                in_game = false;
            } else if in_game && name.is_none() && line.starts_with("name ") {
                name = quoted(line);
            } else if in_game && line.starts_with("rom (") {
                let Some(name) = &name else { continue };
                if let Some(sha1) = token_after(line, " sha1 ") {
                    dat.by_sha1.insert(sha1.to_ascii_lowercase(), name.clone());
                }
                if let Some(serial) = line.find(" serial ").and_then(|i| quoted(&line[i..])) {
                    dat.by_serial.entry(serial).or_insert_with(|| name.clone());
                }
            }
        }
        dat
    }

    pub fn len(&self) -> usize {
        self.by_sha1.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_sha1.is_empty()
    }

    /// The No-Intro name for a ROM: exact hash match first, then serial.
    pub fn lookup(&self, sha1: &str, serial: &str) -> Option<&str> {
        self.by_sha1
            .get(&sha1.to_ascii_lowercase())
            .or_else(|| {
                (!serial.is_empty())
                    .then(|| self.by_serial.get(serial))
                    .flatten()
            })
            .map(String::as_str)
    }
}

/// The first `"..."` string in `text`.
fn quoted(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let len = text[start..].find('"')?;
    Some(text[start..start + len].to_string())
}

fn token_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(key)? + key.len()..];
    rest.split_whitespace().next()
}

/// A readable title from a No-Intro name: drops the tags in parentheses, moves a trailing
/// article to the front and turns No-Intro's " - " back into a colon
/// ("Lost Kingdom, The - The Small Door (USA)" becomes "The Lost Kingdom: The Small Door").
pub fn display_name(no_intro: &str) -> String {
    let base = match no_intro.find(" (") {
        Some(i) => &no_intro[..i],
        None => no_intro,
    };
    let (main, rest) = match base.find(" - ") {
        Some(i) => (&base[..i], &base[i + 3..]),
        None => (base, ""),
    };
    let main = ["The", "A", "An"]
        .iter()
        .find_map(|article| {
            main.strip_suffix(&format!(", {article}"))
                .map(|stem| format!("{article} {stem}"))
        })
        .unwrap_or_else(|| main.to_string());
    if rest.is_empty() {
        main
    } else {
        format!("{main}: {}", rest.replace(" - ", ": "))
    }
}

/// libretro-thumbnails replaces these characters in file names.
pub fn thumbnail_name(game_name: &str) -> String {
    game_name
        .chars()
        .map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c })
        .collect()
}

pub fn dat_url(system: &str) -> String {
    let database = match system {
        "Sony - PlayStation" | "Sony - PlayStation 2" => "redump",
        _ => "no-intro",
    };
    format!("{DAT_BASE}/{database}/{}.dat", percent_encode(system))
}

pub fn boxart_url(system: &str, game_name: &str) -> String {
    format!(
        "{THUMBNAIL_BASE}/{}/Named_Boxarts/{}.png",
        percent_encode(system),
        percent_encode(&thumbnail_name(game_name))
    )
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Downloads box art, caching No-Intro/Redump databases in `cache_dir`.
pub struct BoxArtSource {
    cache_dir: PathBuf,
    agent: ureq::Agent,
    dats: HashMap<&'static str, Dat>,
}

impl BoxArtSource {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("Clockwork/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            cache_dir: cache_dir.into(),
            agent,
            dats: HashMap::new(),
        }
    }

    /// The game's No-Intro name, or `None` if it isn't in the database.
    pub fn identify(&mut self, entry: &LibraryEntry) -> Result<Option<String>, String> {
        let Some(system) = system_name(entry.info.platform) else {
            return Ok(None);
        };
        let dat = self.dat(system)?;
        Ok(dat
            .lookup(&entry.sha1, &entry.info.game_code)
            .map(str::to_string))
    }

    /// The box art image (PNG bytes) for a game with the given No-Intro name, or `None` if there
    /// is none.
    pub fn fetch(&mut self, entry: &LibraryEntry, name: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(system) = system_name(entry.info.platform) else {
            return Ok(None);
        };
        let url = boxart_url(system, name);
        match self.agent.get(&url).call() {
            Ok(mut response) => response
                .body_mut()
                .read_to_vec()
                .map(Some)
                .map_err(|e| format!("downloading box art failed: {e}")),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(format!("downloading box art failed: {e}")),
        }
    }

    fn dat(&mut self, system: &'static str) -> Result<&Dat, String> {
        if !self.dats.contains_key(system) {
            let path = self.cache_dir.join(format!("{system}.dat"));
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => {
                    let text = self
                        .agent
                        .get(&dat_url(system))
                        .call()
                        .and_then(|mut r| r.body_mut().read_to_string())
                        .map_err(|e| format!("downloading the {system} database failed: {e}"))?;
                    std::fs::create_dir_all(&self.cache_dir).map_err(|e| e.to_string())?;
                    std::fs::write(&path, &text).map_err(|e| e.to_string())?;
                    text
                }
            };
            self.dats.insert(system, Dat::parse(&text));
        }
        Ok(&self.dats[system])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"clrmamepro (
	name "Nintendo - Game Boy Advance"
	version "2026.10.07"
)

game (
	name "Lost Kingdom, The - The Small Door (USA)"
	region "USA"
	serial "ABCE"
	rom ( name "Lost Kingdom, The - The Small Door (USA).gba" size 16777216 crc 12345678 md5 0123456789ABCDEF0123456789ABCDEF sha1 AAAABBBBCCCCDDDDEEEEFFFF0000111122223333 serial "ABCE" )
)
game (
	name "Some Homebrew (World)"
	rom ( name "Some Homebrew (World).gba" size 1024 crc 00000000 md5 00 sha1 1111111111111111111111111111111111111111 )
)
"#;

    #[test]
    fn looks_up_by_hash_then_serial() {
        let dat = Dat::parse(SAMPLE);
        assert_eq!(dat.len(), 2);
        let kingdom = "Lost Kingdom, The - The Small Door (USA)";
        assert_eq!(
            dat.lookup("aaaabbbbccccddddeeeeffff0000111122223333", ""),
            Some(kingdom)
        );
        assert_eq!(
            dat.lookup("ffffffffffffffffffffffffffffffffffffffff", "ABCE"),
            Some(kingdom)
        );
        assert_eq!(
            dat.lookup("1111111111111111111111111111111111111111", ""),
            Some("Some Homebrew (World)")
        );
        assert_eq!(
            dat.lookup("ffffffffffffffffffffffffffffffffffffffff", ""),
            None
        );
    }

    #[test]
    fn readable_names() {
        assert_eq!(
            display_name("Lost Kingdom, The - The Small Door (USA)"),
            "The Lost Kingdom: The Small Door"
        );
        assert_eq!(display_name("Space Race 64 (USA)"), "Space Race 64");
        assert_eq!(
            display_name("101 - Night Patrol (USA, Europe) (En,Fr,De)"),
            "101: Night Patrol"
        );
        assert_eq!(display_name("Bird's Flight, A (USA)"), "A Bird's Flight");
    }

    #[test]
    fn builds_thumbnail_urls() {
        assert_eq!(
            boxart_url(
                "Nintendo - Game Boy Advance",
                "Lost Kingdom, The - The Small Door (USA)"
            ),
            "https://thumbnails.libretro.com/Nintendo%20-%20Game%20Boy%20Advance/Named_Boxarts/\
             Lost%20Kingdom%2C%20The%20-%20The%20Small%20Door%20%28USA%29.png"
        );
        assert_eq!(thumbnail_name("Q*bert: A/B"), "Q_bert_ A_B");
    }

    #[test]
    fn routes_console_databases_and_skips_pc() {
        for platform in ["gba", "n64", "snes", "nds"] {
            assert!(dat_url(system_name(platform).unwrap()).contains("/no-intro/"));
        }
        for platform in ["ps1", "ps2"] {
            assert!(dat_url(system_name(platform).unwrap()).contains("/redump/"));
        }
        assert!(system_name("pc").is_none());
    }
}
