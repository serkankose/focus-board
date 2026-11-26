//! focus-board: a shared board of the few items that need attention now.
//!
//! The owner answers on a web page; every Claude session writes through a command. Each cell
//! belongs to one writer, every version of every cell is kept, and a write that names a stale
//! version is refused. See BRIEF.md (the requirement) and DESIGN.md (how).

pub mod hook;
pub mod identity;
pub mod install;
pub mod live;
pub mod names;
pub mod notify;
pub mod server;
pub mod skill;
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

/// The board's port on this machine (owner, 2026-11-22: "to prevent clash we need to use
/// different ports for win/linux/linux(it could be running on wsl)/mac-os"). A Linux board inside
/// WSL shares `localhost` with the Windows one around it, so each platform has its own port.
pub fn default_port() -> u16 {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    port_for(std::env::consts::OS, &kernel)
}

/// The port for an OS (`std::env::consts::OS`) and, on Linux, its kernel release, which says
/// "microsoft" inside WSL (WSL 1: `4.4.0-19041-Microsoft`, WSL 2: `5.15.153.1-microsoft-standard-WSL2`).
pub fn port_for(os: &str, kernel_release: &str) -> u16 {
    match os {
        "windows" => 8812,
        "macos" => 8813,
        "linux" if kernel_release.to_ascii_lowercase().contains("microsoft") => 8811,
        _ => 8810,
    }
}

/// The page's address on this machine.
pub fn page_url() -> String {
    format!("http://127.0.0.1:{}", default_port())
}

/// The home folder: `HOME` where set (Linux, macOS, Git Bash on Windows), else the OS's own
/// (`%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
}

/// Whether a process still runs. On Linux from `/proc`, where a zombie (ended, not yet reaped)
/// counts as gone; elsewhere on Unix by `kill -0`. Where that cannot be told, it counts as running,
/// and the wait's time limit ends it.
pub fn process_alive(pid: u32) -> bool {
    if !cfg!(unix) {
        return true;
    }
    if cfg!(target_os = "linux") {
        return std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .is_some_and(|state| state != "Z")
        });
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(true)
}
