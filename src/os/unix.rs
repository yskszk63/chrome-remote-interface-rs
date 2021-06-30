use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::Stdio;

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, SIGTERM};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::unistd::{close, dup, dup2, setsid};
use tokio::process::{Child, Command};
use tokio_pipe::{PipeRead, PipeWrite};
use which::which;

use crate::pipe::OsPipe;
use crate::process::ProcessBuilder;

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

fn into_io_err(err: nix::Error) -> io::Error {
    match err.as_errno() {
        Some(err) => io::Error::from(err),
        None => io::Error::new(io::ErrorKind::Other, err),
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(into_io_err)?;
    fcntl(
        fd,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) ^ OFlag::O_NONBLOCK),
    )
    .map_err(into_io_err)?;
    Ok(())
}

pub async fn spawn_with_pipe(builder: ProcessBuilder) -> io::Result<(OsProcess, OsPipe)> {
    let mut command = Command::new(builder.get_program());
    command
        .args(builder.get_args())
        .stdin(Stdio::from(builder.get_stdin()))
        .stdout(Stdio::from(builder.get_stdout()))
        .stderr(Stdio::from(builder.get_stderr()));

    let (input, their_input) = tokio_pipe::pipe()?;
    let (their_output, output) = tokio_pipe::pipe()?;

    let input = input.as_raw_fd();
    let output = output.as_raw_fd();
    set_nonblocking(output)?;
    set_nonblocking(output)?;

    unsafe {
        command.pre_exec(move || {
            // dup for drop CLOEXEC
            let input_no_cloexec = dup(input).map_err(into_io_err)?;
            let output_no_cloexec = dup(output).map_err(into_io_err)?;

            // copy child process fd
            dup2(input_no_cloexec, 3).map_err(into_io_err)?;
            dup2(output_no_cloexec, 4).map_err(into_io_err)?;

            close(input_no_cloexec).map_err(into_io_err)?;
            close(output_no_cloexec).map_err(into_io_err)?;

            // No need close because Input and output are flagged with CLOEXEC.

            setsid().map_err(into_io_err)?;
            Ok(())
        });
    }

    let proc = command.spawn()?;
    Ok((proc, OsPipe::new(their_input, their_output)))
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
