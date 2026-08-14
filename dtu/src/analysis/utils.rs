use smalisa::ssa::{SsaClass, SsaMethod};

use crate::db::graph::MethodSpec;

pub fn get_ssa_method<'a, 'b>(
    class: &'b SsaClass<'a>,
    m: &MethodSpec,
) -> Option<&'b SsaMethod<'a>> {
    for ssa_method in &class.methods {
        let method = &ssa_method.method.method;
        if method.name == m.name && method.args == m.signature {
            return Some(ssa_method);
        }
    }

    None
}
