use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
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

    define_insertable(&st, &stripped_fields, &diesel_attrs, &mut tokens);

    let code = quote! {
        #[cfg_attr(debug_assertions, derive(Debug, PartialEq))]
        #[derive(::std::clone::Clone, ::diesel::Queryable, ::diesel::Identifiable, ::diesel::AsChangeset)]
        #( #attrs )*
        #vis struct #ident #generics {
            #(
                #stripped_fields,
            )*
        }
    };

    code.to_tokens(&mut tokens);

    tokens.into()
}

fn define_insertable(
    st: &ItemStruct,
    stripped_fields: &Vec<Field>,
    diesel_attrs: &Vec<Attribute>,
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

    let mut has_string = false;

    let transformed_fields = stripped_fields
        .iter()
        .filter(|f| (*f).ident.as_ref().unwrap().to_string() != "id")
        .map(|f| {
            if !is_string(f) {
                return f.clone();
            }
            has_string = true;
            transform_string_to_str(f)
        })
        .collect::<Vec<Field>>();

    let lifetime = if has_string {
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
        /// Auto generated type for inserting a #name into the database
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

fn is_string(f: &Field) -> bool {
    let ty = &f.ty;
    let s = match ty {
        Type::Path(p) => {
            let s = &p.path;
            (quote! {#s}).to_string()
        }
        _ => return false,
    };
    s.contains("String")
}

fn is_option(f: &Field) -> bool {
    let ty = &f.ty;
    let s = match ty {
        Type::Path(p) => {
            let s = &p.path;
            (quote! {#s}).to_string()
        }
        _ => return false,
    };
    s.ends_with('>') && s.starts_with("Option")
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

fn make_str_ref_type(lifetime: Option<Lifetime>) -> Type {
    let mut segments: Punctuated<PathSegment, Colon2> = Punctuated::new();
    segments.push(PathSegment {
        ident: Ident::new("str", Span::call_site()),
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

fn transform_string_to_str(f: &Field) -> Field {
    let lifetime = Some(Lifetime::new("'data", Span::call_site()));
    let mut new_field = f.clone();
    new_field.ty = match &f.ty {
        Type::Path(tp) => {
            let seg = tp.path.segments.first().unwrap();
            match &seg.arguments {
                PathArguments::None => make_str_ref_type(lifetime),
                PathArguments::AngleBracketed(sargs) => {
                    let mut args = sargs.clone();
                    let gen = args.args.first_mut().unwrap();
                    match gen {
                        GenericArgument::Type(ty) => {
                            *ty = make_str_ref_type(lifetime);
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
