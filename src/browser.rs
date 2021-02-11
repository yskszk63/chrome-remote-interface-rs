use std::io;
use std::os::unix::io::IntoRawFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tokio_pipe::{PipeRead, PipeWrite};
use url::Url;

/// Operate browser error.
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// IO error.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// Cannot detect url for Chrome DevTools Protocol URL.
    #[error("cannot detect url.")]
    CannotDetectUrl,

    /// Unexpected format for DevToolsActivePort.
    #[error("unexpected format.")]
    UnexpectedFormat,

    /// Failed to parse URL.
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
}

type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug)]
enum UserDataDir {
    Generated(TempDir),
    Specified(PathBuf),
    Default,
}

impl UserDataDir {
    fn generated() -> Result<Self> {
        Ok(Self::Generated(TempDir::new()?))
    }
}

/// Browser type.
#[derive(Debug, Clone)]
pub enum BrowserType {
    /// Chromium
    Chromium,
}

#[derive(Debug)]
enum RemoteDebugging {
    Pipe(Option<(PipeWrite, PipeRead)>),
    Ws,
}

/// Launcher (Builder) for Browser.
#[derive(Debug, Default)]
pub struct Launcher {
    browser_type: Option<BrowserType>,
    user_data_dir: Option<UserDataDir>,
    headless: Option<bool>,
    use_pipe: Option<bool>,
}

impl Launcher {
    /// Specify launching browser type. (Default: Chromium)
    pub fn browser_type(&mut self, value: BrowserType) -> &mut Self {
        self.browser_type = Some(value);
        self
    }

    /// Specify user data dir. (If not specified: using temporary file)
    pub fn user_data_dir<P: AsRef<Path>>(&mut self, path: P) -> &mut Self {
        self.user_data_dir = Some(UserDataDir::Specified(path.as_ref().to_path_buf()));
        self
    }

    /// Use default user data dir. (If not specified: using temporary file)
    pub fn user_data_dir_default(&mut self) -> &mut Self {
        self.user_data_dir = Some(UserDataDir::Default);
        self
    }

    /// Specify headless mode or not. (Default: headless)
    pub fn headless(&mut self, value: bool) -> &mut Self {
        self.headless = Some(value);
        self
    }

    /// Specify protocol transport using pipe or not (websocket). (Default: pipe)
    pub fn use_pipe(&mut self, value: bool) -> &mut Self {
        // FIXME not supported for Windows
        self.use_pipe = Some(value);
        self
    }

