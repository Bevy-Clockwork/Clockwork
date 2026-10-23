//! The extraction agent's tools, served over MCP (DESIGN.md §6).
//!
//! Every tool is game-agnostic: game knowledge comes from the ROM, the game's linked decomp,
//! knowledge packs and existing plugins, never from this crate.

pub mod external;
pub mod guide;
pub mod presence;
pub mod server;
pub mod tools;
pub mod workspace;
