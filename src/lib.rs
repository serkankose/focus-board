//! focus-board: a shared board of the few items that need attention now.
//!
//! The owner answers on a web page; every Claude session writes through a command. Each cell
//! belongs to one writer, every version of every cell is kept, and a write that names a stale
//! version is refused. See BRIEF.md (the requirement) and DESIGN.md (how).

pub mod identity;
pub mod store;
