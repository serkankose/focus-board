//! focus-board: a shared board of the few items that need attention now.
//!
//! The owner answers on a web page; every Claude session writes through a command. Each cell
//! belongs to one writer, every version of every cell is kept, and a write that names a stale
//! version is refused. See BRIEF.md (the requirement) and DESIGN.md (how).

pub mod hook;
pub mod identity;
pub mod store;

use std::path::PathBuf;

/// The board's database: `FOCUS_BOARD_DB`, else `$XDG_DATA_HOME/focus-board/board.sqlite`, else
/// `~/.local/share/focus-board/board.sqlite`. Always outside the source tree.
pub fn db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FOCUS_BOARD_DB").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local/share")))?;
    Some(data.join("focus-board").join("board.sqlite"))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}
