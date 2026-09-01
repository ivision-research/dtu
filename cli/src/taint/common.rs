use std::sync::Arc;

use clap::{self, Args};

use dtu::{
    analysis::{
        db::taint::{db::GraphTaintAnalysisDb, writer::RunMeta},
        taint::{TaintAnalyzer, TaintAnalyzerOptions, TaintSeedOptions, TaintSeeds},
        SsaClassLoader,
    },
    db::{
        device::models::DiffedApkIPC,
        graph::{GraphDatabase, MethodSearch, MethodSearchParams, MethodSpec},
        ApkIPC, DeviceDatabase, Diffable,
    },
    utils::ClassName,
    Context,
};

use crate::{diff::get_diff_source, parsers::DiffSourceValueParser, utils::task_canceller};

/// Flags every analysis command shares, independent of what it selects
#[derive(Args)]
pub struct RunOpts {
    /// Output file
    #[arg(short, long)]
    pub out_file: String,

    /// Number of worker threads to analyze with
    #[arg(short = 'T', long)]
    pub threads: Option<usize>,

    /// Maximum recursion depth, this is also capped in the library itself
    #[arg(short = 'd', long)]
    pub depth: Option<usize>,
}

impl RunOpts {
    pub fn analyzer_options(&self) -> TaintAnalyzerOptions {
        let mut opts = TaintAnalyzerOptions::default();
        if let Some(threads) = self.threads {
            opts.num_threads = threads;
        }
        if let Some(depth) = self.depth {
            opts.set_depth(depth);
        }
        opts
    }
}

#[derive(Args)]
pub struct ComponentOpts {
    #[command(flatten)]
    pub run: RunOpts,

    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    pub only_new: bool,

    /// Set the diff source (only valid with -n/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    pub diff_source: Option<dtu::db::device::models::DiffSource>,

    /// Only show public entries
    #[arg(short = 'P', long)]
    pub only_public: bool,

    /// Only show enabled entries
    #[arg(short = 'E', long)]
    pub only_enabled: bool,
}

impl ComponentOpts {
    /// Everything that survives the export, enabled and diff filters.
    pub fn select<T, D>(
        &self,
        ctx: &dyn Context,
        meta: &dyn dtu::db::MetaDatabase,
        db: &DeviceDatabase,
        all: impl FnOnce() -> dtu::db::Result<Vec<T>>,
        diffed: impl FnOnce(i32) -> dtu::db::Result<Vec<D>>,
    ) -> anyhow::Result<Vec<T>>
    where
        T: ApkIPC,
        D: DiffedApkIPC<Inner = T> + Diffable + AsRef<T>,
    {
        if !self.only_new {
            return Ok(all()?
                .into_iter()
                .filter(|it| self.keep(it.is_enabled(), it.is_exported()))
                .collect());
        }

        let source = get_diff_source(ctx, meta, db, &self.diff_source)?;
        Ok(diffed(source.id)?
            .into_iter()
            .filter(|it| {
                let inner = it.as_ref();
                !it.in_diff() && self.keep(inner.is_enabled(), inner.is_exported())
            })
            .map(DiffedApkIPC::into_apk_ipc)
            .collect())
    }

    fn keep(&self, enabled: bool, exported: bool) -> bool {
        (!self.only_enabled || enabled) && (!self.only_public || exported)
    }
}

/// Every method the graph database has for the given class
///
/// Note that this also searches parent classes
pub fn methods_for_class(
    gdb: &dyn GraphDatabase,
    class: &ClassName,
    source: &str,
) -> anyhow::Result<Vec<MethodSpec>> {
    let mut methods = Vec::new();
    let parents = gdb.find_parent_classes_of(class, source)?;
    let search = MethodSearch::new(MethodSearchParams::ByClass { class }, Some(source), None);

    methods.extend(gdb.get_methods(&search)?);
    for parent in parents {
        let search = MethodSearch::new(
            MethodSearchParams::ByClass {
                class: &parent.name,
            },
            Some(&parent.source),
            None,
        );
        methods.extend(gdb.get_methods(&search)?);
    }
    Ok(methods)
}

pub struct Analysis {
    pub methods: Vec<MethodSpec>,
    pub seeds: Box<dyn TaintSeeds>,
    pub loader: Option<Arc<SsaClassLoader>>,
    pub seed: Option<TaintSeedOptions>,
}

impl Analysis {
    pub fn new(methods: Vec<MethodSpec>, seeds: impl TaintSeeds + 'static) -> Self {
        Self {
            methods,
            seeds: Box::new(seeds),
            loader: None,
            seed: None,
        }
    }

    pub fn with_loader(mut self, loader: Arc<SsaClassLoader>) -> Self {
        self.loader = Some(loader);
        self
    }

    /// How to look for methods beyond the ones resolved here
    pub fn with_seed_options(mut self, seed: TaintSeedOptions) -> Self {
        self.seed = Some(seed);
        self
    }
}

/// Run the taint analysis
///
/// `resolve` produces the methods and seeds, and only runs when the cache misses. Resolution is
/// the expensive part of every command, so nothing about it can appear in the key.
pub fn analyze<F>(
    ctx: &dyn Context,
    db: GraphTaintAnalysisDb,
    opts: &RunOpts,
    resolve: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&dyn GraphDatabase) -> anyhow::Result<Analysis>,
{
    let analysis = resolve(db.graph())?;
    let loader = match analysis.loader {
        Some(v) => v,
        None => Arc::new(
            SsaClassLoader::new(ctx)
                .ok_or_else(|| anyhow::anyhow!("failed to open the SSA class cache"))?,
        ),
    };
    let (_sigs, cancel) = task_canceller()?;
    let mut analyzer_opts = opts.analyzer_options();
    analyzer_opts.seed = analysis.seed;

    let opts_json = serde_json::to_string(&analyzer_opts)?;

    let meta = RunMeta::new(db.graph(), opts_json)?;
    let mut taint = TaintAnalyzer::new(ctx, cancel, analyzer_opts, &*analysis.seeds, loader);
    taint.run(analysis.methods, db, &meta)?;
    Ok(())
}
