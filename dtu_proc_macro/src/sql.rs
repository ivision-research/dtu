use std::ops::Deref;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::token::{And, Colon2, Gt, Lt};
use syn::{
    parse_macro_input, AttrStyle, Attribute, Field, GenericArgument, GenericParam, Generics,
    ItemStruct, Lifetime, LifetimeDef, Path, PathArguments, PathSegment, Token, Type, TypePath,
    TypeReference,
};

pub(crate) fn sql_db_row(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let st = parse_macro_input!(item as ItemStruct);
    let mut tokens = TokenStream::new();

    let attrs = &st.attrs;
    let vis = &st.vis;
    let ident = &st.ident;
    let generics = &st.generics;
    let fields = &st.fields;

    let diesel_attrs = attrs
        .iter()
        .filter(|att| {
            att.path
                .segments
                .first()
                .map_or(false, |s| s.ident.to_string() == "diesel")
        })
        .map(|att| att.clone())
        .collect::<Vec<Attribute>>();

    let mut kept_attrs = Vec::new();
    let mut dtu_attrs = None;

    for att in attrs {
        if attr_path_matches_simple(att, "dtu") {
            dtu_attrs = match att.parse_args::<DtuAttrs>() {
                Ok(v) => Some(v),
                Err(e) => {
                    e.to_compile_error().to_tokens(&mut tokens);
                    return tokens.into();
                }
            };
        } else {
            kept_attrs.push(att.clone());
        }
    }

    // TODO don't need this anymore
    let stripped_fields = fields
        .iter()
        .map(|f| {
            let mut field = (*f).clone();
            let attrs = field
                .attrs
                .iter()
                .map(|att| att.clone())
                .collect::<Vec<Attribute>>();
            field.attrs = attrs;
            field
        })
        .collect::<Vec<Field>>();

    let has_id = stripped_fields.iter().any(|it| {
        it.ident
            .as_ref()
            .map(|id| id.to_string() == "id")
            .unwrap_or(false)
    });

    define_insertable(
        &st,
        &stripped_fields,
        &diesel_attrs,
        &dtu_attrs,
        &mut tokens,
    );
    define_selectable(&st, &stripped_fields, &diesel_attrs, &mut tokens);

    let derives = if has_id {
        quote! {
            ::std::clone::Clone, ::diesel::Queryable, ::diesel::Identifiable, ::diesel::AsChangeset
        }
    } else {
        quote! {
            ::std::clone::Clone, ::diesel::Queryable, ::diesel::AsChangeset
        }
    };

    let code = quote! {
        #[cfg_attr(debug_assertions, derive(Debug, PartialEq))]
        #[derive(#derives)]
        #( #kept_attrs )*
        #vis struct #ident #generics {
            #(
                #stripped_fields,
            )*
        }
    };

    code.to_tokens(&mut tokens);

    tokens.into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DtuAttr {
    InsertKeepId,
}

struct DtuAttrs(Vec<DtuAttr>);
impl Deref for DtuAttrs {
    type Target = Vec<DtuAttr>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Parse for DtuAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "insert_keep_id" {
            return Ok(Self::InsertKeepId);
        }

        Err(syn::Error::new(
            input.span(),
            format!("unexpected dtu attr: {ident}"),
        ))
    }
}

impl Parse for DtuAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attrs = Vec::new();
        while !input.is_empty() {
            let attr: DtuAttr = input.parse()?;
            attrs.push(attr);
        }

        Ok(Self(attrs))
    }
}

struct DieselTableDefinition {
    table: Ident,
}

impl Parse for DieselTableDefinition {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident.to_string() != "table_name" {
            return Err(syn::Error::new(
                ident.span(),
                format!("expected `table_name` got {ident}"),
            ));
        }

        let _: Token![=] = input.parse()?;
        let table: Ident = input.parse()?;
        Ok(Self { table })
    }
}

fn attr_path_matches_simple(att: &Attribute, simple: &str) -> bool {
    let path = &att.path;
    if path.segments.len() != 1 {
        return false;
    }
    path.segments.first().unwrap().ident == Ident::new(simple, Span::call_site())
}

fn try_find_table_name(diesel_attrs: &Vec<Attribute>) -> Option<Ident> {
    if diesel_attrs.is_empty() {
        return None;
    }
    for att in diesel_attrs {
        if attr_path_matches_simple(att, "diesel") {
            return att
                .parse_args::<DieselTableDefinition>()
                .map(|it| it.table)
                .ok();
        }
    }
    None
}

