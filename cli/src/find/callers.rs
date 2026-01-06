use clap::{self, Args};

use crate::find::utils::get_method_search;
use crate::printer::{color, Printer};
use crate::utils::ostr;
use dtu::db::graph::GraphDatabase;
use dtu::utils::ClassName;

#[derive(Args)]
pub struct FindCallers {
    /// The source of the method to find callers to
    #[arg(short = 'M', long)]
    method_source: Option<String>,

    /// The source of the calls
    #[arg(short = 'C', long)]
    call_source: Option<String>,

    /// Method name
    #[arg(short, long)]
    name: Option<String>,

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
            ostr(&self.name),
            class_ref,
            ostr(&self.signature),
            ostr(&self.method_source),
        )?;
        let mpaths = db.find_callers(&search, ostr(&self.call_source), self.depth)?;

        let printer = Printer::new();

        // If the name isn't provided we have to show it :)
        let take_offset = if self.name.is_some() {
            1
        } else {
            0
        };

        for p in mpaths {

            let mut iter = p.path.iter().take(p.path.len() - take_offset);
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
