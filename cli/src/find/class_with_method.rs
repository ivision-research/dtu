use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    db::graph::{GraphDatabase, GraphSqliteDatabase},
    smalisa::AccessFlag,
    utils::ClassName,
    Context,
};

use crate::{
    cache_key,
    printer::{color, Printer},
    utils::{bool_hash_key, ostr, project_cacheable_json},
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
    #[arg(long, default_value_t = false)]
    no_cache: bool,

    /// Output as JSON
    #[arg(short, long)]
    json: bool,

    /// Filter out abstract classes
    #[arg(short = 'A', long, default_value_t = false)]
    no_abstract: bool,
}

impl FindClassWithMethod {
    pub fn run(self, ctx: &dyn Context, gdb: &GraphSqliteDatabase) -> anyhow::Result<()> {
        let cache = cache_key!(
            "find-class-with-method",
            ostrs: [
                &self.sig,
                &self.source
            ],
            bool_hash_key(self.no_abstract),
            &self.name
        );

        let class_map =
            project_cacheable_json(ctx, &cache, self.no_cache, self.json, || self.go(gdb))?;

        let printer = Printer::new();

        if self.source.is_some() {
            for classes in class_map.values() {
                for class in classes {
                    printer.println(class);
                }
            }
            return Ok(());
        }

        for (source, classes) in class_map {
            printer.println_colored(source, color::YELLOW);
            for class in classes {
                printer.print("\t");
                printer.println(class);
            }
        }
        Ok(())
    }

    fn go(&self, gdb: &GraphSqliteDatabase) -> anyhow::Result<HashMap<String, Vec<ClassName>>> {
        let mut classes =
            gdb.find_classes_with_method(&self.name, ostr(&self.sig), ostr(&self.source))?;

        if self.no_abstract {
            classes = classes
                .into_iter()
                .filter(|it| {
                    !(it.access_flags.contains(AccessFlag::ABSTRACT)
                        || it.access_flags.contains(AccessFlag::INTERFACE))
                })
                .collect()
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
        Ok(map)
    }
}
