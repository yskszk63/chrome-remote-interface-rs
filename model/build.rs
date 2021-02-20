use std::env;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;
use chrome_remote_interface_model_tools as tools;

fn main() -> anyhow::Result<()> {
    let outdir = env::var("OUT_DIR").context("no OUT_DIR found.")?;
    let destination = Path::new(&outdir).join("model.rs");
    let destination = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(destination)?;

    let mut fmt = Command::new("rustfmt")
        .stdin(Stdio::piped())
        .stdout(destination)
        .spawn()
        .unwrap();
    tools::run("protocol.json", &mut fmt.stdin.take().unwrap()).unwrap();
    if !fmt.wait()?.success() {
        anyhow::bail!("failed to run rustfmt")
    }

    tools::check_features("Cargo.toml", "protocol.json")?;

    println!("cargo:rerun-if-changed=protocol.json");
    Ok(())
}
