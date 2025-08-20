use clap::{self, Args};

use dtu::DefaultContext;
use dtu::tasks::fuzz;

#[derive(Args)]
pub struct Unprotected;

impl Unprotected {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();

        let res_vec = fuzz::get_no_security(&ctx)?;

        println!("Service\tMethod");
        println!("----------------------");

        for res in res_vec {
            println!("{}\t{}", res.service_name, res.method_name); 
        }

        Ok(())
    }
}
