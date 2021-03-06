use std::io;
use std::os::unix::io::IntoRawFd;

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, SIGTERM};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::unistd::{close, dup, dup2};
use tokio::process::{Child, Command};
use tokio_pipe::{PipeRead, PipeWrite};

pub const USE_PIPE_DEFAULT: bool = !cfg!(target_os = "macos");

pub type OsPipeWrite = PipeWrite;

pub type OsPipeRead = PipeRead;

pub fn edit_command_and_new(command: &mut Command) -> io::Result<(OsPipeWrite, OsPipeRead)> {
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

    command.arg("--remote-debugging-pipe");
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

    Ok((pipein, pipeout))
}

pub async fn proc_kill(mut proc: Child) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        proc.wait().await.ok();
    }
}

pub fn proc_kill_sync(proc: Child) {
    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as i32);
        kill(pid, Some(SIGTERM)).ok(); // FIXME
        waitpid(Some(pid), None).ok(); // FIXME blocking
    }
}
