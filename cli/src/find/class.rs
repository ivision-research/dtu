use clap::{self, Args};
use dtu::{
    db::graph::{
        get_default_graphdb,
        schema::{classes, sources},
        DefaultGraphDatabase,
    },
    diesel::prelude::*,
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName},
    Context,
};

#[derive(Args)]
pub struct FindClass {
    /// Class name to search for, may be partial if given in smali form
    #[arg(short, long)]
    class: String,
}

impl FindClass {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let gdb = get_default_graphdb(ctx)?;

        let (is_like, search_param) = if self.class.starts_with("L") {
            if self.class.ends_with(";") {
                (false, self.class.clone())
            } else {
                (true, format!("{}%", self.class))
            }
        } else {
            (
                false,
                ClassName::from(&self.class).get_smali_name().into_owned(),
            )
        };

        if is_like {
            self.do_like(gdb, search_param)
        } else {
            self.do_exact(gdb, search_param)
        }
    }
    fn do_exact(self, gdb: DefaultGraphDatabase, search: String) -> anyhow::Result<()> {
        let result = gdb.with_connection(|c| {
            classes::table
                .inner_join(sources::table)
                .select(sources::name)
                .filter(classes::name.eq(&search))
                .get_results::<String>(c)
        })?;
        for source in result {
            println!("{}", source);
        }
        Ok(())
    }

    fn do_like(self, gdb: DefaultGraphDatabase, search: String) -> anyhow::Result<()> {
        let result = gdb.with_connection(|c| {
            classes::table
                .inner_join(sources::table)
                .select((classes::name, sources::name))
                .filter(classes::name.like(&search))
                .get_results::<(String, String)>(c)
        })?;
        for (class, source) in result {
            println!("{} in {}", class, source);
        }
        Ok(())
    }
}
