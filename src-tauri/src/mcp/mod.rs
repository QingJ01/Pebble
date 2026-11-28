//! Local MCP (#107, M4): Pebble listens on a private local socket (a named
//! pipe on Windows) only while the user has it turned on. MCP clients
//! reach it through the `--mcp` bridge.
//!
//! - **Pairing**: a new client asks to pair; Pebble asks the user, who
//!   picks the accounts and a level for each. Pebble then issues a token
//!   and keeps only its SHA-256 hash.
//! - **Every call** carries the token and is checked against the grant
//!   (see [`tools`]); calls are rate limited per client and recorded in
//!   the audit log without content.
//! - **Trust boundary**: any process of the same user can ask to pair, so
//!   every new pairing needs the user's approval in Pebble.

pub mod server;
pub mod tools;

use std::path::Path;

use sha2::{Digest, Sha256};

pub const PAIR_EVENT: &str = "mcp:pair-request";
pub const CHANGED_EVENT: &str = "mcp:changed";

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A fresh random token (256 bits).
pub fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("pmcp_{}", hex::encode(bytes))
}

fn short_hash(text: &str) -> String {
    hex::encode(&Sha256::digest(text.as_bytes())[..6])
}

/// Where Pebble listens for this profile. Unix socket paths must stay
/// short (about 100 bytes), so long data directories fall back to the
/// user's private temporary directory.
pub fn endpoint_for(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        format!(
            r"\\.\pipe\pebble-mcp-{}",
            short_hash(&data_dir.to_string_lossy())
        )
    }
    #[cfg(not(windows))]
    {
        let in_data = data_dir.join("mcp.sock");
        if in_data.to_string_lossy().len() < 100 {
            return in_data.to_string_lossy().into_owned();
        }
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!(
            "pebble-mcp-{}.sock",
            short_hash(&data_dir.to_string_lossy())
        ))
        .to_string_lossy()
        .into_owned()
    }
}

/// The command an MCP client should run.
pub fn client_command(endpoint: &str) -> (String, Vec<String>) {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "pebble".into());
    (
        exe,
        vec!["--mcp".into(), "--endpoint".into(), endpoint.to_string()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_only_hashes_are_compared() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert!(a.starts_with("pmcp_") && a.len() == 5 + 64);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), a);
    }

    #[test]
    fn endpoints_are_short_and_per_profile() {
        let short = endpoint_for(Path::new("/tmp/p1"));
        let long_dir = "/very/long".repeat(20);
        let long = endpoint_for(Path::new(&long_dir));
        assert_ne!(short, long);
        #[cfg(not(windows))]
        {
            assert_eq!(short, "/tmp/p1/mcp.sock");
            assert!(long.len() < 104, "{long}");
        }
        let (_, args) = client_command(&short);
        assert_eq!(args[0], "--mcp");
        assert_eq!(args[2], short);
    }
}
