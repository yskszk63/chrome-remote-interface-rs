use super::*;
use heck::{CamelCase, SnakeCase};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashSet;

fn find_experimental_types(protocol: &Protocol) -> HashSet<(String, String)> {
    protocol
        .domains
        .iter()
        .flat_map(|d| d.types.iter().map(move |t| (d, t)))
        .filter(|(_, t)| t.annotation().experimental)
        .map(|(d, t)| (d.domain.clone(), t.id.clone()))
        .collect()
}

trait AnnotatableExt: Annotatable {
    fn meta(&self) -> TokenStream {
        let a = self.annotation();
        let depr = if a.deprecated {
            quote! { #[deprecated] }
        } else {
            quote! {}
        };
        let expr = self.meta_for_trait_impl();

        quote! {
            #depr
            #expr
        }
    }

    fn meta_for_trait_impl(&self) -> TokenStream {
        let a = self.annotation();
        match (a.experimental, self.deps().as_deref()) {
            (true, Some(deps)) => {
                quote! {
                    #[cfg(all(feature = "experimental", #(feature = #deps),*))]
                    #[cfg_attr(docsrs, doc(cfg(all(feature = "experimental", #(feature = #deps),*))))]
                }
            }
            (true, None) => {
                quote! {
                    #[cfg(feature = "experimental")]
                    #[cfg_attr(docsrs, doc(cfg(feature = "experimental")))]
                }
            }
            (false, Some(deps)) => {
                quote! {
                    #[cfg(all(#(feature = #deps),*))]
                    #[cfg_attr(docsrs, doc(cfg(all(#(feature = #deps),*))))]
                }
            }
            (false, None) => {
                quote! {}
            }
        }
    }

    fn doc(&self) -> TokenStream {
        let a = self.annotation();
        if let Some(d) = &a.description {
            quote! { #[doc = #d] }
        } else {
            quote! {}
        }
    }
}

impl<T> AnnotatableExt for T where T: Annotatable {}

#[derive(Debug)]
struct Context<'a> {
    experimentals: &'a HashSet<(String, String)>,
    parent: Option<&'a Context<'a>>,
    domain: Option<String>,
    name: Option<String>,
}

impl<'a> Context<'a> {
    fn new_root(experimentals: &'a HashSet<(String, String)>) -> Self {
        Self {
            experimentals,
            parent: None,
            domain: None,
            name: None,
        }
    }

    fn with_none(&'a self) -> Self {
        Self {
            experimentals: &self.experimentals,
            parent: Some(self),
            domain: None,
            name: None,
        }
    }

    fn with_domain<S>(&'a self, domain: S) -> Self
    where
        S: ToString,
    {
        Self {
            experimentals: &self.experimentals,
            parent: Some(self),
            domain: Some(domain.to_string()),
            name: None,
        }
    }

    fn with_name<S>(&'a self, name: S) -> Self
    where
        S: ToString,
    {
        Self {
            experimentals: &self.experimentals,
            parent: Some(self),
            domain: None,
            name: Some(name.to_string()),
        }
    }

    fn render_with<R>(&self, rendarable: &R) -> TokenStream
    where
        R: Rendarable,
    {
        let cx = rendarable.enter(self);
        rendarable.render(&cx)
    }

    fn iter(&self) -> ContextIter<'_, '_> {
        self.into_iter()
    }

    fn type_name(&self) -> Ident {
        let mut name = self
            .iter()
            .filter_map(|v| v.name.as_deref())
            .collect::<Vec<_>>();
        name.reverse();
        format_ident!("{}", name.join(""))
    }

    fn domain(&self) -> String {
        self.iter()
            .find_map(|v| v.domain.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
struct ContextIter<'a, 'b>(Option<&'b Context<'a>>);

impl<'a, 'b> Iterator for ContextIter<'a, 'b> {
    type Item = &'b Context<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(v) = self.0 {
            self.0 = v.parent;
            return Some(v);
        }
        None
    }
}

impl<'a, 'b> IntoIterator for &'b Context<'a> {
    type IntoIter = ContextIter<'a, 'b>;
    type Item = &'b Context<'a>;

    fn into_iter(self) -> Self::IntoIter {
        ContextIter(Some(self))
    }
}

trait Rendarable {
    fn render(&self, cx: &Context) -> TokenStream;

    fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
        cx.with_none()
    }
}

impl<R> Rendarable for Vec<R>
where
    R: Rendarable,
{
    fn render(&self, cx: &Context) -> TokenStream {
        self.iter().map(|i| cx.render_with(i)).collect()
    }
}

impl Type {
    fn need_box(&self) -> bool {
        match self {
            Type::Ref(..) => true,
            Type::Option(inner) => inner.need_box(),
            _ => false,
        }
    }
}

#[derive(Debug)]
struct NestedType<'a> {
    suffix: Option<String>,
    r#type: &'a Type,
}

impl<'a> NestedType<'a> {
    fn new(r#type: &'a Type) -> Self {
        Self {
            suffix: None,
            r#type,
        }
    }

