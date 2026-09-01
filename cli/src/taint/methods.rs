use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    analysis::{
        db::GraphTaintAnalysisDb,
        taint::{TaintSeedOptions, TaintSource},
        typing::complex_param_sources,
    },
    db::graph::{models::MethodId, MethodSearch, MethodSpec, FRAMEWORK_SOURCE},
    utils::ClassName,
    Context,
};

use crate::parsers::GraphSourceValueParser;
use crate::taint::common::{analyze, Analysis, RunOpts};

/// Taint analysis over an arbitrary set of methods
///
/// The seed flags mirror how a [TaintSource] prints, so anything shown in a
/// report can be pasted straight back in.
#[derive(Args)]
pub struct Methods {
    #[command(flatten)]
    run: RunOpts,

    /// Class to analyze, required
    #[arg(short, long)]
    class: String,

    /// Only this method, otherwise every method on the class
    #[arg(short, long)]
    name: Option<String>,

    /// Only a method with this signature, e.g. Landroid/os/Bundle;I
    #[arg(short, long)]
    signature: Option<String>,

    /// Restrict to a single graph source
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,

    /// Seed a source, repeatable: `p1`, `Lfoo;->bar(I)`, `*->getIntent()Landroid/content/Intent;`,
    /// `Lfoo;->mField`
    #[arg(short = 't', long = "taint", value_name = "TAINT-SOURCE")]
    taints: Vec<TaintSource>,

    /// Seed every reference typed parameter of each method, by its own signature
    #[arg(short = 'C', long)]
    complex_params: bool,

    /// Also analyze methods the call graph says a seed is reachable from
    #[arg(long, default_value_t = false)]
    seed_indirect: bool,
}

impl Methods {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        if self.taints.is_empty() && !self.complex_params {
            anyhow::bail!("nothing to track, pass --taint or --complex-params");
        }

        let db = GraphTaintAnalysisDb::new_from_path(ctx, &self.run.out_file)?;

        analyze(ctx, db, &self.run, |gdb| {
            let class = ClassName::from(self.class.as_str());
            let search = MethodSearch::new_from_opts(
                Some(&class),
                self.name.as_deref(),
                self.signature.as_deref(),
                self.source.as_deref(),
                None,
            )
            .map_err(|e| anyhow::anyhow!("invalid method search: {e}"))?;

            let methods = gdb.get_methods(&search)?;
            if methods.is_empty() {
                anyhow::bail!("no methods matched, check the class name and source");
            }

            let (methods, seeds) = self.seed(methods)?;
            if methods.is_empty() {
                anyhow::bail!("every matched method ended up with no sources to track");
            }

            log::info!("{} methods seeded", methods.len());

            let analysis = Analysis::new(methods, seeds);
            Ok(match self.seed_options() {
                Some(seed) => analysis.with_seed_options(seed),
                None => analysis,
            })
        })?;

        Ok(())
    }

    /// Where the call graph may look for seeds, when indirect seeding was asked for
    ///
    /// Scoping to one source still needs the framework, since that is where most seed targets are
    /// declared.
    fn seed_options(&self) -> Option<TaintSeedOptions> {
        if !self.seed_indirect {
            return None;
        }
        let mut sources = Vec::new();
        if let Some(source) = self.source.as_deref() {
            sources.push(source.to_string());
            if source != FRAMEWORK_SOURCE {
                sources.push(String::from(FRAMEWORK_SOURCE));
            }
        }
        Some(TaintSeedOptions {
            sources,
            ..Default::default()
        })
    }

    /// Per method because `--complex-params` depends on each signature
    fn seed(
        &self,
        matched: Vec<MethodSpec>,
    ) -> anyhow::Result<(Vec<MethodSpec>, HashMap<MethodId, Vec<TaintSource>>)> {
        let mut methods = Vec::new();
        let mut seeds: HashMap<MethodId, Vec<TaintSource>> = HashMap::new();

        for method in matched {
            let mut sources = self.taints.clone();
            if self.complex_params {
                match complex_param_sources(&method.signature) {
                    Ok(v) => sources.extend(v),
                    Err(e) => log::warn!(
                        "cannot read the signature of {}->{}({}): {e}",
                        method.class,
                        method.name,
                        method.signature
                    ),
                }
            }

            sources.sort();
            sources.dedup();
            if sources.is_empty() {
                log::debug!(
                    "{}->{}({}) has no sources to track",
                    method.class,
                    method.name,
                    method.signature
                );
                continue;
            }

            seeds.insert(method.id, sources);
            methods.push(method);
        }

        Ok((methods, seeds))
    }
}
