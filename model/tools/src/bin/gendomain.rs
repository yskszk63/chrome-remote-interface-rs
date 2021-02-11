use std::io::stdout;
use anyhow::Context;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args();
    let path = args.nth(1).context("no input found.")?;
    let output = stdout();
    let mut output = output.lock();
    chrome_remote_interface_model_tools::run(path, &mut output)?;
    Ok(())
}
