use std::io::{stdin, stdout, BufReader};
use std::path::Path;

use clap::{self, Args};
use dtu::analysis::taint::TaintSinkKind;
use dtu::utils::ClassName;
use dtu::{analysis::taint::TaintReport, utils::open_file};

use crate::parsers::GraphSourceValueParser;

#[derive(Args)]
pub struct FilterMethod {
    /// Optional file to pass in, can be omitted or `-` for stdin
    #[arg(short, long)]
    file: Option<String>,

    /// Method class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Method name
    #[arg(short, long)]
    name: Option<String>,

    /// Method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Optional source for the method, note that this won't filter external calls
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,
}

impl FilterMethod {
    pub fn run(self) -> anyhow::Result<()> {
        let mut report = get_report(&self.file)?;
        let methods = std::mem::take(&mut report.methods);
        let mut report = report.filter_reaches(|kind| match kind {
            TaintSinkKind::MethodCall { method: method_id } => {
                let method = &methods[method_id];
                self.source.as_ref().is_none_or(|it| it == &method.source)
                    && self.matches(&method.class, &method.name, &method.signature)
            }
            TaintSinkKind::ExternalCall { class, name, args } => self.matches(class, name, args),
            _ => false,
        });
        report.methods = methods;
        Ok(serde_json::to_writer(stdout(), &report)?)
    }

    fn matches(&self, class: &ClassName, name: &str, signature: &str) -> bool {
        self.class.as_ref().is_none_or(|it| it == class)
            && self.name.as_deref().is_none_or(|it| it == name)
            && self.signature.as_deref().is_none_or(|it| it == signature)
    }
}

#[derive(Args)]
pub struct FilterField {
    /// Optional file to pass in, can be omitted or `-` for stdin
    #[arg(short, long)]
    file: Option<String>,

    /// Field class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Field name
    #[arg(short, long)]
    name: Option<String>,
}

impl FilterField {
    pub fn run(self) -> anyhow::Result<()> {
        let report = get_report(&self.file)?;
        let report = report.filter_reaches(|kind| match kind {
            TaintSinkKind::Field { class, name } => {
                self.class.as_ref().is_none_or(|it| it == class)
                    && self.name.as_ref().is_none_or(|it| it == name)
            }
            _ => false,
        });
        Ok(serde_json::to_writer(stdout(), &report)?)
    }
}

fn get_report(file: &Option<String>) -> anyhow::Result<TaintReport> {
    Ok(match file.as_ref().map(|it| it.as_str()) {
        None | Some("-") => serde_json::from_reader(stdin()),
        Some(v) => serde_json::from_reader(BufReader::new(open_file(Path::new(v))?)),
    }?)
}
