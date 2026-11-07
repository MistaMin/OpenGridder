//! AudioGridder's clipboard bridge.
//!
//! `Clipboard` from the client replaces the macOS pasteboard. While an editor
//! is open, the worker reads that pasteboard and pushes a change back. This
//! uses `pbcopy` and `pbpaste`, which are the same pasteboard
//! `SystemClipboard` writes.

use std::io::{Error, ErrorKind, Write};
use std::process::{Command, Stdio};

pub fn set_text(text: &str) -> std::io::Result<()> {
    let mut child = Command::new("pbcopy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::Other, "pbcopy"))
    }
}

pub fn get_text() -> std::io::Result<String> {
    let output = Command::new("pbpaste").stdin(Stdio::null()).output()?;
    if !output.status.success() {
        return Err(Error::new(ErrorKind::Other, "pbpaste"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Main display size, in pixels. AudioGridder's fullscreen capture uses the
/// display rectangle. `(0, 0)` means the query failed.
pub fn main_display() -> (i32, i32) {
    unsafe {
        let id = CGMainDisplayID();
        let width = CGDisplayPixelsWide(id) as i32;
        let height = CGDisplayPixelsHigh(id) as i32;
        if width > 0 && height > 0 {
            (width, height)
        } else {
            (0, 0)
        }
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
}
