use std::process::Stdio;

use nix::libc::pid_t;
use nix::sys::signal::{kill, SIGTERM};
use nix::unistd::Pid;
use reqwest::{Client, Url};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut proc = Command::new("chromium")
        .arg("--headless")
        .arg("--remote-debugging-port=0")
        .arg("--disable-gpu")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = proc.stderr.take().unwrap();
    let mut stderr = BufReader::new(stderr).lines();
    let mut url = loop {
        if let Some(line) = stderr.next_line().await? {
            if let Some(url) = line.strip_prefix("DevTools listening on ") {
                break Url::parse(url)?;
            }
        } else {
            anyhow::bail!("failed to detect url")
        }
    };
    url.set_scheme("http").unwrap();
    url.set_path("/json/protocol");

    let client = Client::new();
    let mut response = client.get(url).send().await?;
    let mut stdout = tokio::io::stdout();
    while let Some(chunk) = response.chunk().await? {
        stdout.write_all(&chunk).await?;
    }

    if let Some(pid) = proc.id() {
        let pid = Pid::from_raw(pid as pid_t);
        kill(pid, SIGTERM)?;
    }
    proc.wait().await?;

    Ok(())
}
