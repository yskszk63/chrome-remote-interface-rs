use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Annotatable;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CargoToml {
    features: BTreeMap<String, Vec<String>>,
}

fn expect<P>(path: P) -> anyhow::Result<CargoToml>
where
    P: AsRef<Path>,
{
    let mut features = BTreeMap::new();

    let protocol_json = fs::read_to_string(path)?;
    let protocol_json = serde_json::from_str::<crate::Protocol>(&protocol_json)?;

    features.insert(
        "default".into(),
        vec!["Browser".into(), "Target".into(), "Page".into()],
    );
    features.insert("experimental".into(), vec![]);
    for mut domain in protocol_json.domains {
        let mut deps = if domain.annotation().experimental {
            vec!["experimental".into()]
        } else {
            vec![]
        };
        deps.append(&mut domain.dependencies);
        features.insert(domain.domain.clone(), deps);
    }

    Ok(CargoToml { features })
}

fn actual<P>(path: P) -> anyhow::Result<CargoToml>
where
    P: AsRef<Path>,
{
    let actual = fs::read_to_string(path)?;
    Ok(toml::from_str::<CargoToml>(&actual)?)
}

pub fn check_features<P1, P2>(cargo_toml: P1, protocol_json: P2) -> anyhow::Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let actual = actual(cargo_toml)?;
    let expect = expect(protocol_json)?;

    if actual != expect {
        anyhow::bail!(
            r#"features not expected.
expected:

{}"#,
            toml::to_string(&expect)?
        );
    }
    Ok(())
}
