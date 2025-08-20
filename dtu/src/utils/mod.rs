pub mod fs;
pub use fs::*;

pub mod class_name;
pub use class_name::*;

pub mod readers;
pub use readers::*;

pub mod device_path;
pub use device_path::*;

#[cfg(feature = "setup")]
pub mod smali;
#[cfg(feature = "setup")]
pub use smali::*;

pub mod hex;
pub use hex::*;

pub mod allowlist;

pub use allowlist::*;
use base64::Engine;

#[cfg(feature = "sql")]
pub fn ensure_prereq(
    ctx: &dyn crate::Context,
    prereq: crate::prereqs::Prereq,
) -> crate::Result<()> {
    use crate::db::sql::{MetaDatabase, MetaSqliteDatabase};

    let mdb = MetaSqliteDatabase::new(ctx).map_err(|e| crate::Error::Generic(e.to_string()))?;
    mdb.ensure_prereq(prereq)
}

pub fn base64_bytes(data: &[u8]) -> String {
    let eng = base64::engine::general_purpose::STANDARD;
    eng.encode(data)
}

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let eng = base64::engine::general_purpose::STANDARD;
    eng.decode(s).ok()
}

#[cfg(not(windows))]
pub(crate) const PATHSAFE_ESCAPE_CHAR: char = '\\';

// lol I dunno
#[cfg(windows)]
pub(crate) const PATHSAFE_ESCAPE_CHAR: char = '/';

/// Replace [target] with [replacement] in the input string, escaping all instances of
/// [replacement] with [PATHSAFE_ESCAPE_CHAR]
pub(crate) fn replace_char(input: &str, target: char, replacement: char) -> String {
    let mut replaced = String::with_capacity(input.len());

    for c in input.chars() {
        if c == replacement {
            replaced.push(PATHSAFE_ESCAPE_CHAR);
            replaced.push(replacement);
        } else if c == PATHSAFE_ESCAPE_CHAR {
            // Escape the escape
            replaced.push(PATHSAFE_ESCAPE_CHAR);
            replaced.push(c);
        } else if c == target {
            // Change the separators
            replaced.push(replacement);
        } else {
            // Nothing special
            replaced.push(c);
        }
    }

    replaced
}

/// Inverse of [replace_char], note that it should be called with the same target and replacement
pub(crate) fn unreplace_char(input: &str, target: char, replacement: char) -> String {
    let mut replaced = String::with_capacity(input.len());

    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        // We only escape `replacement`, so if we see the escape check if it's followed by that. If
        // it isn't, just toss the escape back into the name and keep going.
        if c == PATHSAFE_ESCAPE_CHAR {
            match chars.next() {
                Some(nc) => {
                    // If it's an escaped `replacement`, put a literal `replacement` into the path
                    if nc == replacement {
                        replaced.push(replacement);
                    } else if nc == PATHSAFE_ESCAPE_CHAR {
                        // Unescape the escape
                        replaced.push(target);
                    } else {
                        log::warn!("Bug! `{}` followed by unexpected char in {}", target, input);
                        // This should be unreachable
                        replaced.push(c);
                        replaced.push(nc);
                    }
                }
                // No chars left, push the sep and break
                None => {
                    replaced.push(c);
                    break;
                }
            }
        } else if c == replacement {
            // `replacement` but not escaped, change it to `target`
            replaced.push(target);
        } else {
            // Nothing special
            replaced.push(c);
        }
    }

    replaced
}
