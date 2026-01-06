use std::{
    borrow::Cow,
    collections::HashMap,
    ops::{Deref, DerefMut},
};

#[derive(Clone, Debug)]
pub enum CompleteKind {
    None,

    Uncompletable,

    // When the completions are entirely known at compile time we just return one of these
    List(Cow<'static, [Completable]>),

    SystemService,
    SystemServiceMethod,
    Receiver,
    Activity,
    #[allow(dead_code)]
    Provider,
    Service,
    GraphSource,
    DiffSource,
    Apk,
    TestName,
    ProviderAuthority,

    GraphMethod,
    GraphSignature,
    GraphClass,

    File,
    Dir,
}

pub struct FlagMap(HashMap<String, String>);
impl Deref for FlagMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FlagMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FlagMap {
    fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn get_flag(&self, long: &'_ str, short: &'_ str) -> Option<&String> {
        if let Some(v) = self.get(long) {
            return Some(v);
        }

        if let Some(v) = self.get(short) {
            return Some(v);
        }

        None
    }

    pub fn get_first_flag(&self, items: &'_ [(&'_ str, &'_ str)]) -> Option<&String> {
        for (long, short) in items.iter() {
            if let Some(v) = self.get_flag(long, short) {
                return Some(v);
            }
        }
        None
    }
}

impl CompleteKind {
    /// Given the argument list, excluding the value to be completed, find the [CompleteKind]
    /// that needs to be completed.
    ///
    /// We do this by walking the arguments forward and trying to find:
    ///
    ///     1. The subcommand (if any) that we're in
    ///     2. The flag (if any) before what is being completed
    ///
    /// This uses the contents of the commands.toml file and the code generated
    /// from that file in build.rs
    pub fn find(args: Vec<String>) -> (CompleteKind, FlagMap) {
        let mut flag_map: FlagMap = FlagMap::new();
        // The thing right before what we're completing. If there is nothing there, we're completing
        // base command flags and subcommands
        let Some(last) = args.last().map(String::clone) else {
            return (
                CompleteKind::List(Cow::Borrowed(generated::BASE_COMPLETIONS)),
                flag_map,
            );
        };

        let mut subcommands: Vec<String> = Vec::new();
        let mut it = args.into_iter();
        let mut cur = None;
        loop {
            let arg = match cur.take() {
                None => match it.next() {
                    None => break,
                    Some(v) => v,
                },
                Some(v) => v,
            };

            if arg == "--" {
                return (CompleteKind::Uncompletable, flag_map);
            }

            if arg.starts_with('-') {
                let Some(v) = it.next() else {
                    break;
                };
                if !v.starts_with('-') {
                    flag_map.insert(arg.trim_start_matches('-').into(), v);
                } else {
                    cur = Some(v);
                    continue;
                }
            } else {
                subcommands.push(arg);
            }
        }

        let completing_flag_arg = last.starts_with('-');

        let base = generated::get_completions(subcommands.into_iter());
        if completing_flag_arg {
            for c in base.iter() {
                if let Completable::Flag(f) = c {
                    if f.matches(&last) {
                        // If we found the flag but it doesn't take an argument, whatever we're
                        // completing right now is base
                        if matches!(f.kind, CompleteKind::None) {
                            return (CompleteKind::List(Cow::Borrowed(base)), flag_map);
                        }

                        return (f.kind.clone(), flag_map);
                    }
                }
            }
            // Unknown flag, sorry
            return (CompleteKind::Uncompletable, flag_map);
        }

        (CompleteKind::List(Cow::Borrowed(base)), flag_map)
    }
}

#[derive(Clone, Debug)]
pub struct Flag {
    pub long: &'static str,
    pub short: &'static str,
    pub help: &'static str,
    pub kind: CompleteKind,
}

#[derive(Clone, Debug)]
pub struct Simple {
    pub name: &'static str,
    pub help: &'static str,
}

#[derive(Clone, Debug)]
pub enum Completable {
    Flag(Flag),
    Simple(Simple),
}

impl Completable {
    const fn flag(
        long: &'static str,
        short: &'static str,
        help: &'static str,
        kind: CompleteKind,
    ) -> Self {
        Self::Flag(Flag::new(long, short, help, kind))
    }

    const fn simple(name: &'static str, help: &'static str) -> Self {
        Self::Simple(Simple::new(name, help))
    }
}

impl Simple {
    const fn new(name: &'static str, help: &'static str) -> Self {
        Self { name, help }
    }
}

impl Flag {
    const fn new(
        long: &'static str,
        short: &'static str,
        help: &'static str,
        kind: CompleteKind,
    ) -> Self {
        Self {
            long,
            short,
            help,
            kind,
        }
    }

    fn matches(&self, s: &str) -> bool {
        (!self.long.is_empty() && s == self.long) || (!self.short.is_empty() && s == self.short)
    }
}

include!(concat!(env!("OUT_DIR"), "/commands_gen.rs"));
