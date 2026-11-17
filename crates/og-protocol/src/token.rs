//! Shared secret in the handshake (bytes 56..64). AudioGridder never sent one,
//! so this port rejects any client that does not know the server's token.
//!
//! The server creates the token file once. A client on the same Mac reads the
//! same file. A client on another Mac copies the file, or sets `OG_TOKEN`.

use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/OpenGridder/token"))
}

fn parse(text: &str) -> Option<u64> {
    u64::from_str_radix(text.trim(), 16).ok().filter(|token| *token != 0)
}

/// The token a client sends: `OG_TOKEN`, else the token file, else 0 (always refused).
pub fn load() -> u64 {
    if let Some(token) = std::env::var("OG_TOKEN").ok().as_deref().and_then(parse) {
        return token;
    }
    path().and_then(|p| std::fs::read_to_string(p).ok()).as_deref().and_then(parse).unwrap_or(0)
}

/// The token the server requires. Creates a random one (file mode 0600) on first run.
pub fn load_or_create() -> std::io::Result<u64> {
    let token = load();
    if token != 0 {
        return Ok(token);
    }
    let file = path().ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token = u64::from_le_bytes(bytes) | 1;
    let mut out = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(file)?;
    writeln!(out, "{token:016x}")?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn zero_and_junk_are_not_tokens() {
        assert_eq!(parse("00000000000000ff\n"), Some(255));
        assert_eq!(parse("0"), None);
        assert_eq!(parse("not hex"), None);
    }
}
