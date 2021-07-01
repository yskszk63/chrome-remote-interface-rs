use std::fs::File;
use std::io;
use std::mem;
use std::os::raw::c_int;
use std::path::PathBuf;

use which::which;
use winspawn::{move_fd, spawn as winspawn, Child, FileDescriptor, Mode};

use crate::pipe::OsPipe;
use crate::process::{ProcessBuilder, ProcessStdio};

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

    let pipein_rx = FileDescriptor::from_raw_handle(pipein_rx, Mode::ReadOnly)?;
    let pipeout_tx = FileDescriptor::from_raw_handle(pipeout_tx, Mode::WriteOnly)?;

    let child = move_fd(&pipein_rx, 3, |_| {
        move_fd(&pipeout_tx, 4, |_| spawn_local(&builder))
    })?;

    Ok((child, OsPipe::new(pipein, pipeout)))
}

pub fn spawn(builder: ProcessBuilder) -> io::Result<OsProcess> {
    spawn_local(&builder)
}

fn spawn_local(builder: &ProcessBuilder) -> io::Result<OsProcess> {
    with_stdio(builder.get_stdin(), Mode::ReadOnly, 0, move || {
        with_stdio(builder.get_stdout(), Mode::WriteOnly, 1, move || {
            with_stdio(builder.get_stderr(), Mode::WriteOnly, 2, move || {
                winspawn(builder.get_program(), builder.get_args())
            })
        })
    })
}

fn with_stdio<F, R>(stdio: ProcessStdio, mode: Mode, fd: c_int, func: F) -> io::Result<R>
where
    F: FnOnce() -> io::Result<R>,
{
    if stdio == ProcessStdio::Inherit {
        return func();
    }

    let file = File::open("NUL")?;
    let stdiofd = FileDescriptor::from_raw_handle(file, mode)?;
    let result = move_fd(&stdiofd, fd, move |_| func());
    mem::forget(stdiofd);
    result
}

pub async fn proc_kill(mut proc: OsProcess) {
    proc.kill().ok();
}

pub fn proc_kill_sync(mut proc: OsProcess) {
    proc.kill().ok();
}

pub fn try_wait(proc: &mut OsProcess) -> io::Result<bool> {
    Ok(proc.try_wait()?.is_some())
}