    /// Launching browser.
    pub async fn launch(&mut self) -> Result<Browser> {
        let now = SystemTime::now();

        let user_data_dir = if let Some(dir) = &self.user_data_dir {
            match dir {
                UserDataDir::Specified(dir) => UserDataDir::Specified(dir.clone()),
                UserDataDir::Default => UserDataDir::Default,
                _ => unreachable!(),
            }
        } else {
            UserDataDir::generated()?
        };
        let headless = self.headless.unwrap_or(true);

        let browser_type = self
            .browser_type
            .to_owned()
            .unwrap_or(BrowserType::Chromium);
        let mut command = match &browser_type {
            BrowserType::Chromium => Command::new("chromium"),
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if headless {
            command.args(&["--headless", "--disable-gpu"]);
        }
        match &user_data_dir {
            UserDataDir::Default => &mut command,
            UserDataDir::Generated(p) => command.arg(&format!(
                "--user-data-dir={}",
                p.as_ref().to_string_lossy().to_string()
            )),
            UserDataDir::Specified(p) => command.arg(&format!(
                "--user-data-dir={}",
                p.to_string_lossy().to_string()
            )),
        };

        // https://github.com/puppeteer/puppeteer/blob/9a8479a52a7d8b51690b0732b2a10816cd1b8aef/src/node/Launcher.ts#L159
        command.args(&[
            "--disable-background-networking",
            "--enable-features=NetworkService,NetworkServiceInProcess",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-breakpad",
            "--disable-client-side-phishing-detection",
            "--disable-component-extensions-with-background-pages",
            "--disable-default-apps",
            "--disable-dev-shm-usage",
            "--disable-extensions",
            "--disable-features=Translate",
            "--disable-hang-monitor",
            "--disable-ipc-flooding-protection",
            "--disable-popup-blocking",
            "--disable-prompt-on-repost",
            "--disable-renderer-backgrounding",
            "--disable-sync",
            "--force-color-profile=srgb",
            "--metrics-recording-only",
            "--no-first-run",
            "--enable-automation",
            "--password-store=basic",
            "--use-mock-keychain",
        ]);

        let remote_debugging = if self.use_pipe.unwrap_or(true) {
            use nix::fcntl::{fcntl, FcntlArg, OFlag};
            use nix::unistd::{close, dup, dup2};

            command.arg("--remote-debugging-pipe");
            let (pipein_rx, pipein) = tokio_pipe::pipe()?;
            let (pipeout, pipeout_tx) = tokio_pipe::pipe()?;
            let pipein_rx = pipein_rx.into_raw_fd();
            let pipeout_tx = pipeout_tx.into_raw_fd();

            let flag = fcntl(pipein_rx, FcntlArg::F_GETFL)
                .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            fcntl(
                pipein_rx,
                FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
            )
            .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            let flag = fcntl(pipeout_tx, FcntlArg::F_GETFL)
                .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
            fcntl(
                pipeout_tx,
                FcntlArg::F_SETFL(OFlag::from_bits_truncate(flag) ^ OFlag::O_NONBLOCK),
            )
            .map_err(|e| io::Error::from(e.as_errno().unwrap()))?;

            unsafe {
                command.pre_exec(move || {
                    let pipein2 =
                        dup(pipein_rx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    let pipeout2 =
                        dup(pipeout_tx).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    dup2(pipein2, 3).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    dup2(pipeout2, 4).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    close(pipein2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    close(pipeout2).map_err(|e| io::Error::from(e.as_errno().unwrap()))?;
                    Ok(())
                });
            }
            RemoteDebugging::Pipe(Some((pipein, pipeout)))
        } else {
            command.arg("--remote-debugging-port=0");
            RemoteDebugging::Ws
        };

        Ok(Browser {
            when: now,
            proc: Some(command.spawn()?),
            browser_type,
            user_data_dir: Some(user_data_dir),
            remote_debugging,
        })
    }
}

/// Represent instance.
///
/// Make drop on kill (TERM) and clean generated user data dir best effort.
#[derive(Debug)]
pub struct Browser {
    when: SystemTime,
    proc: Option<Child>,
    browser_type: BrowserType,
    user_data_dir: Option<UserDataDir>,
    remote_debugging: RemoteDebugging,
}

impl Browser {
    /// Construct [`Launcher`] instance.
    pub fn launcher() -> Launcher {
        Default::default()
    }

    fn user_data_dir(&self) -> PathBuf {
        match self.user_data_dir.as_ref().expect("already closed.") {
            UserDataDir::Generated(path) => path.as_ref().to_path_buf(),
            UserDataDir::Specified(path) => path.to_path_buf(),
            UserDataDir::Default => {
                // https://chromium.googlesource.com/chromium/src/+/master/docs/user_data_dir.md
                todo!()
            }
        }
    }

    pub(crate) async fn cdp_url(&self) -> Result<Url> {
        let f = self.user_data_dir().join("DevToolsActivePort");

        for _ in 0..20usize {
            match File::open(&f).await {
                Ok(f) => {
                    let metadata = f.metadata().await?;
                    if metadata.modified()? > self.when {
                        let mut f = BufReader::new(f).lines();
                        let maybe_port = f.next_line().await?;
                        let maybe_path = f.next_line().await?;
                        let maybe_eof = f.next_line().await?;
                        if let (Some(port), Some(path), None) = (maybe_port, maybe_path, maybe_eof) {
                            return Ok(Url::parse(&format!("ws://127.0.0.1:{}{}", port, path))?);
                        } else {
                            return Err(BrowserError::UnexpectedFormat);
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err(BrowserError::CannotDetectUrl)
    }

    /// Connect Chrome DevTools Protocol Client.
    ///
    /// This instance Ownership move to Client.
    pub async fn connect(mut self) -> super::Result<(super::CdpClient, super::Loop)> {
        let maybe_pipeio = match &mut self.remote_debugging {
            RemoteDebugging::Ws => None,
            RemoteDebugging::Pipe(pipeio) => Some(pipeio.take().unwrap()),
        };
        match maybe_pipeio {
            None => super::CdpClient::connect_ws(&self.cdp_url().await?, Some(self)).await,
            Some((pipein, pipeout)) => super::CdpClient::connect_pipe(self, pipein, pipeout).await,
        }
    }
}

impl Browser {
    /// Close browser.
    pub async fn close(&mut self) {
        /// FIXME Windows not supported.
        use nix::sys::signal::{kill, SIGTERM};
        use nix::unistd::Pid;

        if let Some(mut proc) = self.proc.take() {
            if let Some(pid) = proc.id() {
                let pid = Pid::from_raw(pid as i32);
                kill(pid, Some(SIGTERM)).ok(); // FIXME
                proc.wait().await.ok();
            }
            self.user_data_dir.take();
            //std::mem::forget(self.user_data_dir.take());
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        use nix::sys::signal::{kill, SIGTERM};
        use nix::sys::wait::waitpid;
        use nix::unistd::Pid;

        if let Some(proc) = self.proc.take() {
            if let Some(pid) = proc.id() {
                let pid = Pid::from_raw(pid as i32);
                kill(pid, Some(SIGTERM)).ok(); // FIXME
                waitpid(Some(pid), None).ok(); // FIXME blocking
            }
            self.user_data_dir.take();
            //std::mem::forget(self.user_data_dir.take());
        }
    }
}
