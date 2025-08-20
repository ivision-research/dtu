mod sql;
use sql::sql_db_row as sql_db_row_impl;

mod utils;

use crate::utils::SetterMethod;
use proc_macro;
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{self, parse_macro_input, ItemEnum, ItemStruct};

#[proc_macro_attribute]
pub fn wraps_base_error(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let en = parse_macro_input!(item as ItemEnum);
    let mut tokens = TokenStream::new();

    let attrs = &en.attrs;
    let vis = &en.vis;
    let ident = &en.ident;
    let generics = &en.generics;
    let variants = &en.variants;

    let appended = quote! {
        #vis enum #ident #generics  {
            #variants

            #[error("{0}")]
            Base(crate::errors::Error),
        }

        impl #generics ::std::convert::From<::std::io::Error> for #ident #generics {
            fn from(value: ::std::io::Error) -> Self {
                Self::Base(crate::errors::Error::from(value))
            }
        }

        impl #generics ::std::convert::From<crate::errors::Error> for #ident #generics {
            fn from(value: crate::errors::Error) -> Self {
                Self::Base(value)
            }
        }

        impl #generics ::std::convert::From<::std::boxed::Box<dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync>> for #ident #generics  {
            fn from(value: ::std::boxed::Box<dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync>) -> Self {
                Self::Base(crate::errors::Error::from(value))
            }
        }
    };

    for att in attrs {
        att.to_tokens(&mut tokens);
    }

    appended.to_tokens(&mut tokens);
    tokens.into()
}

/// Define `set_{field_name}` functions for the struct
#[proc_macro_attribute]
pub fn define_setters(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let st = parse_macro_input!(item as ItemStruct);
    let mut tokens = TokenStream::new();
    let setters = st
        .fields
        .iter()
        .map(SetterMethod::from_field)
        .collect::<Vec<SetterMethod>>();
    let name = &st.ident;
    let generics = &st.generics;

    (quote! {
        #st

        impl #generics #name #generics {
            #(
                #setters
            )*
        }
    })
    .to_tokens(&mut tokens);

    tokens.into()
}

// TODO DRY

#[proc_macro_attribute]
pub fn wraps_decompile_error(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let en = parse_macro_input!(item as ItemEnum);
    let mut tokens = TokenStream::new();

    let attrs = &en.attrs;
    let vis = &en.vis;
    let ident = &en.ident;
    let generics = &en.generics;
    let variants = &en.variants;

    let appended = quote! {
        #vis enum #ident #generics  {
            #variants

            #[error("{0}")]
            Decompile(crate::decompile::DecompileError),
        }

        impl #generics ::std::convert::From<crate::decompile::DecompileError> for #ident #generics {
            fn from(value: crate::decompile::DecompileError) -> Self {
                Self::Decompile(value)
            }
        }


    };

    for att in attrs {
        att.to_tokens(&mut tokens);
    }

    appended.to_tokens(&mut tokens);
    tokens.into()
}

/// Marks a struct as a SQL database row
///
/// This will put `#[derive(Queryable, AsChangeset, Identifiable)]` on the
/// struct and also generate a `#[derive(Insertable)]` wrapped struct named
/// Insert${name}.
#[proc_macro_attribute]
pub fn sql_db_row(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    sql_db_row_impl(attr, item)
}
