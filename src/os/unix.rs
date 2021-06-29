use std::io;
use std::os::unix::io::IntoRawFd;
use std::path::PathBuf;
use std::process::Stdio;

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, SIGTERM};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::unistd::{close, dup, dup2};
use tokio::process::{Child, Command};
use tokio_pipe::{PipeRead, PipeWrite};
use which::which;

use crate::pipe::OsPipe;
use crate::process::ProcessBuilder;

#[cfg(target_os = "macos")]
pub const USE_PIPE_DEFAULT: bool = false;

#[cfg(not(target_os = "macos"))]
pub const USE_PIPE_DEFAULT: bool = true;

pub type OsPipeWrite = PipeWrite;

pub type OsPipeRead = PipeRead;

pub type OsProcess = Child;

#[cfg(target_os = "macos")]
pub fn find_browser(_browser: &crate::browser::BrowserType) -> Option<PathBuf> {
    if let Ok(bin) = which("/Applications/Chromium.app/Contents/MacOS/Chromium") {
        return Some(bin);
    }

    which("chromium").ok()
}

#[cfg(not(target_os = "macos"))]
pub fn find_browser(_browser: &crate::browser::BrowserType) -> Option<PathBuf> {
    if let Ok(bin) = which("/usr/bin/chromium") {
        return Some(bin);
    }

    if let Ok(bin) = which("/usr/bin/chromium-browser") {
        return Some(bin);
    }

    which("chromium").ok()
}

pub async fn spawn_with_pipe(builder: ProcessBuilder) -> io::Result<(OsProcess, OsPipe)> {
    let mut command = Command::new(builder.get_program());
    command
        .args(builder.get_args())
        .stdin(Stdio::from(builder.get_stdin()))
        .stdout(Stdio::from(builder.get_stdout()))
        .stderr(Stdio::from(builder.get_stderr()));

    let (pipein_rx, pipein) = tokio_pipe::pipe()?;
    let (pipeout, pipeout_tx) = tokio_pipe::pipe()?;
    let pipein_rx = pipein_rx.into_raw_fd();
    let pipeout_tx = pipeout_tx.into_raw_fd();

    let flag =
        fcntl(pipein_rx, FcntlArg::F_GETFL).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
    fcntl(
        pipein_rx,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
    )
    .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
    let flag =
        fcntl(pipeout_tx, FcntlArg::F_GETFL).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
    fcntl(
        pipeout_tx,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
    )
    .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;

    unsafe {
        command.pre_exec(move || {
            let pipein2 = dup(pipein_rx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            let pipeout2 = dup(pipeout_tx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            dup2(pipein2, 3).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            dup2(pipeout2, 4).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            close(pipein2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            close(pipeout2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            Ok(())
        });
    }

    let proc = command.spawn()?;
    Ok((proc, OsPipe::new(pipein, pipeout)))
}

pub fn spawn(builder: ProcessBuilder) -> io::Result<OsProcess> {
    let proc = Command::new(builder.get_program())
        .args(builder.get_args())
        .stdin(Stdio::from(builder.get_stdin()))
        .stdout(Stdio::from(builder.get_stdout()))
        .stderr(Stdio::from(builder.get_stderr()))
        .spawn()?;
    Ok(proc)
}

pub async fn proc_kill(mut proc: OsProcess) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        proc.wait().await.ok();
    }
}

pub fn proc_kill_sync(proc: OsProcess) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        waitpid(Some(pid), None).ok(); // FIXME blocking
    }
}

pub fn try_wait(proc: &mut OsProcess) -> io::Result<bool> {
    Ok(proc.try_wait()?.is_some())
}