fn define_selectable(
    st: &ItemStruct,
    stripped_fields: &Vec<Field>,
    diesel_attrs: &Vec<Attribute>,
    tokens: &mut TokenStream,
) {
    let name = &st.ident;
    let table_name = try_find_table_name(diesel_attrs).unwrap_or_else(|| to_table_name(name));
    let fields = stripped_fields
        .iter()
        .filter_map(|it| it.ident.clone())
        .collect::<Vec<_>>();

    let mut select: Punctuated<Path, Token![,]> = Punctuated::new();

    for field in fields {
        let mut segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
        segments.push(table_name.clone().into());
        segments.push(field.into());
        select.push(Path {
            leading_colon: None,
            segments,
        });
    }

    let code = quote! {
        impl<DB: ::diesel::backend::Backend> ::diesel::expression::Selectable<DB> for #name {
            type SelectExpression = (
                #select
            );

            fn construct_selection() -> Self::SelectExpression {
                (
                    #select
                )
            }

        }
    };

    code.to_tokens(tokens);
}

fn define_insertable(
    st: &ItemStruct,
    stripped_fields: &Vec<Field>,
    diesel_attrs: &Vec<Attribute>,
    dtu_attrs: &Option<DtuAttrs>,
    tokens: &mut TokenStream,
) {
    let name = &st.ident;
    let vis = &st.vis;
    let new_name = Ident::new(&format!("Insert{}", name), name.span());

    let atts = if !diesel_attrs.is_empty() {
        diesel_attrs.clone()
    } else {
        let table_name = to_table_name(&name);
        let tokens = quote! {
            (table_name = #table_name)
        };
        let mut segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
        segments.push(PathSegment {
            ident: Ident::new("diesel", Span::call_site()),
            arguments: PathArguments::None,
        });
        vec![Attribute {
            pound_token: Default::default(),
            style: AttrStyle::Outer,
            bracket_token: Default::default(),
            path: Path {
                leading_colon: None,
                segments,
            },
            tokens,
        }]
    };

    let keep_id = dtu_attrs
        .as_ref()
        .is_some_and(|it| it.contains(&DtuAttr::InsertKeepId));
    let mut has_string_or_class = false;

    let transformed_fields = stripped_fields
        .iter()
        .filter(|f| keep_id || (*f).ident.as_ref().unwrap().to_string() != "id")
        .map(|f| {
            if is_string(f) {
                has_string_or_class |= true;
                transform_type_to_ref(f, "str")
            } else if is_class_name(f) {
                has_string_or_class |= true;
                transform_type_to_ref(f, "ClassName")
            } else {
                return f.clone();
            }
        })
        .collect::<Vec<Field>>();

    let lifetime = if has_string_or_class {
        let mut params: Punctuated<GenericParam, Token![,]> = Punctuated::new();
        params.push(GenericParam::Lifetime(LifetimeDef::new(Lifetime::new(
            "'data",
            Span::call_site(),
        ))));
        Some(Generics {
            lt_token: Some(Lt::default()),
            params,
            gt_token: Some(Gt::default()),
            where_clause: None,
        })
    } else {
        None
    };

    let required_fields = transformed_fields
        .iter()
        .filter(|f| !is_option(*f))
        .map(|f| FieldArg::from_field(f))
        .collect::<Vec<FieldArg>>();

    let field_assignments = transformed_fields.iter().map(|f| InsertFieldAssignment {
        name: f.ident.as_ref().unwrap().clone(),
        has_arg: !is_option(f),
    });

    let setters = transformed_fields
        .iter()
        .map(|f| InsertSetter::from_field(f))
        .collect::<Vec<InsertSetter>>();

    let code = quote! {
        /// Auto generated type for inserting into the database
        #[derive(Insertable)]
        #(#atts)*
        #vis struct #new_name #lifetime {
            #(
                #transformed_fields,
            )*
        }

        impl #lifetime #new_name #lifetime {
            #vis fn new(
                #(
                    #required_fields,
                )*
            ) -> Self {
                Self {
                    #(
                        #field_assignments,
                    )*
                }
            }

            #(
                #setters
            )*
        }
    };

    code.to_tokens(tokens);
}

struct InsertSetter {
    name: Ident,
    ty: Type,
}

impl InsertSetter {
    fn from_field(f: &Field) -> Self {
        let name = f.ident.as_ref().unwrap().clone();
        let ty = f.ty.clone();
        Self { name, ty }
    }
}

