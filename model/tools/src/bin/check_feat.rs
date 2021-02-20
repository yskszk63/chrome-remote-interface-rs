use std::env;

use chrome_remote_interface_model_tools::check_features;

fn main() -> anyhow::Result<()> {
    let (cargo_toml, protocol_json) =
        if let [_, cargo_toml, protocol_json] = &*env::args().collect::<Vec<_>>() {
            (cargo_toml.clone(), protocol_json.clone())
        } else {
            anyhow::bail!("usage: %prog path/to/Cargo.toml path/to/protocol.json")
        };

    check_features(cargo_toml, protocol_json)?;
    Ok(())
}
