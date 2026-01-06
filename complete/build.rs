use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
};

use itertools::join;
use serde::de::{self, Visitor};

#[derive(Debug)]
struct Opt {
    short: String,
    long: String,
    kind: String,
    help: String,
}

#[derive(serde::Deserialize, Debug)]
struct Command {
    options: Option<Vec<Opt>>,
    #[serde(default = "String::new")]
    help: String,
    #[serde(flatten)]
    subcommands: HashMap<String, Command>,
}

impl Command {
    fn subcmds_as_simple_slice_elems(&self) -> String {
        if self.subcommands.is_empty() {
            return String::from("");
        };

        let mut s = join(
            self.subcommands
                .iter()
                .map(|(key, value)| format!("Completable::simple(\"{}\",\"{}\")", key, value.help)),
            ",\n",
        );
        s.push(',');
        s
    }

    fn opts_as_flag_slice_elems(&self) -> String {
        let Some(opts) = &self.options else {
            return String::from("");
        };

        if opts.is_empty() {
            return String::from("");
        }

        let mut s = join(
            opts.iter().map(|it| {
                format!(
                    "Completable::flag(\"{}\",\"{}\",\"{}\", CompleteKind::{})",
                    it.long, it.short, it.help, it.kind
                )
            }),
            ",\n",
        );
        s.push(',');
        s
    }
}

//macro_rules! ps {
//    ($out:ident, $s:literal) => {
//        $out.push_str($s)
//    };
//
//    ($out:ident, $s:ident) => {
//        $out.push_str($s.as_ref())
//    };
//}

macro_rules! psf {
    ($out:ident, $s:literal) => {
        $out.push_str(&format!($s))
    };

    ($out:ident, $s:literal, $($args:expr),+) => {
        $out.push_str(&format!($s, $($args,)+))
    };
}
macro_rules! wf {
    ($out:ident, $s:literal) => {
        $out.write_all(format!($s).as_bytes()).expect("failed to write to output file")
    };

    ($out:ident, $s:literal, $($args:expr),+) => {
        $out.write_all(format!($s, $($args,)+).as_bytes()).expect("failed to write to output file")
    };
}

//macro_rules! w {
//    ($out:ident, $s:literal) => {
//        $out.write_all($s.as_bytes())
//            .expect("failed to write to output file")
//    };
//
//    ($out:ident, $s:ident) => {
//        $out.write_all($s.as_bytes())
//            .expect("failed to write to output file")
//    };
//}

type OutFile = BufWriter<File>;

fn cmd_enum<'a>(parent: Option<&str>, name: &'a str) -> Cow<'a, str> {
    let name = if name.contains('-') {
        Cow::Owned(name.replace('-', "_"))
    } else {
        Cow::Borrowed(name)
    };
    match parent {
        Some(v) => Cow::Owned(format!("{v}_{name}")),
        None => name,
    }
}

fn write_gen_cmd_values(
    out: &mut OutFile,
    sub_matches: &mut String,
    commands: &HashMap<String, Command>,
    parent: Option<&str>,
) {
    for (cmd, cmddef) in commands {
        let name = cmd_enum(parent, cmd.as_str());

        let flag_elems = cmddef.opts_as_flag_slice_elems();
        let subcmd_elems = cmddef.subcmds_as_simple_slice_elems();

        if !cmddef.subcommands.is_empty() {
            let mut sub_sub_matches = String::new();
            write_gen_cmd_values(
                out,
                &mut sub_sub_matches,
                &cmddef.subcommands,
                Some(name.as_ref()),
            );

            wf!(
                out,
                r#"
fn {name}_get_completions<I: Iterator<Item = String>>(mut it: I) -> &'static [Completable] {{
    #[allow(non_upper_case_globals)]
    static __{name}_COMPLETIONS: &'static [Completable] = &[
        Completable::flag("--help", "-h", "Show this help and exit", CompleteKind::None),
        {flag_elems}
        {subcmd_elems}
    ];

    let Some(sub) = it.next() else {{
        return __{name}_COMPLETIONS;
    }};

    match sub.as_str() {{
        {sub_sub_matches}
        _ => __{name}_COMPLETIONS,
    }}
}}
"#
            );

            psf!(sub_matches, "\"{cmd}\" => {name}_get_completions(it),\n");
        } else {
            psf!(
                sub_matches,
                r#""{cmd}" => {{
        #[allow(non_upper_case_globals)]
        static __{name}_COMPLETIONS: &'static [Completable] = &[
            Completable::flag("--help", "-h", "Show this help and exit", CompleteKind::None),
            {flag_elems}
            {subcmd_elems}

        ];
        __{name}_COMPLETIONS
        }},"#
            );
        }
    }
}

fn main() {
    println!("cargo::rerun-if-changed=commands.toml");
    let commands: HashMap<String, Command> =
        toml::from_str(&fs::read_to_string("commands.toml").expect("reading commands.toml"))
            .expect("parsing commands.toml");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let name = Path::new(&out_dir).join("commands_gen.rs");
    let mut out = BufWriter::new(File::create(&name).unwrap());

    let mut sub_matches = String::new();

    let base_subcommands = join(
        commands
            .iter()
            .map(|(key, value)| format!("Completable::simple(\"{}\", \"{}\")", key, value.help)),
        ",",
    );

    write_gen_cmd_values(&mut out, &mut sub_matches, &commands, None);
    wf!(
        out,
        r#"mod generated {{
use super::*;
pub static BASE_COMPLETIONS: &'static [Completable] = &[
    {base_subcommands},
    Completable::flag("--log-stderr", "-e", "Log to stderr instead of a file", CompleteKind::None),
    Completable::flag("--log-file", "-f", "Send log to the given file", CompleteKind::File),
    Completable::flag("--log-spec", "-s", "flexi_logger log spec", CompleteKind::None),
    Completable::flag("--log-level", "-l", "Set the log level, 0 = warn, 1 = info, etc", CompleteKind::None),
    Completable::flag("--help", "-h", "Show this help and exit", CompleteKind::None),
];

#[rust_analyzer::skip]
pub fn get_completions<I: Iterator<Item = String>>(mut it: I) -> &'static [Completable] {{

    let Some(sub) = it.next() else {{
        return BASE_COMPLETIONS;
    }};
    match sub.as_str() {{
        {sub_matches}
        _ => BASE_COMPLETIONS,
    }}
}}
}}"#
    );
}

impl<'de> serde::Deserialize<'de> for Opt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;

        impl<'v> Visitor<'v> for V {
            type Value = Opt;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "array of 3 strings")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'v>,
            {
                let mut long: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &"long flag at index 0"))?;

                if !long.is_empty() {
                    long = format!("--{long}");
                };

                let mut short: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"short flag at index 1"))?;
                if !short.is_empty() {
                    short = format!("-{short}");
                };

                let kind = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"kind at index 2"))?;
                let help = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"help at index 3"))?;

                Ok(Opt {
                    long,
                    short,
                    kind,
                    help,
                })
            }
        }

        deserializer.deserialize_seq(V)
    }
}
