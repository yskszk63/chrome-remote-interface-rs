use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::sink::Sink;
use futures::Stream;
use serde_json::Value;
use tokio::io::Empty;
use tokio::process::{Child, Command};

pub const USE_PIPE_DEFAULT: bool = false;

pub type OsPipeWrite = tokio::fs::File; // FIXME dummy

pub type OsPipeRead = tokio::fs::File; // FIXME dummy

pub fn find_browser(_browser: &crate::browser::BrowserType) -> Option<PathBuf> {
    if let Ok(bin) = which(r#"C:\Program Files\Chromium\Application\chrome.exe"#) {
        return Some(bin);
    }

    which("chromium").ok()
}

pub fn edit_command_and_new(command: &mut Command) -> io::Result<(OsPipeWrite, OsPipeRead)> {
    unimplemented!()
}

pub async fn proc_kill(mut proc: Child) {
    proc.kill().await.ok();
}

pub fn proc_kill_sync(mut proc: Child) {
    proc.start_kill().ok();
}
