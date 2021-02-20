use std::convert::TryFrom;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::Context as _;
pub use check_features::check_features;
use serde::Deserialize;

mod check_features;
mod render;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Property {
    name: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(flatten)]
    r#type: Type,
}

impl Typed for Property {
    fn r#type(&self) -> &Type {
        &self.r#type
    }
}

impl Annotatable for Property {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
struct RawType {
    r#type: Option<String>,
    #[serde(rename = "$ref")]
    r#ref: Option<String>,
    r#enum: Option<Vec<String>>,
    properties: Option<Vec<Property>>,
    items: Option<Type>,
    optional: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawType")]
enum Type {
    String,

    Integer,

    Number,

    Boolean,

    Binary,

    Any,

    EnumString(Vec<String>),

    Object(Vec<Property>),

    Array(Box<Type>),

    Ref(String),

    Option(Box<Type>),
}

impl TryFrom<RawType> for Type {
    type Error = anyhow::Error;

    fn try_from(value: RawType) -> Result<Self, Self::Error> {
        let RawType {
            r#type,
            r#ref,
            r#enum,
            properties,
            items,
            optional,
        } = value;
        let result = match (r#type.as_deref(), r#ref, r#enum, properties, items) {
            (None, Some(r), None, None, None) => Type::Ref(r),
            (Some("string"), None, Some(v), None, None) => Type::EnumString(v),
            (Some("string"), None, None, None, None) => Type::String,
            (Some("integer"), None, None, None, None) => Type::Integer,
            (Some("number"), None, None, None, None) => Type::Number,
            (Some("boolean"), None, None, None, None) => Type::Boolean,
            (Some("binary"), None, None, None, None) => Type::Binary,
            (Some("any"), None, None, None, None) => Type::Any,
            (Some("object"), None, None, properties, None) => {
                Type::Object(properties.unwrap_or_default())
            }
            (Some("array"), None, None, None, Some(items)) => Type::Array(Box::new(items)),
            other => anyhow::bail!("{:?}", other),
        };
        if optional.unwrap_or_default() {
            Ok(Self::Option(Box::new(result)))
        } else {
            Ok(result)
        }
    }
}

trait Typed {
    fn r#type(&self) -> &Type;
}

trait Annotatable {
    fn annotation(&self) -> &Annotation;

    fn deps(&self) -> Option<Vec<String>> {
        None
    }
}

#[derive(Debug, Deserialize, Default)]
struct Annotation {
    description: Option<String>,
    #[serde(default)]
    experimental: bool,
    #[serde(default)]
    deprecated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainType {
    id: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(flatten)]
    r#type: Type,
}

impl Typed for DomainType {
    fn r#type(&self) -> &Type {
        &self.r#type
    }
}

impl Annotatable for DomainType {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameter {
    name: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(flatten)]
    r#type: Type,
}

impl Typed for Parameter {
    fn r#type(&self) -> &Type {
        &self.r#type
    }
}

impl Annotatable for Parameter {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Return {
    name: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(flatten)]
    r#type: Type,
}

impl Typed for Return {
    fn r#type(&self) -> &Type {
        &self.r#type
    }
}

impl Annotatable for Return {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    name: String,
    redirect: Option<String>,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(default)]
    parameters: Vec<Parameter>,
    #[serde(default)]
    returns: Vec<Return>,
}

impl Annotatable for Command {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    name: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(default)]
    parameters: Vec<Parameter>,
}

impl Annotatable for Event {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    domain: String,
    #[serde(flatten)]
    annotation: Annotation,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    types: Vec<DomainType>,
    #[serde(default)]
    commands: Vec<Command>,
    #[serde(default)]
    events: Vec<Event>,
}

impl Annotatable for Domain {
    fn annotation(&self) -> &Annotation {
        &self.annotation
    }

    fn deps(&self) -> Option<Vec<String>> {
        Some(
            vec![self.domain.clone()]
                .into_iter()
                .chain(self.dependencies.clone().into_iter())
                .collect(),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolVersion {
    major: String,
    minor: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol {
    domains: Vec<Domain>,
    version: ProtocolVersion,
}

pub fn run<P: AsRef<Path>, W: Write>(path: P, output: &mut W) -> anyhow::Result<()> {
    let mut f = File::open(path)?;
    let protocol = serde_json::from_reader(&mut f).context("failed to parse protocol json.")?;
    let program = render::render(&protocol);
    write!(output, "{}", program)?;
    Ok(())
}
