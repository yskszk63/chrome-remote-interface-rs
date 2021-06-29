use std::env;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use dirs::home_dir;
use futures::TryFutureExt;
use tempfile::TempDir;
use tokio::fs::{create_dir_all, metadata, File};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::sleep;
use url::Url;
use which::which;

use crate::process::{Process, ProcessBuilder, ProcessStdio};

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

    /// Browser not found.
    #[error("browser not found.")]
    BrowserNotFound,
}

type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug)]
enum UserDataDir {
    Generated(TempDir),
    Specified(PathBuf),
    Default,
}

impl UserDataDir {
    async fn generated() -> Result<Self> {
        if let Ok(..) = metadata("/snap").await {
            // Newer Ubunts chromium runs in snapcraft.
            // Snapcraft chromium can not access /tmp dir.
            let snapdir = home_dir()
                .unwrap_or_else(|| "".into())
                .join("snap/chromium/common");
            create_dir_all(&snapdir).await?;
            Ok(Self::Generated(TempDir::new_in(&snapdir)?))
        } else {
            Ok(Self::Generated(TempDir::new()?))
        }
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
    Pipe(Option<crate::pipe::OsPipe>),
    Ws,
}

fn which_browser(browser: &BrowserType) -> Option<PathBuf> {
    if let Ok(val) = env::var(crate::BROWSER_BIN) {
        return which(val).ok();
    }
    crate::os::find_browser(browser)
}

/// Launcher (Builder) for Browser.
#[derive(Debug, Default)]
pub struct Launcher {
    browser_type: Option<BrowserType>,
    user_data_dir: Option<UserDataDir>,
    headless: Option<bool>,
    use_pipe: Option<bool>,
    output: Option<bool>,
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

    /// Specify protocol transport using pipe or not (websocket). (Default: Windows/Mac: false,
    /// Other: true)
    pub fn use_pipe(&mut self, value: bool) -> &mut Self {
        self.use_pipe = Some(value);
        self
    }

    /// Whether or not browser process stdout / stderr. (Default: false)
    pub fn output(&mut self, value: bool) -> &mut Self {
        self.output = Some(value);
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
            UserDataDir::generated().await?
        };
        let headless = self.headless.unwrap_or(true);

        let browser_type = self
            .browser_type
            .to_owned()
            .unwrap_or(BrowserType::Chromium);

        let mut command = if let Some(bin) = which_browser(&browser_type) {
            ProcessBuilder::new(bin)
        } else {
            return Err(BrowserError::BrowserNotFound);
        };

        command.stdin(ProcessStdio::null());
        if self.output.unwrap_or(false) {
            command
                .stdout(ProcessStdio::inherit())
                .stderr(ProcessStdio::inherit());
        } else {
            command
                .stdout(ProcessStdio::null())
                .stderr(ProcessStdio::null());
        }

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

        let (proc, remote_debugging) = if self.use_pipe.unwrap_or(true) {
            command.arg("--remote-debugging-pipe");
            log::debug!("browser spawned {:?}", command);
            let (proc, ospipe) = command.spawn_with_pipe().await?;
            (proc, RemoteDebugging::Pipe(Some(ospipe)))
        } else {
            command.arg("--remote-debugging-port=0");
            log::debug!("browser spawned {:?}", command);
            let proc = command.spawn()?;
            (proc, RemoteDebugging::Ws)
        };

        Ok(Browser {
            when: now,
            proc: Some(proc),
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
    proc: Option<Process>,
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

    pub(crate) async fn cdp_url(&mut self) -> Result<Url> {
        let f = self.user_data_dir().join("DevToolsActivePort");

        let interval = Duration::from_millis(200);
        for n in 0..50usize {
            match File::open(&f).await {
                Ok(f) => {
                    let metadata = f.metadata().await?;
                    if metadata.modified()? >= self.when {
                        let mut f = BufReader::new(f).lines();
                        let maybe_port = f.next_line().await?;
                        let maybe_path = f.next_line().await?;
                        let maybe_eof = f.next_line().await?;
                        if let (Some(port), Some(path), None) = (maybe_port, maybe_path, maybe_eof)
                        {
                            return Ok(Url::parse(&format!("ws://127.0.0.1:{}{}", port, path))?);
                        } else {
                            return Err(BrowserError::UnexpectedFormat);
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    if self.proc.as_mut().unwrap().try_wait()? {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "process may be terminated.",
                        )
                        .into());
                    }
                    log::trace!("{}: {:?} not found. wait {}.", n, f, interval.as_millis());
                }
                Err(e) => return Err(e.into()),
            }
            sleep(interval).await;
        }

        Err(BrowserError::CannotDetectUrl)
    }

    /// Connect Chrome DevTools Protocol Client.
    ///
    /// This instance Ownership move to Client.
    pub async fn connect(mut self) -> super::Result<(super::CdpClient, super::Loop)> {
        let maybe_channel = match &mut self.remote_debugging {
            RemoteDebugging::Ws => None,
            RemoteDebugging::Pipe(inner) => Some(inner.take().unwrap().into()),
        };
        match maybe_channel {
            None => super::CdpClient::connect_ws(&self.cdp_url().await?, Some(self)).await,
            Some(channel) => super::CdpClient::connect_pipe(self, channel).await,
        }
    }

    /// Connect Chrome DevTools Protocol Client and run async block.
    pub async fn run_with<F, E, R, Fut>(self, fun: F) -> std::result::Result<R, E>
    where
        F: FnOnce(super::CdpClient) -> Fut,
        E: From<super::Error>,
        Fut: Future<Output = std::result::Result<R, E>>,
    {
        let (client, r#loop) = self.connect().await?;
        let (_, result) = tokio::try_join!(r#loop.map_err(E::from), fun(client))?;

        Ok(result)
    }
}

impl Browser {
    /// Close browser.
    pub async fn close(&mut self) {
        if let Some(proc) = self.proc.take() {
            proc.kill().await;
        }
        self.user_data_dir.take();
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(proc) = self.proc.take() {
            proc.kill_sync();
        }
        self.user_data_dir.take();
    }
}
