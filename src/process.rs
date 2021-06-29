use std::ffi::{OsStr, OsString};
use std::io;
use std::iter;
use std::process::Stdio;

use crate::os;
use crate::pipe::OsPipe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStdio {
    Inherit,
    Null,
}

impl ProcessStdio {
    pub fn null() -> Self {
        Self::Null
    }
    pub fn inherit() -> Self {
        Self::Inherit
    }
}

impl Default for ProcessStdio {
    fn default() -> Self {
        Self::Inherit
    }
}

impl From<ProcessStdio> for Stdio {
    fn from(v: ProcessStdio) -> Self {
        match v {
            ProcessStdio::Null => Stdio::null(),
            ProcessStdio::Inherit => Stdio::inherit(),
        }
    }
}

#[derive(Debug)]
pub struct ProcessBuilder {
    program: OsString,
    stdin: ProcessStdio,
    stdout: ProcessStdio,
    stderr: ProcessStdio,
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
            stdin: Default::default(),
            stdout: Default::default(),
            stderr: Default::default(),
            args: Default::default(),
        }
    }

    pub fn get_program(&self) -> &OsStr {
        self.program.as_os_str()
    }

    pub fn stdin(&mut self, cfg: ProcessStdio) -> &mut Self {
        self.stdin = cfg;
        self
    }

    pub fn get_stdin(&self) -> ProcessStdio {
        self.stdin
    }

    pub fn stdout(&mut self, cfg: ProcessStdio) -> &mut Self {
        self.stdout = cfg;
        self
    }

    pub fn get_stdout(&self) -> ProcessStdio {
        self.stdout
    }

    pub fn stderr(&mut self, cfg: ProcessStdio) -> &mut Self {
        self.stderr = cfg;
        self
    }

    pub fn get_stderr(&self) -> ProcessStdio {
        self.stderr
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
