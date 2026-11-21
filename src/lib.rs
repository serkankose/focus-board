//! focus-board: a shared board of the few items that need attention now.
//!
//! The owner answers on a web page; every Claude session writes through a command. Each cell
//! belongs to one writer, every version of every cell is kept, and a write that names a stale
//! version is refused. See BRIEF.md (the requirement) and DESIGN.md (how).

pub mod hook;
pub mod identity;
pub mod names;
pub mod server;
pub mod store;

use std::path::PathBuf;

/// The board's database: `FOCUS_BOARD_DB`, else `board.sqlite` in a `focus-board` folder in the
/// OS's data folder (review P1): `$XDG_DATA_HOME` or `~/.local/share` on Linux,
/// `~/Library/Application Support` on macOS, `%APPDATA%` on Windows. Always outside the source tree.
pub fn db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FOCUS_BOARD_DB").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let base = directories::BaseDirs::new()?;
    Some(base.data_dir().join("focus-board").join("board.sqlite"))
}

/// The home folder: `HOME` where set (Linux, macOS, Git Bash on Windows), else the OS's own
/// (`%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
}
