use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{Field, Type, Visibility};

pub(crate) struct SetterMethod {
    name: Ident,
    ty: Type,
    vis: Visibility,
}

impl SetterMethod {
    pub(crate) fn from_field(f: &Field) -> Self {
        let name = f.ident.as_ref().unwrap().clone();
        let ty = f.ty.clone();
        let vis = f.vis.clone();
        Self { name, ty, vis }
    }
}

impl ToTokens for SetterMethod {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ty = &self.ty;
        let field_name = &self.name;
        let vis = &self.vis;
        let func_name = Ident::new(&format!("set_{}", field_name), Span::call_site());
        (quote! {
            #vis fn #func_name(mut self, value: #ty) -> Self {
                self.#field_name = value;
                self
            }
        })
        .to_tokens(tokens);
    }
}

//pub(crate) fn quote_string(s: &str) -> String {
//    let mut into = String::from("\"");
//    for c in s.escape_default() {
//        into.push(c);
//    }
//    into.push('"');
//    into
//}
//
//pub(crate) fn quote_string_into(s: &str, into: &mut String) {
//    into.truncate(0);
//    into.push('"');
//    for c in s.escape_default() {
//        into.push(c);
//    }
//    into.push('"');
//}
