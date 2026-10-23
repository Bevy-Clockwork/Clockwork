//! The interface every platform module implements (DESIGN.md §4.1).

/// What a platform module could read from a game image or executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomInfo {
    /// The platform's id, e.g. `gba` or `n64`.
    pub platform: &'static str,
    /// Internal title from the header, trimmed. May be empty when unavailable.
    pub title: String,
    /// Internal game code from the header, e.g. `BZME`.
    pub game_code: String,
}

/// A console or computer platform Clockwork can read games from.
pub trait Platform: Send + Sync {
    /// Short, stable id, e.g. `gba`.
    fn id(&self) -> &'static str;

    /// Returns header info if `rom` looks like a ROM for this platform, otherwise `None`.
    fn identify(&self, rom: &[u8]) -> Option<RomInfo>;

    /// Converts a dump to the platform's canonical form (e.g. N64 byte order), so the same game
    /// always has the same hash. Returns `None` if `rom` already is canonical.
    fn canonicalize(&self, _rom: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

/// Decodes a fixed-width header text field: stops at the first NUL and trims padding.
pub fn header_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}
