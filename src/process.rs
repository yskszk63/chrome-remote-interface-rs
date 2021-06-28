use std::ffi::{OsStr, OsString};
use std::io;
use std::iter;
use std::process::Stdio;

use crate::os;
use crate::pipe::OsPipe;

#[derive(Debug)]
pub struct ProcessBuilder {
    program: OsString,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    args: Vec<OsString>,
}

impl ProcessBuilder {
    pub fn new<S>(program: S) -> Self
    where
        S: AsRef<OsStr>,
    {
        let program = program.as_ref().to_os_string();
        Self {
            program,
            stdin: Some(Stdio::inherit()),
            stdout: Some(Stdio::inherit()),
            stderr: Some(Stdio::inherit()),
            args: Default::default(),
        }
    }

    pub fn get_program(&self) -> &OsStr {
        self.program.as_os_str()
    }

    pub fn stdin<T>(&mut self, cfg: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stdin = Some(cfg.into());
        self
    }

    pub fn take_stdin(&mut self) -> Stdio {
        self.stdin.take().unwrap()
    }

    pub fn stdout<T>(&mut self, cfg: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stdout = Some(cfg.into());
        self
    }

    pub fn take_stdout(&mut self) -> Stdio {
        self.stdout.take().unwrap()
    }

    pub fn stderr<T>(&mut self, cfg: T) -> &mut Self
    where
        T: Into<Stdio>,
    {
        self.stderr = Some(cfg.into());
        self
    }

    pub fn take_stderr(&mut self) -> Stdio {
        self.stderr.take().unwrap()
    }

    pub fn arg<S>(&mut self, arg: S) -> &mut Self
    where
        S: AsRef<OsStr>,
    {
        self.args(iter::once(arg))
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|s| s.as_ref().to_os_string()));
        self
    }

    pub fn get_args(&self) -> &[OsString] {
        self.args.as_ref()
    }

    pub async fn spawn_with_pipe(self) -> io::Result<(Process, OsPipe)> {
        os::spawn_with_pipe(self)
            .await
            .map(|(proc, pipe)| (Process(proc), pipe))
    }

    pub fn spawn(self) -> io::Result<Process> {
        os::spawn(self).map(Process)
    }
}

#[derive(Debug)]
pub struct Process(os::OsProcess);

impl Process {
    pub async fn kill(self) {
        os::proc_kill(self.0).await;
    }

    pub fn kill_sync(self) {
        os::proc_kill_sync(self.0)
    }

    pub fn try_wait(&mut self) -> io::Result<bool> {
        os::try_wait(&mut self.0)
    }
}