    fn new_with_suffix(r#type: &'a Type, suffix: &str) -> Self {
        Self {
            suffix: Some(suffix.into()),
            r#type,
        }
    }
}

impl NestedType<'_> {
    fn ty(&self, cx: &Context<'_>) -> TokenStream {
        let name = cx.type_name();
        match self.r#type() {
            Type::String | Type::Binary => quote! { String },
            Type::EnumString(..) | Type::Object(..) => quote! { #name },
            Type::Integer => quote! { u32 },
            Type::Number => quote! { f64 },
            Type::Boolean => quote! { bool },
            Type::Array(item) => {
                let item = NestedType::new(item).ty(cx);
                quote! { Vec<#item> }
            }
            Type::Option(item) => {
                let item = NestedType::new(item).ty(cx);
                quote! { Option<#item> }
            }
            Type::Any => quote! { JsonValue },
            Type::Ref(r) => {
                let mut r = r.split('.');
                match (r.next(), r.next()) {
                    (Some(domain), Some(typ))
                        if cx
                            .experimentals
                            .contains(&(domain.to_string(), typ.to_string())) =>
                    {
                        quote! { JsonValue }
                    }
                    (Some(typ), None)
                        if cx.experimentals.contains(&(cx.domain(), typ.to_string())) =>
                    {
                        quote! { JsonValue }
                    }
                    (Some(domain), Some(typ)) => {
                        let domain = format_ident!("{}", domain.to_snake_case());
                        let typ = format_ident!("{}", typ.to_camel_case());
                        quote! { #domain::#typ }
                    }
                    (Some(typ), None) => {
                        let typ = format_ident!("{}", typ.to_camel_case());
                        quote! { #typ }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn tyref(&self, cx: &Context<'_>) -> TokenStream {
        let name = cx.type_name();
        match self.r#type() {
            Type::String | Type::Binary => quote! { &str },
            Type::EnumString(..) | Type::Object(..) => quote! { &#name },
            Type::Integer => quote! { u32 },
            Type::Number => quote! { f64 },
            Type::Boolean => quote! { bool },
            Type::Array(item) => {
                let item = NestedType::new(item).ty(&cx);
                quote! { &[#item] }
            }
            Type::Option(item) => {
                let item = NestedType::new(item).ty(&cx);
                quote! { Option<&#item> }
            }
            Type::Any => quote! { &JsonValue },
            Type::Ref(r) => {
                let mut r = r.split('.');
                match (r.next(), r.next()) {
                    (Some(domain), Some(typ))
                        if cx
                            .experimentals
                            .contains(&(domain.to_string(), typ.to_string())) =>
                    {
                        quote! { &JsonValue }
                    }
                    (Some(typ), None)
                        if cx.experimentals.contains(&(cx.domain(), typ.to_string())) =>
                    {
                        quote! { &JsonValue }
                    }
                    (Some(domain), Some(typ)) => {
                        let domain = format_ident!("{}", domain.to_snake_case());
                        let typ = format_ident!("{}", typ.to_camel_case());
                        quote! { &#domain::#typ }
                    }
                    (Some(typ), None) => {
                        let typ = format_ident!("{}", typ.to_camel_case());
                        quote! { &#typ }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn refexpr(&self, ident: &Ident) -> TokenStream {
        match self.r#type() {
            Type::String
            | Type::Binary
            | Type::EnumString(..)
            | Type::Object(..)
            | Type::Any
            | Type::Ref(..) => quote! { &self.#ident },
            Type::Integer | Type::Number | Type::Boolean => quote! { self.#ident },
            Type::Array(..) => quote! { &self.#ident },
            Type::Option(inner) => {
                if inner.need_box() {
                    quote! { (&*self.#ident).as_ref() }
                } else {
                    quote! { self.#ident.as_ref() }
                }
            }
        }
    }

    fn builder_ty(&self, cx: &Context<'_>) -> TokenStream {
        let ty = self.ty(&cx);
        if let Type::Option(..) = self.r#type() {
            ty
        } else {
            quote! { Option<#ty> }
        }
    }

    fn builder_setty(&self, cx: &Context<'_>) -> TokenStream {
        if let Type::Option(item) = self.r#type() {
            NestedType::new(item).ty(&cx)
        } else {
            self.ty(&cx)
        }
    }
}

impl Rendarable for NestedType<'_> {
    fn render(&self, cx: &Context) -> TokenStream {
        match self.r#type() {
            Type::EnumString(..) | Type::Object(..) => self.render_type_definition(cx),
            Type::Array(item) => cx.render_with(&NestedType::new(item)),
            Type::Option(item) => cx.render_with(&NestedType::new(item)),
            _ => Default::default(),
        }
    }

    fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
        if let Some(s) = &self.suffix {
            cx.with_name(s)
        } else {
            cx.with_none()
        }
    }
}

impl Typed for NestedType<'_> {
    fn r#type(&self) -> &Type {
        self.r#type
    }
}

impl<'a> Annotatable for NestedType<'a> {
    fn annotation(&self) -> &Annotation {
        const EMPTY: Annotation = Annotation {
            description: None,
            deprecated: false,
            experimental: false,
        };
        &EMPTY
    }
}

trait Field: AnnotatableExt + Typed {
    fn name(&self) -> &str;

    fn nested_type(&self) -> NestedType {
        NestedType::new_with_suffix(self.r#type(), &self.name().to_camel_case())
    }

    fn ident(&self) -> Ident {
        format_ident!("r#{}", self.name().to_snake_case())
    }

    fn render_field_declaration(&self, cx: &Context<'_>) -> TokenStream {
        let cx = self.nested_type().enter(cx);
        let doc = self.doc();
        let meta = self.meta();
        let meta = if let Type::Option(..) = self.r#type() {
            quote! {
                #meta
                #[serde(skip_serializing_if = "Option::is_none")]
            }
        } else {
            meta
        };
        let name = self.name();
        let ident = self.ident();
        let ty = self.nested_type().ty(&cx);
        let ty = if self.need_box() {
            quote! { Box<#ty> }
        } else {
            ty
        };

        quote! {
            #doc
            #meta
            #[serde(rename = #name)]
            #ident: #ty
        }
    }

    fn render_getter(&self, cx: &Context<'_>) -> TokenStream {
        let cx = self.nested_type().enter(cx);
        let doc = self.doc();
        let meta = self.meta();
        let ident = self.ident();
        let ty = self.nested_type().tyref(&cx);
        let expr = self.nested_type().refexpr(&ident);

        quote! {
            #doc
            #meta
            pub fn #ident(&self) -> #ty {
                #expr
            }
        }
    }

    fn render_builder_field_declaration(&self, cx: &Context<'_>) -> TokenStream {
        let cx = self.nested_type().enter(cx);
        let doc = self.doc();
        let meta = self.meta();
        let ident = self.ident();
        let ty = self.nested_type().builder_ty(&cx);

        quote! {
            #doc
            #meta
            #ident: #ty
        }
    }

    fn render_builder_setter(&self, cx: &Context<'_>) -> TokenStream {
        let cx = self.nested_type().enter(cx);
        let doc = self.doc();
        let meta = self.meta();
        let ident = self.ident();
        let ty = self.nested_type().builder_setty(&cx);

        quote! {
            #doc
            #meta
            pub fn #ident(&mut self, v: #ty) -> &mut Self {
                self.#ident = Some(v);
                self
            }
        }
    }

    fn render_builder_contract(&self, cx: &Context<'_>) -> TokenStream {
        let cx = self.nested_type().enter(cx);
        let ident = self.ident();
        let meta = self.meta_for_trait_impl();
        let ty = self.nested_type().builder_ty(&cx);

        quote! {
            #meta
            #ident: None as #ty
        }
    }

    fn render_builder_take(&self) -> TokenStream {
        let ident = self.ident();
        let meta = self.meta_for_trait_impl();

        quote! {
            #meta
            let #ident = self.#ident.take();
        }
    }

    fn render_builder_assign(&self) -> TokenStream {
        let ident = self.ident();
        let meta = self.meta_for_trait_impl();
        let pair = if let Type::Option(..) = self.r#type() {
            if self.need_box() {
                quote! { #ident: Box::new(#ident) }
            } else {
                quote! { #ident }
            }
        } else {
            let msg = format!("needs {}", ident);
            if self.need_box() {
                quote! { #ident: Box::new(#ident.ok_or(#msg)?) }
            } else {
                quote! { #ident: #ident.ok_or(#msg)? }
            }
        };

        quote! {
            #meta
            #pair
        }
    }

    fn need_box(&self) -> bool {
        self.r#type().need_box()
    }
}

impl Field for Property {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Field for Parameter {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Field for Return {
    fn name(&self) -> &str {
        &self.name
    }
}

fn render_struct<A, F>(cx: &Context<'_>, annotatable: &A, fields: &[F]) -> TokenStream
where
    A: Annotatable,
    F: Field,
{
    match fields {
        [] => {
            let doc = annotatable.doc();
            let meta = annotatable.meta();
            let name = cx.type_name();

            quote! {
                #doc
                #meta
                #[derive(Debug, Clone, Default, Serialize, Deserialize)]
                pub struct #name {}

                #meta
                impl #name {
                    pub fn new() -> Self {
                        Default::default()
                    }
                }
            }
        }

        [field] => {
            let doc = annotatable.doc();
            let meta = annotatable.meta();
            let meta_for_trait_impl = annotatable.meta_for_trait_impl();
            let name = cx.type_name();
            let field_decl = field.render_field_declaration(cx);
            let getter = field.render_getter(cx);

            let nestedty =
                NestedType::new_with_suffix(field.r#type(), &field.name().to_camel_case());
            let nestedty = cx.render_with(&nestedty);

            let field_ident = field.ident();
            let field_bare_ty = field.nested_type().ty(&field.nested_type().enter(cx));
            let field_meta = field.meta_for_trait_impl();
            let assign = if field.need_box() {
                quote! { #field_ident: Box::new(#field_ident) }
            } else {
                quote! { #field_ident }
            };

            quote! {
                #nestedty

                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name {
                    #field_decl,
                }

                #meta
                impl #name {
                    pub fn new(#field_meta #field_ident : #field_bare_ty) -> Self {
                        Self {
                            #field_meta
                            #assign
                        }
                    }
                    #getter
                }

                #meta_for_trait_impl
                #field_meta
                impl Deref for #name {
                    type Target = #field_bare_ty;
                    fn deref(&self) -> &Self::Target {
                        &self.#field_ident
                    }
                }
            }
        }

        [..] if fields.len() <= 3 => {
            let doc = annotatable.doc();
            let meta = annotatable.meta();
            let name = cx.type_name();
            let field_decls = fields.iter().map(|f| f.render_field_declaration(cx));
            let getters = fields.iter().map(|f| f.render_getter(cx));

            let nestedtys = fields
                .iter()
                .map(|p| NestedType::new_with_suffix(p.r#type(), &p.name().to_camel_case()))
                .collect::<Vec<_>>();
            let nestedtys = nestedtys.render(cx); // FIXME

            let field_idents = fields.iter().map(|field| field.ident()).collect::<Vec<_>>();
            let field_bare_tys = fields
                .iter()
                .map(|field| field.nested_type().ty(&field.nested_type().enter(cx)))
                .collect::<Vec<_>>();
            let field_metas = fields
                .iter()
                .map(|field| field.meta_for_trait_impl())
                .collect::<Vec<_>>();
            let assigns = fields
                .iter()
                .map(|field| {
                    let field_ident = field.ident();
                    if field.need_box() {
                        quote! { #field_ident: Box::new(#field_ident) }
                    } else {
                        quote! { #field_ident }
                    }
                })
                .collect::<Vec<_>>();

            quote! {
                #nestedtys

                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name {
                    #(#field_decls,)*
                }

                #meta
                impl #name {
                    pub fn new(#(#field_metas #field_idents : #field_bare_tys,)*) -> Self {
                        Self {
                            #(#field_metas #assigns,)*
                        }
                    }
                    #(#getters)*
                }
            }
        }

        [..] => {
            let doc = annotatable.doc();
            let meta = annotatable.meta();
            let meta_for_trait_impl = annotatable.meta_for_trait_impl();
            let name = cx.type_name();
            let builder_name = format_ident!("{}Builder", name);

            let nestedtys = fields
                .iter()
                .map(|p| NestedType::new_with_suffix(p.r#type(), &p.name().to_camel_case()))
                .collect::<Vec<_>>();
            let nestedtys = nestedtys.render(cx); // FIXME

            let field_decls = fields.iter().map(|f| f.render_field_declaration(cx));
            let getters = fields.iter().map(|f| f.render_getter(cx));

            let builder_field_decls = fields
                .iter()
                .map(|f| f.render_builder_field_declaration(cx));
            let builder_contracts = fields.iter().map(|f| f.render_builder_contract(cx));
            let builder_setters = fields.iter().map(|f| f.render_builder_setter(cx));
            let builder_take = fields.iter().map(|f| f.render_builder_take());
            let builder_assign = fields.iter().map(|f| f.render_builder_assign());

            quote! {
                #nestedtys

                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name {
                    #(#field_decls,)*
                }

                #meta
                impl #name {
                    pub fn builder() -> #builder_name {
                        Default::default()
                    }
                    #(#getters)*
                }

                #meta
                #[derive(Debug, Clone)]
                pub struct #builder_name {
                    #(#builder_field_decls,)*
                }

                #meta_for_trait_impl
                impl Default for #builder_name {
                    fn default() -> Self {
                        Self {
                            #(#builder_contracts,)*
                        }
                    }
                }

                #meta
                impl #builder_name {
                    #(#builder_setters)*

                    pub fn build(&mut self) -> Result<#name, &'static str> {
                        #(#builder_take)*
                        Ok(#name {
                            #(#builder_assign,)*
                        })
                    }
                }
            }
        }
    }
}

trait TypedExt: Typed + AnnotatableExt + Sized {
    fn render_type_definition(&self, cx: &Context) -> TokenStream {
        let doc = self.doc();
        let meta = self.meta();
        let meta_for_trait_impl = self.meta_for_trait_impl();
        let name = cx.type_name();

        match self.r#type() {
            Type::String => quote! {
                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name(String);

                #meta_for_trait_impl
                impl From<String> for #name {
                    fn from(v: String) -> Self {
                        Self(v)
                    }
                }

                #meta_for_trait_impl
                impl From<&str> for #name {
                    fn from(v: &str) -> Self {
                        Self(v.into())
                    }
                }

                #meta_for_trait_impl
                impl Deref for #name {
                    type Target = str;
                    fn deref(&self) -> &Self::Target {
                        &self.0
                    }
                }
            },

            Type::Integer => quote! {
                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name(u32);

                #meta_for_trait_impl
                impl From<u32> for #name {
                    fn from(v: u32) -> Self {
                        Self(v)
                    }
                }

                #meta_for_trait_impl
                impl Deref for #name {
                    type Target = u32;
                    fn deref(&self) -> &Self::Target {
                        &self.0
                    }
                }
            },

            Type::Number => quote! {
                #doc
                #meta
                #[derive(Debug, Clone, Serialize, Deserialize)]
                pub struct #name(f64);

                #meta_for_trait_impl
                impl From<f64> for #name {
                    fn from(v: f64) -> Self {
                        Self(v)
                    }
                }

                #meta_for_trait_impl
                impl Deref for #name {
                    type Target = f64;
                    fn deref(&self) -> &Self::Target {
                        &self.0
                    }
                }
            },

            Type::Array(item) => {
                let item = NestedType::new(item);
                let ty = cx.render_with(&item);
                let item = item.ty(cx);

                quote! {
                    #ty

                    #doc
                    #meta
                    #[derive(Debug, Clone, Serialize, Deserialize)]
                    pub struct #name(Vec<#item>);

                    #meta_for_trait_impl
                    impl FromIterator<#item> for #name {
                        fn from_iter<T>(iter: T) -> Self where T: IntoIterator<Item = #item> {
                            Self(FromIterator::from_iter(iter))
                        }
                    }
                }
            }

            Type::EnumString(items) => {
                let variants = items
                    .iter()
                    .map(|i| format_ident!("{}", i.to_camel_case()))
                    .collect::<Vec<_>>();
                quote! {
                    #meta
                    #[derive(Debug, Clone, Serialize, Deserialize)]
                    pub enum #name {
                        #(
                            #[doc = #items]
                            #[serde(rename = #items)]
                            #variants,
                        )*
                    }
                }
            }

            Type::Object(properties) => render_struct(cx, self, properties),

            e => unimplemented!("{:?}", e),
        }
    }
}

impl<T> TypedExt for T where T: Typed + Annotatable {}

mod domain_type {
    use super::*;

    impl Rendarable for DomainType {
        fn render(&self, cx: &Context) -> TokenStream {
            self.render_type_definition(cx)
        }

        fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
            cx.with_name(self.id.to_camel_case())
        }
    }
}

mod command {
    use super::*;

    impl Command {
        fn pname(&self) -> String {
            format!("{}Command", self.name.to_camel_case())
        }

        fn rname(&self) -> String {
            format!("{}Return", self.name.to_camel_case())
        }

        fn pident(&self) -> Ident {
            format_ident!("{}", self.pname())
        }

        fn rident(&self) -> Ident {
            format_ident!("{}", self.rname())
        }
    }

    #[derive(Debug)]
    struct CommandParameter<'a> {
        command: &'a Command,
    }

    impl Rendarable for CommandParameter<'_> {
        fn render(&self, cx: &Context) -> TokenStream {
            let pident = self.command.pident();
            let rident = self.command.rident();
            let meta_for_trait_impl = self.command.meta_for_trait_impl();
            let fullname = format!("{}.{}", cx.domain(), self.command.name);
            let body = render_struct(cx, self.command, &self.command.parameters);

            quote! {
                #body

                #meta_for_trait_impl
                impl Command for #pident {
                    type Return = #rident;
                    const METHOD: &'static str = #fullname;
                }
            }
        }

        fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
            cx.with_name(self.command.pname())
        }
    }

    #[derive(Debug)]
    struct CommandReturn<'a> {
        command: &'a Command,
    }

    impl Rendarable for CommandReturn<'_> {
        fn render(&self, cx: &Context) -> TokenStream {
            render_struct(cx, self.command, &self.command.returns)
        }

        fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
            cx.with_name(self.command.rname())
        }
    }

