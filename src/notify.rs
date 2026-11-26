//! Desktop notifications for the owner (owner, 2026-11-26, B45 "a"): the server tells the owner
//! when a session raises an item, and when a review press is still unseen by some sessions a while
//! later. It runs a program, never waits on it, and never fails because of it.

use std::process::{Command, Stdio};
use std::time::Duration;

/// How the owner is told.
#[derive(Debug, Clone)]
pub struct Notifier {
    /// `notify-send` on Linux, `osascript` on macOS, else any program run as `PROGRAM TITLE BODY`.
    pub program: String,
    /// How long a review press may stay unseen before the owner is told who has not seen it.
    pub unseen_after: Duration,
}

impl Notifier {
    /// From `FOCUS_BOARD_NOTIFY`: a program, or `off`; unset, the OS's own (ten minutes for an
    /// unseen press).
    pub fn from_env() -> Option<Notifier> {
        let program = std::env::var("FOCUS_BOARD_NOTIFY")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "osascript".into()
                } else {
                    "notify-send".into()
                }
            });
        (program != "off").then(|| Notifier {
            program,
            unseen_after: Duration::from_secs(600),
        })
    }

    /// Shows one notification. A missing program or a failure is ignored; the child is reaped on
    /// a thread of its own, so the board never waits.
    pub fn send(&self, title: &str, body: &str) {
        let name = self.program.rsplit(['/', '\\']).next().unwrap_or("");
        let args: Vec<String> = match name {
            // Stays until the owner closes it (owner, 2026-11-26: the first test vanished too quickly).
            "notify-send" => vec![
                "--app-name=focus-board".into(),
                "--expire-time=0".into(),
                title.into(),
                body.into(),
            ],
            "osascript" => vec![
                "-e".into(),
                format!(
                    "display notification {} with title {}",
                    apple_text(body),
                    apple_text(title)
                ),
            ],
            _ => vec![title.into(), body.into()],
        };
        if let Ok(mut child) = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

/// A string literal for AppleScript.
fn apple_text(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// At most `max` characters of `text`, with an ellipsis when cut.
pub fn shorten(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}
