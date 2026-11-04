//! Child processes and HTTP for the native app.
//!
//! The import pipeline is Node (see `dist-tools/`), so the app spawns Node
//! for uploads, metadata and cover previews. Everything else that touches
//! the network goes through [`http_get_text`], which needs no external tool.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The Node binary: `KARAOKEMACHINE_NODE` if set, otherwise `node` on PATH.
pub fn node_binary() -> PathBuf {
    std::env::var_os("KARAOKEMACHINE_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("node"))
}

/// A command that runs one of the bundled tools in `dist-tools/` from the
/// project root, without a console window on Windows.
pub fn node_command(root: &Path, script: &Path) -> Command {
    let mut command = Command::new(node_binary());
    command.arg(script).current_dir(root);
    hide_console(&mut command);
    command
}

/// The release exe is built without a console (`windows_subsystem =
/// "windows"`), so a child process would otherwise open one over the game.
pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Fetch `url` as UTF-8 text; see [`http_get_bytes`] for the limits.
pub fn http_get_text(url: &str, max_bytes: u64) -> Result<String, String> {
    let bytes = http_get_bytes(url, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| format!("{url} is not UTF-8 text"))
}

/// Fetch `url`. Fails after 10 seconds without a connection, 30 seconds
/// without a byte, or when the body exceeds `max_bytes`, so a stalled or
/// oversized download cannot hang a job or exhaust memory.
pub fn http_get_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent("KaraokeMachine/2.0")
        .build();
    let response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::Status(code, _) => format!("HTTP {code} from {url}"),
        ureq::Error::Transport(transport) => format!("Could not download {url}: {transport}"),
    })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {url}: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "{url} is larger than the {} MB limit",
            max_bytes / (1024 * 1024)
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_urls_fail_without_a_network() {
        assert!(http_get_text("not a url", 1024).is_err());
        assert!(http_get_text("ftp://example.invalid/index.json", 1024).is_err());
    }

    #[test]
    fn node_command_targets_the_script() {
        let command = node_command(Path::new("root"), Path::new("dist-tools/song-info.js"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, vec![std::ffi::OsStr::new("dist-tools/song-info.js")]);
        assert_eq!(command.get_current_dir(), Some(Path::new("root")));
    }
}