    impl Rendarable for Command {
        fn render(&self, cx: &Context) -> TokenStream {
            let param = cx.render_with(&CommandParameter { command: self });
            let ret = cx.render_with(&CommandReturn { command: self });

            quote! {
                #param
                #ret
            }
        }
    }
}

mod event {
    use super::*;

    impl Rendarable for Event {
        fn render(&self, cx: &Context) -> TokenStream {
            render_struct(cx, self, &self.parameters)
        }

        fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
            cx.with_name(format!("{}Event", self.name.to_camel_case()))
        }
    }
}

mod domain {
    use super::*;

    impl Domain {
        fn experimental(&self) -> bool {
            !self.types.is_empty()
                && self.commands.is_empty()
                && self.types.is_empty()
                && self.types.iter().all(|t| t.annotation().experimental)
                && self.commands.iter().all(|c| c.annotation().experimental)
                && self.events.iter().all(|e| e.annotation().experimental)
        }
    }

    impl Rendarable for Domain {
        fn render(&self, cx: &Context) -> TokenStream {
            let doc = self.doc();
            let meta = self.meta();
            let experimental = if self.experimental() {
                quote! { #[cfg(feature = "experimental")] }
            } else {
                quote! {}
            };
            let name = format_ident!("{}", self.domain.to_snake_case());
            let types = cx.render_with(&self.types);
            let commands = cx.render_with(&self.commands);
            let events = cx.render_with(&self.events);

            quote! {
                #doc
                #meta
                #experimental
                pub mod #name {
                    use super::*;

                    #types
                    #commands
                    #events
                }
            }
        }

