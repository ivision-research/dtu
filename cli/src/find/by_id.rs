use std::io::stdout;

use clap::{self, Args};
use crossterm::style::Color;
use dtu::{
    db::graph::{
        get_default_graphdb,
        models::{ClassId, MethodId},
        GraphDatabase,
    },
    prereqs::Prereq,
    smalisa::{parse_method_args, SmaliClassName, Type},
    utils::ensure_prereq,
    Context,
};

use crate::printer::Printer;

#[derive(Args)]
pub struct MethodById {
    /// Output as JSON
    #[arg(short, long)]
    json: bool,

    /// The method ID[s]
    #[arg()]
    id: Vec<i32>,
}

impl MethodById {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let mids = self
            .id
            .iter()
            .map(|it| MethodId::from(*it))
            .collect::<Vec<_>>();
        let methods = db.get_methods_by_id(&mids)?;
        if self.json {
            serde_json::to_writer(stdout(), &methods)?;
            return Ok(());
        }
        let printer = Printer::new();
        for m in methods {
            printer.print_colored(m.access_flags.to_string(), Color::DarkYellow);
            printer.print(" ");
            let class = m.class.get_smali_name();
            print_class(&printer, &SmaliClassName::from_raw(&class));
            printer.print(&format!("->{}(", m.name));
            if m.signature.len() > 0 {
                let args = parse_method_args(&m.signature)
                    .map_err(|e| anyhow::Error::msg(format!("invalid args: {e}")))?;
                for arg in args.into_iter() {
                    match arg {
                        Type::Primitive(p, d) => {
                            printer.print(format!("{}{}", "[".repeat(d as usize), p))
                        }
                        Type::Class(c, d) => {
                            if d > 0 {
                                printer.print("[".repeat(d as usize));
                            }
                            print_class(&printer, c);
                        }
                        Type::Unknown => {}
                    }
                }
            }
            printer.print(")");
            let ret = m.ret;
            if ret.starts_with('L') {
                let sret = SmaliClassName::from_raw(&ret);
                print_class(&printer, &sret);
            } else {
                printer.print(&ret);
            }
            printer.print('\n');
        }
        Ok(())
    }
}

fn print_class(printer: &Printer, class: &SmaliClassName) {
    printer.print("L");
    printer.print_colored(class.without_markers(), Color::Magenta);
    printer.print(";");
}

#[derive(Args)]
pub struct ClassById {
    /// Output as JSON
    #[arg(short, long)]
    json: bool,

    /// The class ID[s]
    #[arg()]
    id: Vec<i32>,
}

impl ClassById {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let cids = self
            .id
            .iter()
            .map(|it| ClassId::from(*it))
            .collect::<Vec<_>>();
        let classes = db.get_classes_by_id(&cids)?;
        if self.json {
            serde_json::to_writer(stdout(), &classes)?;
            return Ok(());
        }

        let printer = Printer::new();

        for class in classes {
            printer.print_colored(class.access_flags.to_string(), Color::DarkYellow);
            printer.print(" ");
            let name = class.name.get_smali_name();
            print_class(&printer, &SmaliClassName::from_raw(&name));
            printer.print('\n');
        }

        Ok(())
    }
}
