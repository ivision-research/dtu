use std::{
    collections::{HashMap, HashSet},
    io::stdout,
};

use anyhow::bail;
use clap::{self, Args};
use dtu::{
    analysis::db::taint::{
        db::{
            Filter as AnalysisFilter, GraphTaintAnalysisDb, ReportIdMap, ResolvableIds,
            UnresolvedOrigin, UnresolvedTaintRoutes, UnresolvedTaintSinkKind,
        },
        models::{AnalyzedMethodId, MethodStatus},
    },
    db::graph::{models::MethodId, GraphDatabase, MethodSearch},
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName},
    Context,
};

#[derive(Args)]
pub struct CommonOpts {}

#[derive(Args)]
pub struct Filter {
    /// Input result file
    #[arg(short = 'F', long)]
    pub file: String,

    /// An optional class name for the entry method
    #[arg(short, long)]
    class: Option<ClassName>,

    /// An optional method name for the entry method
    #[arg(short, long)]
    name: Option<String>,

    /// Optional signature for the entry method
    #[arg(short, long)]
    signature: Option<String>,

    /// Optional source for the entry method
    #[arg(short = 'S', long)]
    source: Option<String>,

    /// Ignore any cached result
    #[arg(long)]
    no_cache: bool,

    #[arg(short = 'f', long = "filter")]
    /// Filters to use, can be provided multiple times
    filters: Vec<String>,
}

#[derive(serde::Serialize)]
struct JsonTaintSink {
    in_method: i32,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i32>,
}

#[derive(serde::Serialize)]
struct JsonRoute {
    source: i32,
    sinks: Vec<JsonTaintSink>,
}

#[derive(serde::Serialize)]
struct JsonAnalyzedMethod {
    entry_method: i32,
    origins: Option<Vec<Vec<MethodId>>>,
    routes: Vec<JsonRoute>,
}

#[derive(serde::Serialize)]
struct JsonOutput {
    results: Vec<JsonAnalyzedMethod>,
    map: ReportIdMap,
}

impl Filter {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;

        if self.filters.len() == 0 {
            bail!("no filters provided");
        }

        // TODO This currently isn't cacheable until we rework for a yoked version of caching postcards.

        let mut filters = Vec::with_capacity(self.filters.len());

        for f in &self.filters {
            filters.push(f.parse()?);
        }

        let out = self.run_inner(ctx, &filters)?;
        serde_json::to_writer(stdout().lock(), &out)?;
        Ok(())
    }

    fn run_inner(
        self,
        ctx: &dyn Context,
        filters: &[AnalysisFilter],
    ) -> anyhow::Result<JsonOutput> {
        let db = GraphTaintAnalysisDb::new_from_path(ctx, &self.file)?;
        let gdb = db.graph();
        let mids = self.get_method_filter(gdb)?;

        let mut analyzed_methods = db.get_all_analyzed_methods()?;

        if let Some(mids) = mids {
            analyzed_methods = analyzed_methods
                .into_iter()
                .filter(|it| mids.contains(&it.method))
                .collect::<Vec<_>>();
        }

        if analyzed_methods.is_empty() {
            bail!("no analyzed methods found");
        }

        let matching_anas = db.get_analysis_matching(filters)?;
        let mut matching_routes = HashSet::new();
        for id in matching_anas.iter().copied() {
            matching_routes.extend(db.get_routes_matching(id, filters)?);
        }
        let matching_anas: HashSet<AnalyzedMethodId> =
            HashSet::from_iter(matching_anas.iter().copied());

        let mut analyses = Vec::new();

        let mut seen_sinks = HashSet::new();
        let mut seen_methods = HashSet::new();
        let mut seen_sources = HashSet::new();

        // TODO: This can probably get huge, but it probably won't ever get so huge that we can't do
        // it. Ideally we could apply the filtering to the map retrieval so we only get exactly what
        // we want, but programming is hard.
        let map = db.get_complete_report_id_map(ResolvableIds::All)?;

        for ana in analyzed_methods {
            if !matches!(ana.status, MethodStatus::Done) {
                continue;
            }
            if !matching_anas.contains(&ana.id) {
                continue;
            }

            let routes = db.get_routes_for_analysis(&ana)?;

            let UnresolvedTaintRoutes { origin, routes } = routes;

            let routes = routes
                .into_iter()
                .filter(|it| matching_routes.contains(&it.id))
                .collect::<Vec<_>>();

            if routes.is_empty() {
                continue;
            }

            let mut jroutes = Vec::new();
            seen_methods.insert(ana.method.raw());

            for route in routes {
                seen_sources.insert(route.source.raw());
                let mut jsinks = Vec::new();
                for sink in route.sinks {
                    let (kind, id) = match sink.sink {
                        UnresolvedTaintSinkKind::Phi => ("phi", None),
                        UnresolvedTaintSinkKind::Array => ("array", None),
                        UnresolvedTaintSinkKind::Field(id) => ("field", Some(id.raw())),
                        UnresolvedTaintSinkKind::Call(id) => {
                            seen_methods.insert(id.raw());
                            ("call", Some(id.raw()))
                        }
                        UnresolvedTaintSinkKind::Instruction(id) => ("instruction", Some(id.raw())),
                        UnresolvedTaintSinkKind::ExternalCall(id) => ("external", Some(id.raw())),
                    };

                    if let Some(id) = id {
                        // Calls are already accounted for in the methods entry
                        if kind != "call" {
                            seen_sinks.insert(id);
                        }
                    }

                    let jts = JsonTaintSink {
                        kind,
                        id,
                        in_method: sink.in_method.raw(),
                    };
                    jsinks.push(jts);
                }
                jroutes.push(JsonRoute {
                    source: route.source.raw(),
                    sinks: jsinks,
                });
            }

            analyses.push(JsonAnalyzedMethod {
                entry_method: ana.method.raw(),
                routes: jroutes,
                origins: match origin {
                    UnresolvedOrigin::Direct => None,
                    UnresolvedOrigin::CallGraph { chains } => Some(chains),
                },
            });
        }

        let ReportIdMap {
            mut methods,
            mut sinks,
            mut sources,
        } = map;

        let methods = methods.take().map(|it| {
            it.into_iter()
                .filter(|(k, _)| seen_methods.contains(&k.raw()))
                .collect::<HashMap<_, _>>()
        });

        let sinks = sinks.take().map(|it| {
            it.into_iter()
                .filter(|(k, _)| seen_sinks.contains(&k.raw()))
                .collect::<HashMap<_, _>>()
        });

        let sources = sources.take().map(|it| {
            it.into_iter()
                .filter(|(k, _)| seen_sources.contains(&k.raw()))
                .collect::<HashMap<_, _>>()
        });

        let output = JsonOutput {
            results: analyses,
            map: ReportIdMap {
                methods,
                sinks,
                sources,
            },
        };

        Ok(output)
    }

    fn get_method_filter(
        &self,
        gdb: &dyn GraphDatabase,
    ) -> anyhow::Result<Option<HashSet<MethodId>>> {
        // Should check other things here, but this will be an error when trying to build the method
        // search
        if self.class.is_none() && self.name.is_none() {
            return Ok(None);
        }

        let ms = MethodSearch::new_from_opts(
            self.class.as_ref(),
            self.name.as_deref(),
            self.signature.as_deref(),
            self.source.as_deref(),
            None,
        )
        .map_err(anyhow::Error::msg)?;

        let mids = HashSet::from_iter(gdb.get_method_ids(&ms)?.into_iter());

        if mids.is_empty() {
            bail!("method selection flags turned up no methods in the graph database");
        }

        Ok(Some(mids))
    }
}
