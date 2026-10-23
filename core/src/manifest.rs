//! The plugin manifest, `clockwork.toml` (DESIGN.md §5.1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    pub source: SourceIdentity,
    #[serde(default)]
    pub provides: Capabilities,
    #[serde(default)]
    pub requires: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    /// The `core` version this plugin targets.
    pub core: String,
}

/// Which game, and which exact ROM revisions, the plugin's recipe applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub game: String,
    /// Lowercase hex SHA-1 hashes of the supported ROM revisions.
    pub sha1: Vec<String>,
}

/// What a plugin provides or requires, grouped by vocabulary area.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub character: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("invalid clockwork.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

impl PluginManifest {
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        Ok(toml::from_str(text)?)
    }

    /// Whether this plugin's recipe applies to a ROM with the given SHA-1.
    pub fn supports_rom(&self, sha1: &str) -> bool {
        self.source
            .sha1
            .iter()
            .any(|h| h.eq_ignore_ascii_case(sha1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        [plugin]
        name = "game-a-magic-cane"
        version = "0.1.0"
        core = "0.1"

        [source]
        game = "first_game"
        sha1 = ["0123456789abcdef0123456789abcdef01234567"]

        [provides]
        abilities = ["Flip"]

        [requires]
        surfaces = ["FallThrough"]
        character = ["Jump"]
    "#;

    #[test]
    fn parses_design_doc_example() {
        let m = PluginManifest::from_toml(EXAMPLE).unwrap();
        assert_eq!(m.plugin.name, "game-a-magic-cane");
        assert_eq!(m.provides.abilities, ["Flip"]);
        assert_eq!(m.requires.surfaces, ["FallThrough"]);
        assert!(m.supports_rom("0123456789ABCDEF0123456789ABCDEF01234567"));
        assert!(!m.supports_rom("ffffffffffffffffffffffffffffffffffffffff"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = EXAMPLE.replace("[provides]", "[provides]\nweapons = [\"Sword\"]");
        assert!(PluginManifest::from_toml(&text).is_err());
    }
}
