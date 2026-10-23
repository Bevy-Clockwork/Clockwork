//! Decompilation projects linked to Library games. A decomp names the game's code and data,
//! which makes extraction much easier.

use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct KnownDecomp {
    pub name: String,
    pub repository: String,
    pub sha1: Vec<String>,
}

/// The file in a content folder's `knowledge/` that lists known decomps: name, repository and
/// the SHA-1 of every ROM version the project builds.
const FILE: &str = "decomps.ron";

/// Known decomps, from `knowledge/decomps.ron` of every content folder (read once).
pub fn known() -> &'static [KnownDecomp] {
    static KNOWN: OnceLock<Vec<KnownDecomp>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        clockwork_library::content::knowledge_dirs()
            .iter()
            .filter_map(|dir| std::fs::read_to_string(dir.join(FILE)).ok())
            .flat_map(|text| parse(&text))
            .collect()
    })
}

/// The decomps listed in a `decomps.ron`; none if it is invalid.
fn parse(text: &str) -> Vec<KnownDecomp> {
    ron::from_str(text).unwrap_or_default()
}

/// A known decomp for a ROM with this SHA-1.
pub fn suggest(sha1: &str) -> Option<&'static KnownDecomp> {
    find(known(), sha1)
}

fn find<'a>(known: &'a [KnownDecomp], sha1: &str) -> Option<&'a KnownDecomp> {
    known
        .iter()
        .find(|d| d.sha1.iter().any(|h| h.eq_ignore_ascii_case(sha1)))
}

/// Checks and tidies a repository URL the user typed.
pub fn normalize(input: &str) -> Result<String, String> {
    let url = input.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let rest = url
        .strip_prefix("https://")
        .ok_or("Enter an https:// link to the repository")?;
    let (host, path) = rest
        .split_once('/')
        .ok_or("The link needs a repository path")?;
    if host.is_empty()
        || !host.contains('.')
        || path.is_empty()
        || url.contains(char::is_whitespace)
    {
        return Err("That doesn't look like a repository link".into());
    }
    Ok(url.to_string())
}

/// "owner/repo" for display, or the whole URL if it has no such path.
pub fn short_name(url: &str) -> &str {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split_once('/'))
        .map_or(url, |(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_list_parses_and_matches() {
        let known = parse(
            r#"[(name: "Some Game", repository: "https://example.com/team/game",
                 sha1: ["aaaabbbbccccddddeeeeffff0000111122223333"])]"#,
        );
        let game = find(&known, "AAAABBBBCCCCDDDDEEEEFFFF0000111122223333").unwrap();
        assert_eq!(game.repository, "https://example.com/team/game");
        assert!(find(&known, "0000000000000000000000000000000000000000").is_none());
        assert!(parse("not ron").is_empty());
    }

    #[test]
    fn normalizes_links() {
        assert_eq!(
            normalize(" https://github.com/team/game.git/ ").unwrap(),
            "https://github.com/team/game"
        );
        assert!(normalize("github.com/team/game").is_err());
        assert!(normalize("https://github.com").is_err());
        assert_eq!(short_name("https://github.com/team/game"), "team/game");
    }
}
