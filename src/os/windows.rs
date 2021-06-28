use std::io;
use std::os::windows::io::IntoRawHandle;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::sink::Sink;
use futures::Stream;
use serde_json::Value;
use tokio::io::Empty;
use which::which;
use winspawn::{move_fd, spawn as winspawn, Child, FileDescriptor, Mode};

use crate::pipe::OsPipe;
use crate::process::Process;
use crate::process::ProcessBuilder;

pub const USE_PIPE_DEFAULT: bool = true;

pub type OsPipeWrite = tokio_anon_pipe::AnonPipeWrite;

pub type OsPipeRead = tokio_anon_pipe::AnonPipeRead;

pub type OsProcess = Child;

pub fn find_browser(_browser: &crate::browser::BrowserType) -> Option<PathBuf> {
    if let Ok(bin) = which(r#"C:\Program Files\Chromium\Application\chrome.exe"#) {
        return Some(bin);
    }

    which("chromium").ok()
}

pub async fn spawn_with_pipe(builder: ProcessBuilder) -> io::Result<(OsProcess, OsPipe)> {
    let (pipein_rx, pipein) = tokio_anon_pipe::anon_pipe().await?;
    let (pipeout, pipeout_tx) = tokio_anon_pipe::anon_pipe().await?;
    let pipein_rx = pipein_rx.into_raw_handle();
    let pipeout_tx = pipeout_tx.into_raw_handle();

    let pipein_rx = FileDescriptor::from_raw_handle(pipein_rx, Mode::ReadOnly)?;
    let pipeout_tx = FileDescriptor::from_raw_handle(pipeout_tx, Mode::WriteOnly)?;

    let child = move_fd(&pipein_rx, 3, |_| {
        move_fd(&pipeout_tx, 4, |_| {
            // FIXME stdio
            winspawn(builder.get_program(), builder.get_args())
        })
    })?;

    Ok((child, OsPipe::new(pipein, pipeout)))
}

pub fn spawn(builder: ProcessBuilder) -> io::Result<OsProcess> {
    // FIXME stdio
    winspawn(builder.get_program(), builder.get_args())
}

pub async fn proc_kill(mut proc: OsProcess) {
    proc.kill();
}

pub fn proc_kill_sync(mut proc: OsProcess) {
    proc.kill();
}

pub fn try_wait(proc: &mut OsProcess) -> io::Result<bool> {
    Ok(proc.try_wait()?.is_some())
}
