use clap::{self, Args};

use crate::find::utils::get_method_search;
use crate::printer::{color, Printer};
use dtu::db::graph::GraphDatabase;
use dtu::utils::ClassName;

#[derive(Args)]
pub struct FindCallers {
    /// The source of the method to find callers to
    #[arg(short, long)]
    method_source: Option<String>,

    /// The source of the calls
    #[arg(short, long)]
    call_source: Option<String>,

    /// Method name
    #[arg(short, long)]
    name: String,

    /// Method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Method class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Depth to search
    #[arg(short, long, default_value_t = 3)]
    depth: usize,
}

impl FindCallers {
    pub fn run(&self, db: &dyn GraphDatabase) -> anyhow::Result<()> {
        let class_ref = self.class.as_ref();
        let search = get_method_search(
            Some(self.name.as_str()),
            class_ref,
            self.signature.as_ref().map(String::as_str),
            self.method_source.as_ref().map(String::as_str),
        )?;
        let mpaths = db.find_callers(
            &search,
            self.call_source.as_ref().map(String::as_str),
            self.depth,
        )?;

        let printer = Printer::new();

        for p in mpaths {
            let mut iter = p.path.iter().take(p.path.len() - 1);
            let first = match iter.next() {
                Some(v) => v,
                None => continue,
            };

            printer.println_colored(first.as_smali(), color::YELLOW);

            for c in iter {
                printer.print("   ");
                printer.println_colored(c.as_smali(), color::GREY);
            }
        }
        Ok(())
    }
}
