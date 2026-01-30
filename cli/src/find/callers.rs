use clap::{self, Args};
use dtu::Context;
use sha2::{Digest, Sha256};

use crate::find::utils::get_method_search;
use crate::printer::{color, Printer};
use crate::utils::{oshash, ostr, project_cacheable};
use dtu::db::graph::{GraphDatabase, MethodCallPath};
use dtu::utils::{hex, ClassName};

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

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,
}

impl FindCallers {
    pub fn run(&self, ctx: &dyn Context, db: &dyn GraphDatabase) -> anyhow::Result<()> {
        let mut hasher = Sha256::new();
        oshash(&mut hasher, &self.method_source);
        oshash(&mut hasher, &self.method_source);
        oshash(&mut hasher, &self.name);
        oshash(&mut hasher, &self.signature);
        oshash(&mut hasher, &self.class);
        let digest = hasher.finalize();
        let cache = format!("find-callers-{}-{}", hex::bytes_to_hex(&digest), self.depth);
        let mpaths = project_cacheable(&ctx, &cache, self.no_cache, || self.go(db))?;
        // If the name isn't provided we have to show it
        let take_offset = if self.name.is_some() { 1 } else { 0 };
        // If they didn't provide a source, we have to show it
        let show_source = self.call_source.is_none();

        let printer = Printer::new();
        for p in mpaths {
            let mut iter = p.path.iter().take(p.path.len() - take_offset);
            let first = match iter.next() {
                Some(v) => v,
                None => continue,
            };

            if show_source {
                printer.println_colored(
                    format!("{} in {}", first.as_smali(), first.source),
                    color::YELLOW,
                );
            } else {
                printer.println_colored(first.as_smali(), color::YELLOW);
            }

            for c in iter {
                printer.print("   ");
                printer.println_colored(c.as_smali(), color::GREY);
            }
        }
        Ok(())
    }

    fn go(&self, db: &dyn GraphDatabase) -> anyhow::Result<Vec<MethodCallPath>> {
        let class_ref = self.class.as_ref();
        let search = get_method_search(
            ostr(&self.name),
            class_ref,
            ostr(&self.signature),
            ostr(&self.method_source),
        )?;
        Ok(db.find_callers(&search, ostr(&self.call_source), self.depth)?)
    }
}

#[derive(Args)]
pub struct FindOutgoingCalls {
    /// Only show outgoing calls that end up in this source
    #[arg(short = 'T', long)]
    into_source: Option<String>,

    #[arg(short = 'L', long)]
    /// Specify the source for the provided class or method
    leaving_source: Option<String>,

    /// Caller method name
    #[arg(short, long)]
    name: Option<String>,

    /// Caller method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Caller class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Depth to search
    #[arg(short, long, default_value_t = 3)]
    depth: usize,

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,
}

impl FindOutgoingCalls {
    pub fn run(&self, ctx: &dyn Context, db: &dyn GraphDatabase) -> anyhow::Result<()> {
        let mut hasher = Sha256::new();
        // We deliberately leave the `into_source` out of this cache string because the
        // filtering happens here and not in the graph database call. Until one day it
        // does and someone shares this comment with me explaining the soure of a bug.
        oshash(&mut hasher, &self.leaving_source);
        oshash(&mut hasher, &self.name);
        oshash(&mut hasher, &self.signature);
        oshash(&mut hasher, &self.class);
        let digest = hasher.finalize();
        let cache = format!(
            "find-outgoing-call-{}-{}",
            hex::bytes_to_hex(&digest),
            self.depth
        );
        let mpaths = project_cacheable(&ctx, &cache, self.no_cache, || self.go(db))?;

        // If the name isn't provided we have to show it :)
        let take_offset = if self.name.is_some() { 1 } else { 0 };

        if let Some(s) = &self.into_source {
            self.show_into(s, take_offset, mpaths)
        } else {
            self.show(take_offset, mpaths)
        }
    }

    fn go(&self, db: &dyn GraphDatabase) -> anyhow::Result<Vec<MethodCallPath>> {
        let class_ref = self.class.as_ref();
        let search = get_method_search(
            ostr(&self.name),
            class_ref,
            ostr(&self.signature),
            ostr(&self.leaving_source),
        )?;
        Ok(db.find_outgoing_calls(&search, self.depth)?)
    }

    fn show_into(
        &self,
        into: &str,
        take_offset: usize,
        mpaths: Vec<MethodCallPath>,
    ) -> anyhow::Result<()> {
        let iter = mpaths.into_iter().filter(|it| {
            let dst = it.must_get_dst_method();
            dst.source == into
        });

        self.show(take_offset, iter)
    }

    fn show<I: IntoIterator<Item = MethodCallPath>>(
        &self,
        take_offset: usize,
        mpaths: I,
    ) -> anyhow::Result<()> {
        let printer = Printer::new();
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