        fn enter<'a>(&self, cx: &'a Context<'a>) -> Context<'a> {
            cx.with_domain(&self.domain)
        }
    }
}

mod protocol_version {
    use super::*;

    impl Rendarable for ProtocolVersion {
        fn render(&self, _: &Context) -> TokenStream {
            let ProtocolVersion { major, minor } = &self;
            let version = format!("{}.{}", major, minor);
            quote! {
                #[doc = "Generating Chrome DevTools protocol version."]
                pub const VERSION: &str = #version;
            }
        }
    }
}

mod protocol {
    use super::*;

    fn render_common() -> TokenStream {
        quote! {
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
            pub struct Request<T> where T: Command, {
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
                        RawResponse::Event { session_id, event }
                            => Self::Event(session_id, event),
                        RawResponse::UnknownEvent { session_id, method, params, }
                            => Self::Event(session_id, Event::Unknown(method, params)),
                        RawResponse::Return { session_id, id, result }
                            => Self::Return(session_id, id, result),
                        RawResponse::Error { session_id, id, error, }
                            => Self::Error(session_id, id, error),
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
                fn into_request(self, session_id: Option<SessionId>, id: u32) -> Request<Self> where Self: Sized, {
                    Request {
                        session_id,
                        id,
                        method: Self::METHOD,
                        params: self,
                    }
                }
            }
        }
    }

