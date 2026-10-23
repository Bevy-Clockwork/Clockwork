//! The platform modules Clockwork ships with, so the Library, the editor and the agent all
//! recognize the same game formats. Add a new platform module here.

use clockwork_core::platform::Platform;

/// Every platform module, in the order ROMs are tried against them.
pub fn all() -> Vec<Box<dyn Platform>> {
    vec![
        Box::new(clockwork_gba::Gba),
        Box::new(clockwork_n64::N64),
        Box::new(clockwork_nds::Nds),
        Box::new(clockwork_playstation::Ps1),
        Box::new(clockwork_playstation::Ps2),
        Box::new(clockwork_pc::Pc),
        Box::new(clockwork_snes::Snes),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let ids: Vec<_> = all().iter().map(|p| p.id()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(ids.len(), unique.len());
        assert_eq!(unique, ["gba", "n64", "nds", "pc", "ps1", "ps2", "snes"]);
    }
}
