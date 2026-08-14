use std::collections::HashSet;

use smalisa::cfg::InstructionId;
use smalisa::instructions::{InvArgs, INS_CHECK_CAST, INS_NEW_INSTANCE};
use smalisa::ssa::{SsaMethod, Value, ValueId};
use smalisa::{Literal, Primitive, RawLiteral, Type};

use crate::analysis::taint::TaintSource;
use crate::utils::smali::SmaliMethodSignatureIterator;
use crate::utils::ClassName;

/// Every concrete class a method's `return-object` can hand back.
///
/// Resolved from the def site of each returned value: a `new-instance` or
/// `check-cast` names the type outright, a field read contributes the field's
/// declared type, and a call contributes the callee's declared return type. A
/// phi fans out to all of its operands.
///
/// The walk stays inside this method. Nothing follows a call into the callee, so
/// a value produced by a factory resolves only as far as that factory's declared
/// return type, which for an `IBinder` factory is not useful. Callers should
/// treat an empty result as "unknown" rather than "returns nothing".
pub fn resolve_returned_classes(method: &SsaMethod<'_>) -> Vec<ClassName> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for block in method.reverse_post_order() {
        for ins in method.block_instructions(block) {
            if !method.instruction(ins).instruction().is_return() {
                continue;
            }
            // return-void reads nothing
            for &value in method.uses(ins) {
                resolve_value(method, value, &mut seen, &mut out);
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn resolve_value(
    method: &SsaMethod<'_>,
    value: ValueId,
    seen: &mut HashSet<ValueId>,
    out: &mut Vec<ClassName>,
) {
    // Phis can be mutually recursive, so a value is only ever expanded once
    if !seen.insert(value) {
        return;
    }

    match method.value(value) {
        Value::Phi(phi) => {
            for operand in &method.phi(*phi).operands {
                resolve_value(method, operand.value, seen, out);
            }
        }
        Value::Const(const_id) => match method.constant(*const_id) {
            Literal::Type(Type::Class(clazz, _)) => {
                out.push(ClassName::from(*clazz));
            }
            _ => {}
        },
        Value::Instruction(ins) => resolve_instruction(method, *ins, seen, out),
        // A parameter's declared type is in the signature, which this does not parse
        Value::Param(_) | Value::Undef => {}
    }
}

fn resolve_instruction(
    method: &SsaMethod<'_>,
    ins: InstructionId,
    seen: &mut HashSet<ValueId>,
    out: &mut Vec<ClassName>,
) {
    let inv = method.instruction(ins);
    let opcode = inv.instruction();

    // A move or a move-result just renames a value, so keep walking back
    if opcode.is_move() || opcode.reads_result() {
        for &used in method.uses(ins) {
            resolve_value(method, used, seen, out);
        }
        return;
    }

    let ty = match inv.args() {
        InvArgs::OneRegLiteral(_, RawLiteral::Type(ty))
            if opcode == INS_NEW_INSTANCE || opcode == INS_CHECK_CAST =>
        {
            Some(*ty)
        }
        // sget-object and iget-object contribute the field's declared type
        InvArgs::OneRegField(_, field) => Some(field.ty),
        InvArgs::TwoRegField(_, _, field) => Some(field.ty),
        InvArgs::VarRegMethod(_, target) if opcode.is_call() => Some(target.return_type),
        InvArgs::Polymorphic(_, target, _, _) if opcode.is_call() => Some(target.return_type),
        _ => None,
    };

    // Arrays and primitives are never a class we can look up
    if let Some(Type::Class(name, 0)) = ty {
        out.push(ClassName::from(name));
    }
}

/// A parameter of a non static method, as the register the body sees it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamRegister {
    /// Position in the signature, ignoring the receiver
    pub index: usize,
    /// The `p` register number, so `p0` is the receiver
    pub register: u16,
}

/// The registers holding the interesting parameters of an instance method.
///
/// Class types always count. Primitives never do: a scalar an attacker picks is
/// rarely interesting on its own and every one would seed a walk. `[B` is the one
/// primitive array kept, because a byte array is a serialized blob rather than a
/// bag of numbers; `[I` and friends are skipped with the rest.
///
/// `p0` is the receiver, and a `long` or `double` occupies two registers, so the
/// register a parameter lands in is not its position in the signature.
pub fn complex_params(signature: &str) -> Result<Vec<ParamRegister>, &str> {
    let mut out = Vec::new();
    // p0 is `this`
    let mut register: u16 = 1;

    for (index, ty) in SmaliMethodSignatureIterator::new(signature)?.enumerate() {
        let complex = match ty {
            Type::Class(..) => true,
            Type::Primitive(Primitive::Byte, depth) => depth > 0,
            Type::Primitive(..) | Type::Unknown => false,
        };
        if complex {
            out.push(ParamRegister { index, register });
        }
        register += register_width(&ty);
    }

    Ok(out)
}

/// Reference typed parameters as taint sources
pub fn complex_param_sources(signature: &str) -> Result<Vec<TaintSource>, &str> {
    Ok(complex_params(signature)?
        .into_iter()
        .map(|it| TaintSource::Param {
            register: it.register,
        })
        .collect())
}

/// Longs and doubles take a register pair, everything else takes one
fn register_width(ty: &Type<'_>) -> u16 {
    match ty {
        Type::Primitive(prim, 0) => match prim {
            smalisa::Primitive::Long | smalisa::Primitive::Double => 2,
            _ => 1,
        },
        _ => 1,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_complex_params_skips_primitives_and_pairs_wides() {
        // p0=this p1=String p2=int p3:p4=long p5=byte[] p6=Bundle
        let got = complex_params("Ljava/lang/String;IJ[BLandroid/os/Bundle;").expect("parse");
        assert_eq!(
            got,
            vec![
                ParamRegister {
                    index: 0,
                    register: 1
                },
                ParamRegister {
                    index: 3,
                    register: 5
                },
                ParamRegister {
                    index: 4,
                    register: 6
                },
            ]
        );
    }

    #[test]
    fn test_complex_params_all_primitive_is_empty() {
        assert!(complex_params("IJZ").expect("parse").is_empty());
        assert!(complex_params("").expect("parse").is_empty());
    }

    #[test]
    fn test_complex_params_keeps_byte_arrays_only() {
        // p0=this p1=int[] p2=long[] p3=byte[] p4=String[]
        let got = complex_params("[I[J[B[Ljava/lang/String;").expect("parse");
        assert_eq!(
            got,
            vec![
                ParamRegister {
                    index: 2,
                    register: 3
                },
                ParamRegister {
                    index: 3,
                    register: 4
                },
            ]
        );
    }

    #[test]
    fn test_complex_params_double_pairs() {
        // p0=this p1:p2=double p3=String
        let got = complex_params("DLjava/lang/String;").expect("parse");
        assert_eq!(
            got,
            vec![ParamRegister {
                index: 1,
                register: 3
            }]
        );
    }
}