    fn render_event(_cx: &Context<'_>, protocol: &Protocol) -> TokenStream {
        let variants = protocol
            .domains
            .iter()
            .flat_map(|d| d.events.iter().map(move |e| (d, e)))
            .map(|(domain, event)| {
                let dommeta = domain.meta();
                let doc = event.doc();
                let meta = event.meta();
                let fullname = format!("{}.{}", domain.domain, event.name);
                let ident = format_ident!(
                    "{}{}",
                    domain.domain.to_camel_case(),
                    event.name.to_camel_case()
                );
                let modident = format_ident!("{}", domain.domain.to_snake_case());
                let structident = format_ident!("{}Event", event.name.to_camel_case());

                quote! {
                    #doc
                    #dommeta
                    #meta
                    #[serde(rename = #fullname)]
                    #ident(#modident::#structident)
                }
            })
            .collect::<Vec<_>>();

        quote! {
            #[doc = "Chrome DevTools Protocol event."]
            #[derive(Debug, Clone, Deserialize)]
            #[serde(tag = "method", content = "params")]
            pub enum Event {
                #(#variants,)*
                #[doc = "Unknown event."]
                #[serde(skip)]
                Unknown(String, JsonValue),
            }
        }
    }

    impl Rendarable for Protocol {
        fn render(&self, cx: &Context) -> TokenStream {
            let common = render_common();
            let version = cx.render_with(&self.version);
            let domains = cx.render_with(&self.domains);
            let events = render_event(cx, self);

            quote! {
                mod g {
                    #common
                    #version
                    #domains
                    #events
                }
                pub use g::*;
            }
        }
    }
}

pub(super) fn render(protocol: &Protocol) -> TokenStream {
    let experimentals = find_experimental_types(protocol);
    let cx = Context::new_root(&experimentals);
    cx.render_with(protocol)
}

#[cfg(test)]
mod tests;
