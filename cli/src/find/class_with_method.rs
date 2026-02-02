use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    db::graph::{GraphDatabase, GraphSqliteDatabase},
    smalisa::AccessFlag,
    utils::{hex, ClassName},
    Context,
};
use sha2::{Digest, Sha256};

use crate::{
    printer::{color, Printer},
    utils::{oshash, ostr, project_cacheable, shash},
};

#[derive(Args)]
pub struct FindClassWithMethod {
    /// Method name
    #[arg(short, long)]
    name: String,

    /// Method signature
    #[arg(short, long)]
    sig: Option<String>,

    /// Source for the class
    #[arg(short = 'S', long)]
    source: Option<String>,

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,

    /// Filter out abstract classes
    #[arg(short = 'A', long, default_value_t = false)]
    no_abstract: bool,
}

impl FindClassWithMethod {
    pub fn run(self, ctx: &dyn Context, gdb: &GraphSqliteDatabase) -> anyhow::Result<()> {
        let mut hasher = Sha256::new();
        shash(&mut hasher, &self.name);
        oshash(&mut hasher, &self.sig);
        oshash(&mut hasher, &self.source);
        let digest = hasher.finalize();

        let cache = format!("find-class-with-method-{}", hex::bytes_to_hex(&digest));
        let mut classes = project_cacheable(&ctx, &cache, self.no_cache, || {
            Ok(gdb.find_classes_with_method(&self.name, ostr(&self.sig), ostr(&self.source))?)
        })?;

        if self.no_abstract {
            classes = classes
                .into_iter()
                .filter(|it| {
                    !(it.access_flags.contains(AccessFlag::ABSTRACT)
                        || it.access_flags.contains(AccessFlag::INTERFACE))
                })
                .collect()
        }

        let printer = Printer::new();

        if self.source.is_some() {
            for class in classes {
                printer.println(class.name);
            }
            return Ok(());
        }

        let mut map: HashMap<String, Vec<ClassName>> = HashMap::new();

        for class in classes {
            match map.get_mut(&class.source) {
                Some(v) => v.push(class.name),
                None => {
                    _ = map.insert(class.source, vec![class.name]);
                }
            }
        }

        for (source, classes) in map {
            printer.println_colored(source, color::YELLOW);
            for class in classes {
                printer.print("\t");
                printer.println(class);
            }
        }

        Ok(())
    }
}