impl ToTokens for InsertSetter {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ty = &self.ty;
        let field_name = &self.name;
        let func_name = Ident::new(&format!("set_{}", field_name), Span::call_site());
        (quote! {
            pub fn #func_name(mut self, value: #ty) -> Self {
                self.#field_name = value;
                self
            }
        })
        .to_tokens(tokens);
    }
}

struct InsertFieldAssignment {
    name: Ident,
    has_arg: bool,
}

impl ToTokens for InsertFieldAssignment {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self.has_arg {
            self.name.to_tokens(tokens);
        } else {
            let name = &self.name;
            (quote! {#name: None}).to_tokens(tokens)
        }
    }
}

struct FieldArg {
    name: Ident,
    ty: Type,
}

impl ToTokens for FieldArg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.name;
        let ty = &self.ty;
        (quote! {
            #name: #ty
        })
        .to_tokens(tokens)
    }
}

impl FieldArg {
    fn from_field(f: &Field) -> Self {
        Self {
            name: f.ident.as_ref().unwrap().clone(),
            ty: f.ty.clone(),
        }
    }
}

/// The last segment of a path type, so `std::string::String` and `String` both
/// answer `String`.
fn last_segment(ty: &Type) -> Option<&PathSegment> {
    match ty {
        Type::Path(p) => p.path.segments.last(),
        _ => None,
    }
}

/// The type inside `Option<..>`, if this is an option
fn option_inner(ty: &Type) -> Option<&Type> {
    let seg = last_segment(ty)?;
    if seg.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    match args.args.first() {
        Some(GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}

/// Whether the field is borrowed as a `&str` in the generated insertable struct.
///
/// Match the type exactly rather than by name: an id newtype like `StringId`
/// would otherwise be rewritten into a `&str` and fail to bind to its column.
fn is_string(f: &Field) -> bool {
    is_ty(&f.ty, "String")
}

fn is_class_name(f: &Field) -> bool {
    is_ty(&f.ty, "ClassName")
}

fn is_ty(ty: &Type, raw: &str) -> bool {
    let Some(seg) = last_segment(ty) else {
        return false;
    };
    if seg.ident == raw {
        return true;
    }
    option_inner(ty).is_some_and(|it| is_ty(it, raw))
}

fn is_option(f: &Field) -> bool {
    option_inner(&f.ty).is_some()
}

fn to_table_name(id: &Ident) -> Ident {
    let as_str = id.to_string();
    let mut new_str = String::with_capacity(as_str.len());
    for (i, c) in as_str.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                new_str.push('_');
            }
            new_str.push(c.to_ascii_lowercase());
        } else {
            new_str.push(c);
        }
    }
    new_str.push('s');
    Ident::new(&new_str, id.span())
}

fn make_ref_type(raw: &str, lifetime: Option<Lifetime>) -> Type {
    let mut segments: Punctuated<PathSegment, Colon2> = Punctuated::new();
    segments.push(PathSegment {
        ident: Ident::new(raw, Span::call_site()),
        arguments: PathArguments::None,
    });
    let elem = Box::new(Type::Path(TypePath {
        qself: None,
        path: Path {
            leading_colon: None,
            segments,
        },
    }));
    Type::Reference(TypeReference {
        lifetime,
        mutability: None,
        and_token: And::default(),
        elem,
    })
}

fn transform_type_to_ref(f: &Field, raw: &str) -> Field {
    let lifetime = Some(Lifetime::new("'data", Span::call_site()));
    let mut new_field = f.clone();
    new_field.ty = match &f.ty {
        Type::Path(tp) => {
            let seg = tp.path.segments.last().unwrap();
            match &seg.arguments {
                PathArguments::None => make_ref_type(raw, lifetime),
                PathArguments::AngleBracketed(sargs) => {
                    let mut args = sargs.clone();
                    let gen_ = args.args.first_mut().unwrap();
                    match gen_ {
                        GenericArgument::Type(ty) => {
                            *ty = make_ref_type(raw, lifetime);
                        }
                        _ => panic!("ohno"),
                    }
                    let mut segments: Punctuated<PathSegment, Colon2> = Punctuated::new();
                    segments.push(PathSegment {
                        ident: seg.ident.clone(),
                        arguments: PathArguments::AngleBracketed(args),
                    });
                    Type::Path(TypePath {
                        qself: None,
                        path: Path {
                            leading_colon: None,
                            segments,
                        },
                    })
                }
                _ => panic!("unreachable"),
            }
        }
        _ => panic!("unreachable"),
    };
    new_field
}
