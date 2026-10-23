//! Resolve a CUE's first binary data track. Audio files are never mistaken for game images.

use std::path::Path;

use crate::ImportError;

pub fn read(path: &Path) -> Result<Vec<u8>, ImportError> {
    let fail = |message: String| ImportError::Cue {
        path: path.to_path_buf(),
        message,
    };
    let text = std::fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
    let (name, offset) = data_file(&text).map_err(fail)?;
    let file = path.parent().unwrap_or(Path::new(".")).join(name);
    let io = |source| ImportError::Io {
        path: file.clone(),
        source,
    };
    let mut data = std::fs::File::open(&file).map_err(io)?;
    let len = data.metadata().map_err(io)?.len();
    if offset as u64 >= len {
        return Err(fail("data track starts beyond the end of its file".into()));
    }
    // Read from the track's start rather than reading everything and shifting it down.
    // Keep raw sectors (including XA data) intact; recipes use offsets into these bytes.
    use std::io::{Read, Seek, SeekFrom};
    data.seek(SeekFrom::Start(offset as u64)).map_err(io)?;
    let mut bytes = Vec::with_capacity((len - offset as u64) as usize);
    data.read_to_end(&mut bytes).map_err(io)?;
    Ok(bytes)
}

fn data_file(text: &str) -> Result<(String, usize), String> {
    let mut file = None;
    let mut sector_size = None;
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else {
            continue;
        };
        match command.to_ascii_uppercase().as_str() {
            "FILE" => {
                if sector_size.is_some() {
                    return Err("data track is missing INDEX 01".into());
                }
                let rest = line[command.len()..].trim();
                let (name, kind) = if let Some(rest) = rest.strip_prefix('"') {
                    let end = rest.find('"').ok_or("unterminated FILE name")?;
                    (&rest[..end], rest[end + 1..].trim())
                } else {
                    let end = rest.find(char::is_whitespace).ok_or("missing FILE type")?;
                    (&rest[..end], rest[end..].trim())
                };
                file = Some((name.to_string(), kind.eq_ignore_ascii_case("BINARY")));
            }
            "TRACK" => {
                if sector_size.is_some() {
                    return Err("data track is missing INDEX 01".into());
                }
                let _number = words.next().ok_or("missing track number")?;
                let mode = words
                    .next()
                    .ok_or("missing track mode")?
                    .to_ascii_uppercase();
                sector_size = match mode.as_str() {
                    "AUDIO" => None,
                    "MODE1/2048" => Some(2048usize),
                    "MODE1/2352" | "MODE2/2352" => Some(2352usize),
                    _ => return Err(format!("unsupported data track mode {mode}")),
                };
                if sector_size.is_some() && !file.as_ref().is_some_and(|(_, binary)| *binary) {
                    return Err("data track needs a BINARY FILE".into());
                }
            }
            "INDEX" if sector_size.is_some() => {
                if words.next() != Some("01") {
                    continue;
                }
                let time = words.next().ok_or("missing INDEX time")?;
                let parts = time
                    .split(':')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "invalid INDEX time")?;
                let [minutes, seconds, frames] = parts.as_slice() else {
                    return Err("INDEX time must be mm:ss:ff".into());
                };
                if *seconds >= 60 || *frames >= 75 {
                    return Err("INDEX seconds or frames out of range".into());
                }
                let offset = minutes
                    .checked_mul(60)
                    .and_then(|n| n.checked_add(*seconds))
                    .and_then(|n| n.checked_mul(75))
                    .and_then(|n| n.checked_add(*frames))
                    .and_then(|n| n.checked_mul(sector_size.unwrap()))
                    .ok_or("INDEX time is too large")?;
                let (name, _) = file.ok_or("missing FILE")?;
                // CUE filenames use Windows separators even when read on another host.
                return Ok((name.replace('\\', "/"), offset));
            }
            _ => {}
        }
    }
    Err("no supported data track with INDEX 01".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_quoted_data_file_and_pregap() {
        assert_eq!(data_file("FILE \"Game (Track 1).bin\" BINARY\n TRACK 01 MODE2/2352\n INDEX 00 00:00:00\n INDEX 01 00:02:00").unwrap(), ("Game (Track 1).bin".into(), 150 * 2352));
    }

    #[test]
    fn skips_audio_files_and_accepts_case_insensitive_keywords() {
        assert_eq!(data_file("FILE audio.wav WAVE\n TRACK 01 AUDIO\n INDEX 01 00:00:00\n file data.bin BINARY\n track 02 MODE1/2048\n index 01 00:00:00").unwrap(), ("data.bin".into(), 0));
    }

    #[test]
    fn rejects_unsupported_and_malformed_cues() {
        for text in [
            "FILE audio.wav WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00",
            "FILE data.bin BINARY\nTRACK 01 MODE2/2336\nINDEX 01 00:00:00",
            "FILE data.bin BINARY\nTRACK 01 MODE2/2352\nINDEX 01 00:00:75",
            "FILE data.bin BINARY\nTRACK 01 MODE2/2352\nTRACK 02 AUDIO\nINDEX 01 00:00:00",
            "FILE \"unterminated BINARY\nTRACK 01 MODE1/2048\nINDEX 01 00:00:00",
        ] {
            assert!(data_file(text).is_err(), "{text}");
        }
    }
}
