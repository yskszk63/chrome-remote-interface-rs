use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dirs::home_dir;
use tempfile::TempDir;
use tokio::fs::{create_dir_all, metadata};
use which::which;

use crate::process::{Process, ProcessBuilder, ProcessStdio};

/// Operate browser error.
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// IO error.
    #[error(transparent)]
    Io(#[from] io::Error),

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

        command.arg("--remote-debugging-pipe");
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

        log::debug!("browser spawn {:?}", command);
        let (proc, ospipe) = command.spawn_with_pipe().await?;

        Ok(Browser {
            when: now,
            proc: Some(proc),
            browser_type,
            user_data_dir: Some(user_data_dir),
            pipe: Some(ospipe),
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
    pipe: Option<crate::pipe::OsPipe>,
}

impl Browser {
    /// Construct [`Launcher`] instance.
    pub fn launcher() -> Launcher {
        Default::default()
    }

    /// Connect Chrome DevTools Protocol Client.
    ///
    /// This instance Ownership move to Client.
    pub fn connect(mut self) -> super::CdpClient {
        let channel = self.pipe.take().unwrap().into();
        super::CdpClient::connect_pipe(self, channel)
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

    pub async fn wait(&mut self) -> io::Result<()> {
        if let Some(proc) = self.proc.as_mut() {
            proc.wait().await?;
        }
        Ok(())
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
