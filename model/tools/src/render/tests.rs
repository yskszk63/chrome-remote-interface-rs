use super::*;
use std::io::Read;
use std::io::Write;
use std::process::{Command as StdCommand, Stdio};

use similar::{ChangeTag, TextDiff};

fn rustfmt(prog: &TokenStream) -> String {
    let mut proc = StdCommand::new("rustfmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    write!(&mut proc.stdin.take().unwrap(), "{}", prog).unwrap();
    let mut buf = vec![];
    proc.stdout.take().unwrap().read_to_end(&mut buf).unwrap();
    if !proc.wait().unwrap().success() {
        panic!("failed to run rustfmt.")
    }
    String::from_utf8(buf).unwrap()
}

fn assert_eq(actual: &TokenStream, expect: &TokenStream) {
    let actual = rustfmt(actual);
    let expect = rustfmt(expect);

    let mut changes = String::new();
    let mut change_detect = false;
    let diff = TextDiff::from_lines(&actual, &expect);
    for change in diff.iter_all_changes() {
        change_detect |= match change.tag() {
            ChangeTag::Delete | ChangeTag::Insert => true,
            ChangeTag::Equal => false,
        };

        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        changes += &format!("{}{}", sign, change);
    }

    if change_detect {
        panic!("not equals\n{}", changes);
    }
}

#[test]
fn test_protocol() {
    let proto = Protocol {
        version: ProtocolVersion {
            major: "1".to_string(),
            minor: "3".to_string(),
        },
        domains: Default::default(),
    };
    let prog = render(&proto);
    let expect = quote! {
        mod g {
            #![allow(deprecated)]
            #![allow(clippy::many_single_char_names)]
            #![allow(clippy::new_without_default)]
            #![allow(clippy::wrong_self_convention)]
            use std::iter::{IntoIterator, FromIterator};
            use std::ops::Deref;
            use serde::{Serialize, Deserialize};
            use serde_json::Value as JsonValue;
            #[doc = "Session id."]
            #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
            pub struct SessionId(String);
            impl From<&str> for SessionId {
                fn from(v: &str) -> Self {
                    Self(v.into())
                }
            }
            impl From<String> for SessionId {
                fn from(v: String) -> Self {
                    Self(v)
                }
            }
            #[doc = "Message from client."]
            #[derive(Debug, Clone, Serialize)]
            pub struct Request<T>
                where
                    T: Command,
                {
                    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
                    session_id: Option<SessionId>,
                    id: u32,
                    method: &'static str,
                    params: T,
                }
            #[doc(hidden)]
            #[derive(Debug, Clone, Deserialize)]
            #[serde(untagged)]
            pub enum RawResponse {
                Event {
                    #[serde(rename = "sessionId")]
                    session_id: Option<SessionId>,
                    #[serde(flatten)]
                    event: Event,
                },
                UnknownEvent {
                    #[serde(rename = "sessionId")]
                    session_id: Option<SessionId>,
                    method: String,
                    params: JsonValue,
                },
                Return {
                    #[serde(rename = "sessionId")]
                    session_id: Option<SessionId>,
                    id: u32,
                    result: JsonValue,
                },
                Error {
                    #[serde(rename = "sessionId")]
                    session_id: Option<SessionId>,
                    id: u32,
                    error: JsonValue,
                },
            }
            impl From<RawResponse> for Response {
                fn from(v: RawResponse) -> Self {
                    match v {
                        RawResponse::Event { session_id, event } => Self::Event(session_id, event),
                        RawResponse::UnknownEvent {
                            session_id,
                            method,
                            params,
                        } => Self::Event(session_id, Event::Unknown(method, params)),
                        RawResponse::Return {
                            session_id,
                            id,
                            result
                        } => Self::Return(session_id, id, result),
                        RawResponse::Error {
                            session_id,
                            id,
                            error,
                        } => Self::Error(session_id, id, error),
                    }
                }
            }
            #[doc = "Message structure from Chrome."]
            #[derive(Debug, Clone, Deserialize)]
            #[serde(from = "RawResponse")]
            pub enum Response {
                #[doc = "Event message."]
                Event(Option<SessionId>, Event),
                #[doc = "Command success message."]
                Return(Option<SessionId>, u32, JsonValue),
                #[doc = "Command failure message."]
                Error(Option<SessionId>, u32, JsonValue),
            }
            #[doc = "Chrome DevTools Protocol Command."]
            pub trait Command: Serialize {
                #[doc = "Return type."]
                type Return: for<'a> Deserialize<'a>;
                #[doc = "Command method name."]
                const METHOD: &'static str;
                #[doc = "Into command request."]
                fn into_request(self, session_id: Option<SessionId>, id: u32) -> Request<Self>
                where
                    Self: Sized,
                {
                    Request {
                        session_id,
                        id,
                        method: Self::METHOD,
                        params: self,
                    }
                }
            }
            #[doc = "Generating Chrome DevTools protocol version."]
            pub const VERSION: &str = "1.3";
            #[doc = "Chrome DevTools Protocol event."]
            #[derive(Debug, Clone, Deserialize)]
            #[serde(tag = "method", content = "params")]
            pub enum Event {
                #[doc = "Unknown event."]
                #[serde(skip)]
                Unknown(String, JsonValue),
            }
        }
        pub use g::*;
    };

    assert_eq(&prog, &expect);
}

#[test]
fn test_domain() {
    let domain = Domain {
        domain: "domain".to_string(),
        annotation: Default::default(),
        commands: Default::default(),
        dependencies: Default::default(),
        events: Default::default(),
        types: Default::default(),
    };
    let prog = Context::new_root(&Default::default()).render_with(&domain);
    let expect = quote! {
        #[cfg(all(feature = "domain"))]
        #[cfg_attr(docsrs, doc(cfg(all(feature = "domain"))))]
        pub mod domain {
            use super::*;
        }
    };

    assert_eq(&prog, &expect);
}

#[test]
fn test_event() {
    let event = Event {
        annotation: Default::default(),
        name: "Evt".to_string(),
        parameters: Default::default(),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&event);
    let expect = quote! {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct EvtEvent {}
        impl EvtEvent {
            pub fn new() -> Self {
                Default::default()
            }
        }
    };

    assert_eq(&prog, &expect);
}

#[test]
fn test_command() {
    let command = Command {
        annotation: Default::default(),
        name: "Cmd".to_string(),
        parameters: Default::default(),
        redirect: None,
        returns: Default::default(),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&command);
    let expect = quote! {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct CmdCommand {}
        impl CmdCommand {
            pub fn new() -> Self {
                Default::default()
            }
        }
        impl Command for CmdCommand {
            type Return = CmdReturn;
            const METHOD : &'static str = "foo.Cmd";
        }
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct CmdReturn {}
        impl CmdReturn {
            pub fn new() -> Self {
                Default::default()
            }
        }
    };

    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_string() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::String,
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty(String);
        impl From<String> for Ty {
            fn from(v: String) -> Self {
                Self(v)
            }
        }
        impl From<&str> for Ty {
            fn from(v: &str) -> Self {
                Self(v.into())
            }
        }
        impl Deref for Ty {
            type Target = str;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_integer() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Integer,
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty(u32);
        impl From<u32> for Ty {
            fn from(v: u32) -> Self {
                Self(v)
            }
        }
        impl Deref for Ty {
            type Target = u32;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_number() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Number,
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty(f64);
        impl From<f64> for Ty {
            fn from(v: f64) -> Self {
                Self(v)
            }
        }
        impl Deref for Ty {
            type Target = f64;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
    assert_eq(&prog, &expect);
}

/*
#[test]
fn test_domain_type_bool() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Boolean,
    };
    let prog = Context::new_root(&Default::default()).with_domain("foo").render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty(bool);
        impl From<bool> for Ty {
            fn from(v: bool) -> Self {
                Self(v)
            }
        }
        impl Deref for Ty {
            type Target = bool;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
    assert_eq(prog.to_string(), expect.to_string());
}
*/

#[test]
fn test_domain_type_array() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Array(Box::new(Type::String)),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty(Vec<String>);
        impl FromIterator<String> for Ty {
            fn from_iter<T>(iter: T) -> Self where T: IntoIterator<Item = String> {
                Self(FromIterator::from_iter(iter))
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_enum() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::EnumString(vec!["bar".into()]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum Ty {
            #[doc = "bar"]
            #[serde(rename = "bar")]
            Bar,
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_object() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Object(vec![
            Property {
                annotation: Default::default(),
                name: "string".into(),
                r#type: Type::String,
            },
            Property {
                annotation: Default::default(),
                name: "integer".into(),
                r#type: Type::Integer,
            },
            Property {
                annotation: Default::default(),
                name: "number".into(),
                r#type: Type::Number,
            },
            Property {
                annotation: Default::default(),
                name: "boolean".into(),
                r#type: Type::Boolean,
            },
            Property {
                annotation: Default::default(),
                name: "binary".into(),
                r#type: Type::Binary,
            },
            Property {
                annotation: Default::default(),
                name: "any".into(),
                r#type: Type::Any,
            },
            Property {
                annotation: Default::default(),
                name: "enum".into(),
                r#type: Type::EnumString(vec![]),
            },
            Property {
                annotation: Default::default(),
                name: "array".into(),
                r#type: Type::Array(Box::new(Type::String)),
            },
            Property {
                annotation: Default::default(),
                name: "ref".into(),
                r#type: Type::Ref("baz".to_string()),
            },
        ]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum TyEnum {
        }
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty {
            #[serde(rename = "string")]
            r#string: String,
            #[serde(rename = "integer")]
            r#integer: u32,
            #[serde(rename = "number")]
            r#number: f64,
            #[serde(rename = "boolean")]
            r#boolean: bool,
            #[serde(rename = "binary")]
            r#binary: String,
            #[serde(rename = "any")]
            r#any: JsonValue,
            #[serde(rename = "enum")]
            r#enum: TyEnum,
            #[serde(rename = "array")]
            r#array: Vec<String>,
            #[serde(rename = "ref")]
            r#ref: Box<Baz>,
        }
        impl Ty {
            pub fn builder() -> TyBuilder {
                Default::default()
            }
            pub fn r#string(&self) -> &str {
                &self.r#string
            }
            pub fn r#integer(&self) -> u32 {
                self.r#integer
            }
            pub fn r#number(&self) -> f64 {
                self.r#number
            }
            pub fn r#boolean(&self) -> bool {
                self.r#boolean
            }
            pub fn r#binary(&self) -> &str {
                &self.r#binary
            }
            pub fn r#any(&self) -> &JsonValue {
                &self.r#any
            }
            pub fn r#enum(&self) -> &TyEnum {
                &self.r#enum
            }
            pub fn r#array(&self) -> &[String] {
                &self.r#array
            }
            pub fn r#ref(&self) -> &Baz {
                &self.r#ref
            }
        }
        #[derive(Debug, Clone)]
        pub struct TyBuilder {
            r#string: Option<String>,
            r#integer: Option<u32>,
            r#number: Option<f64>,
            r#boolean: Option<bool>,
            r#binary: Option<String>,
            r#any: Option<JsonValue>,
            r#enum: Option<TyEnum>,
            r#array: Option<Vec<String>>,
            r#ref: Option<Baz>,
        }
        impl Default for TyBuilder {
            fn default() -> Self {
                Self {
                    r#string: None as Option<String>,
                    r#integer: None as Option<u32>,
                    r#number: None as Option<f64>,
                    r#boolean: None as Option<bool>,
                    r#binary: None as Option<String>,
                    r#any: None as Option<JsonValue>,
                    r#enum: None as Option<TyEnum>,
                    r#array: None as Option<Vec<String>>,
                    r#ref: None as Option<Baz>,
                }
            }
        }
        impl TyBuilder {
            pub fn r#string(&mut self, v: String) -> &mut Self {
                self.r#string = Some(v);
                self
            }
            pub fn r#integer(&mut self, v: u32) -> &mut Self {
                self.r#integer = Some(v);
                self
            }
            pub fn r#number(&mut self, v: f64) -> &mut Self {
                self.r#number = Some(v);
                self
            }
            pub fn r#boolean(&mut self, v: bool) -> &mut Self {
                self.r#boolean = Some(v);
                self
            }
            pub fn r#binary(&mut self, v: String) -> &mut Self {
                self.r#binary = Some(v);
                self
            }
            pub fn r#any(&mut self, v: JsonValue) -> &mut Self {
                self.r#any = Some(v);
                self
            }
            pub fn r#enum(&mut self, v: TyEnum) -> &mut Self {
                self.r#enum = Some(v);
                self
            }
            pub fn r#array(&mut self, v: Vec<String>) -> &mut Self {
                self.r#array = Some(v);
                self
            }
            pub fn r#ref(&mut self, v: Baz) -> &mut Self {
                self.r#ref = Some(v);
                self
            }
            pub fn build(&mut self) -> Result<Ty, &'static str> {
                let r#string = self.r#string.take();
                let r#integer = self.r#integer.take();
                let r#number = self.r#number.take();
                let r#boolean = self.r#boolean.take();
                let r#binary = self.r#binary.take();
                let r#any = self.r#any.take();
                let r#enum = self.r#enum.take();
                let r#array = self.r#array.take();
                let r#ref = self.r#ref.take();
                Ok(Ty {
                    r#string: r#string.ok_or("needs r#string")?,
                    r#integer: r#integer.ok_or("needs r#integer")?,
                    r#number: r#number.ok_or("needs r#number")?,
                    r#boolean: r#boolean.ok_or("needs r#boolean")?,
                    r#binary: r#binary.ok_or("needs r#binary")?,
                    r#any: r#any.ok_or("needs r#any")?,
                    r#enum: r#enum.ok_or("needs r#enum")?,
                    r#array: r#array.ok_or("needs r#array")?,
                    r#ref: Box::new(r#ref.ok_or("needs r#ref")?),
                })
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_object_object() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Object(vec![Property {
            annotation: Default::default(),
            name: "object".into(),
            r#type: Type::Object(vec![]),
        }]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct TyObject {}
        impl TyObject {
            pub fn new() -> Self {
                Default::default()
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty {
            #[serde(rename = "object")]
            r#object: TyObject,
        }
        impl Ty {
            pub fn new(r#object: TyObject) -> Self {
                Self {
                    r#object,
                }
            }
            pub fn r#object(&self) -> &TyObject {
                &self.r#object
            }
        }
        impl Deref for Ty {
            type Target = TyObject;
            fn deref(&self) -> &Self::Target {
                &self.r#object
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_object_option() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Object(vec![
            Property {
                annotation: Default::default(),
                name: "string".into(),
                r#type: Type::Option(Box::new(Type::String)),
            },
            Property {
                annotation: Default::default(),
                name: "integer".into(),
                r#type: Type::Option(Box::new(Type::Integer)),
            },
            Property {
                annotation: Default::default(),
                name: "number".into(),
                r#type: Type::Option(Box::new(Type::Number)),
            },
            Property {
                annotation: Default::default(),
                name: "boolean".into(),
                r#type: Type::Option(Box::new(Type::Boolean)),
            },
            Property {
                annotation: Default::default(),
                name: "binary".into(),
                r#type: Type::Option(Box::new(Type::Binary)),
            },
            Property {
                annotation: Default::default(),
                name: "any".into(),
                r#type: Type::Option(Box::new(Type::Any)),
            },
            Property {
                annotation: Default::default(),
                name: "enum".into(),
                r#type: Type::Option(Box::new(Type::EnumString(vec![]))),
            },
            Property {
                annotation: Default::default(),
                name: "array".into(),
                r#type: Type::Option(Box::new(Type::Array(Box::new(Type::String)))),
            },
            Property {
                annotation: Default::default(),
                name: "ref".into(),
                r#type: Type::Option(Box::new(Type::Ref("baz".to_string()))),
            },
        ]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum TyEnum {
        }
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty {
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "string")]
            r#string: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "integer")]
            r#integer: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "number")]
            r#number: Option<f64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "boolean")]
            r#boolean: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "binary")]
            r#binary: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "any")]
            r#any: Option<JsonValue>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "enum")]
            r#enum: Option<TyEnum>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "array")]
            r#array: Option<Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "ref")]
            r#ref: Box<Option<Baz>>,
        }
        impl Ty {
            pub fn builder() -> TyBuilder {
                Default::default()
            }
            pub fn r#string(&self) -> Option<&String> {
                self.r#string.as_ref()
            }
            pub fn r#integer(&self) -> Option<&u32> {
                self.r#integer.as_ref()
            }
            pub fn r#number(&self) -> Option<&f64> {
                self.r#number.as_ref()
            }
            pub fn r#boolean(&self) -> Option<&bool> {
                self.r#boolean.as_ref()
            }
            pub fn r#binary(&self) -> Option<&String> {
                self.r#binary.as_ref()
            }
            pub fn r#any(&self) -> Option<&JsonValue> {
                self.r#any.as_ref()
            }
            pub fn r#enum(&self) -> Option<&TyEnum> {
                self.r#enum.as_ref()
            }
            pub fn r#array(&self) -> Option<&Vec<String>> {
                self.r#array.as_ref()
            }
            pub fn r#ref(&self) -> Option<&Baz> {
                (&*self.r#ref).as_ref()
            }
        }
        #[derive(Debug, Clone)]
        pub struct TyBuilder {
            r#string: Option<String>,
            r#integer: Option<u32>,
            r#number: Option<f64>,
            r#boolean: Option<bool>,
            r#binary: Option<String>,
            r#any: Option<JsonValue>,
            r#enum: Option<TyEnum>,
            r#array: Option<Vec<String>>,
            r#ref: Option<Baz>,
        }
        impl Default for TyBuilder {
            fn default() -> Self {
                Self {
                    r#string: None as Option<String>,
                    r#integer: None as Option<u32>,
                    r#number: None as Option<f64>,
                    r#boolean: None as Option<bool>,
                    r#binary: None as Option<String>,
                    r#any: None as Option<JsonValue>,
                    r#enum: None as Option<TyEnum>,
                    r#array: None as Option<Vec<String>>,
                    r#ref: None as Option<Baz>,
                }
            }
        }
        impl TyBuilder {
            pub fn r#string(&mut self, v: String) -> &mut Self {
                self.r#string = Some(v);
                self
            }
            pub fn r#integer(&mut self, v: u32) -> &mut Self {
                self.r#integer = Some(v);
                self
            }
            pub fn r#number(&mut self, v: f64) -> &mut Self {
                self.r#number = Some(v);
                self
            }
            pub fn r#boolean(&mut self, v: bool) -> &mut Self {
                self.r#boolean = Some(v);
                self
            }
            pub fn r#binary(&mut self, v: String) -> &mut Self {
                self.r#binary = Some(v);
                self
            }
            pub fn r#any(&mut self, v: JsonValue) -> &mut Self {
                self.r#any = Some(v);
                self
            }
            pub fn r#enum(&mut self, v: TyEnum) -> &mut Self {
                self.r#enum = Some(v);
                self
            }
            pub fn r#array(&mut self, v: Vec<String>) -> &mut Self {
                self.r#array = Some(v);
                self
            }
            pub fn r#ref(&mut self, v: Baz) -> &mut Self {
                self.r#ref = Some(v);
                self
            }
            pub fn build(&mut self) -> Result<Ty, &'static str> {
                let r#string = self.r#string.take();
                let r#integer = self.r#integer.take();
                let r#number = self.r#number.take();
                let r#boolean = self.r#boolean.take();
                let r#binary = self.r#binary.take();
                let r#any = self.r#any.take();
                let r#enum = self.r#enum.take();
                let r#array = self.r#array.take();
                let r#ref = self.r#ref.take();
                Ok(Ty {
                    r#string,
                    r#integer,
                    r#number,
                    r#boolean,
                    r#binary,
                    r#any,
                    r#enum,
                    r#array,
                    r#ref: Box::new(r#ref),
                })
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_object_option_object() {
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Object(vec![Property {
            annotation: Default::default(),
            name: "object".into(),
            r#type: Type::Option(Box::new(Type::Object(vec![]))),
        }]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct TyObject {}
        impl TyObject {
            pub fn new() -> Self {
                Default::default()
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty {
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "object")]
            r#object: Option<TyObject>,
        }
        impl Ty {
            pub fn new(r#object: Option<TyObject>) -> Self {
                Self {
                    r#object,
                }
            }
            pub fn r#object(&self) -> Option<&TyObject> {
                self.r#object.as_ref()
            }
        }
        impl Deref for Ty {
            type Target = Option<TyObject>;
            fn deref(&self) -> &Self::Target {
                &self.r#object
            }
        }
    };
    assert_eq(&prog, &expect);
}

#[test]
fn test_domain_type_object_array_object() {
    // FIXME
    let ty = DomainType {
        annotation: Default::default(),
        id: "ty".to_string(),
        r#type: Type::Object(vec![Property {
            annotation: Default::default(),
            name: "object".into(),
            r#type: Type::Array(Box::new(Type::Object(vec![]))),
        }]),
    };
    let prog = Context::new_root(&Default::default())
        .with_domain("foo")
        .render_with(&ty);
    let expect = quote! {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct TyObject {}
        impl TyObject {
            pub fn new() -> Self {
                Default::default()
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct Ty {
            #[serde(rename = "object")]
            r#object: Vec<TyObject>,
        }
        impl Ty {
            pub fn new(r#object: Vec<TyObject>) -> Self {
                Self {
                    r#object,
                }
            }
            pub fn r#object(&self) -> &[TyObject] {
                &self.r#object
            }
        }
        impl Deref for Ty {
            type Target = Vec<TyObject>;
            fn deref(&self) -> &Self::Target {
                &self.r#object
            }
        }
    };
    assert_eq(&prog, &expect);
}
