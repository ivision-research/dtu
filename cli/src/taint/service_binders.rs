use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use clap::{self, Args};
use dtu::analysis::db::GraphTaintAnalysisDb;
use dtu::{
    analysis::{
        get_ssa_method,
        taint::TaintSource,
        typing::{complex_param_sources, resolve_returned_classes},
        SsaClassLoader,
    },
    db::{
        graph::{
            models::MethodId, ClassSearch, GraphDatabase, MethodSearch, MethodSearchParams,
            MethodSpec,
        },
        meta::get_default_metadb,
        ApkComponent, ApkIPC, DeviceDatabase, MetaDatabase,
    },
    prereqs::Prereq,
    utils::ClassName,
    Context,
};

use crate::taint::common::{analyze, Analysis, ComponentOpts};

#[derive(Args)]
pub struct ServiceBinders {
    #[command(flatten)]
    opts: ComponentOpts,
}

/// Tell the user which services got no analysis at all, so a gap is visible without the log
///
/// This goes to stderr rather than the log because the report on stdout gives no hint that a
/// service was dropped.
fn report_unresolved(unresolved: &[(ClassName, String)]) {
    if unresolved.is_empty() {
        return;
    }
    let mut sorted = unresolved.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    eprintln!("\n{} services had no binder analyzed:", sorted.len());
    let mut last = "";
    for (class, reason) in sorted {
        if reason != last {
            eprintln!("  {reason}:");
            last = reason;
        }
        eprintln!("    {class}");
    }
}

/// A binder implementation together with the interface it serves
struct Endpoint {
    binder: ClassName,
    iface: ClassName,
    source: String,
}

impl ServiceBinders {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let db = GraphTaintAnalysisDb::new_from_path(ctx, &self.opts.run.out_file)?;

        let mut unresolved: Vec<(ClassName, String)> = Vec::new();

        analyze(ctx, db, &self.opts.run, |gdb| {
            let meta = get_default_metadb(ctx)?;
            meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
            let db = DeviceDatabase::new(ctx)?;

            let services = self.opts.select(
                ctx,
                &meta,
                &db,
                || db.get_services(),
                |id| db.get_service_diffs_by_diff_id(id),
            )?;

            let loader = Arc::new(
                SsaClassLoader::new(ctx)
                    .ok_or_else(|| anyhow::anyhow!("failed to open the SSA class cache"))?,
            );
            let mut endpoints = Vec::new();

            for service in services {
                let apk = db.get_apk_by_id(service.get_apk_id())?;
                let source = apk.device_path.as_squashed_str().to_string();
                let class = service.get_class_name();

                match self.resolve_endpoints(ctx, gdb, &loader, &class, &source) {
                    Ok(found) if found.is_empty() => {
                        log::warn!("no binder interface resolved for {class}, skipping it");
                        unresolved.push((class, String::from("no binder interface resolved")));
                    }
                    Ok(found) => endpoints.extend(found),
                    Err(e) => {
                        log::warn!("failed to resolve the binder for {class}: {e}");
                        unresolved.push((class, e.to_string()));
                    }
                }
            }

            let (methods, seeds) = self.seed(gdb, &endpoints)?;

            log::info!(
                "{} endpoints across {} transaction methods, {} services unresolved",
                endpoints.len(),
                methods.len(),
                unresolved.len()
            );

            if methods.is_empty() {
                anyhow::bail!("no binder transaction methods to analyze");
            }

            // resolve_endpoints already read these classes so keep the same loader with the cache
            Ok(Analysis::new(methods, seeds).with_loader(loader))
        })?;

        report_unresolved(&unresolved);
        Ok(())
    }

    /// Resolve the binder(s) a service's `onBind` returns and the interface each serves
    fn resolve_endpoints(
        &self,
        ctx: &dyn Context,
        gdb: &dyn GraphDatabase,
        loader: &SsaClassLoader,
        class: &ClassName,
        source: &str,
    ) -> anyhow::Result<Vec<Endpoint>> {
        let on_bind = self.find_on_bind(gdb, class, source)?;
        let ssa_class =
            loader.get_ssa_class(ctx, on_bind.class_id, &on_bind.class, &on_bind.source)?;
        let Some(ssa) = get_ssa_method(ssa_class.get(), &on_bind) else {
            anyhow::bail!("onBind has no body");
        };

        let mut out = Vec::new();
        for binder in resolve_returned_classes(&ssa) {
            // A bare IBinder means resolution landed on the declared type and learned nothing. We
            // could do a deeper resolution path, but that's work for another day. Wonder how long
            // this comment will stay around?
            if binder.get_smali_name() == "Landroid/os/IBinder;" {
                log::warn!("{class}->onBind only resolved to IBinder, cannot find the interface");
                continue;
            }
            out.extend(self.resolve_candidates(gdb, class, &binder, source)?);
        }
        Ok(out)
    }

    /// `resolve_returned_classes` only reports the declared type of the returned value, which
    /// may be:
    ///
    /// - the concrete binder itself (the common `new-instance` case) - used as is
    /// - an abstract class such as `IFoo$Stub` (a field typed as the Stub, initialized with an
    ///   anonymous subclass) - also resolve to its concrete subclasses.
    /// - the AIDL interface itself (a field typed as the bare interface) - resolve to the classes
    ///   implementing it, rather than classes extending it
    fn resolve_candidates(
        &self,
        gdb: &dyn GraphDatabase,
        service: &ClassName,
        binder: &ClassName,
        source: &str,
    ) -> anyhow::Result<Vec<Endpoint>> {
        // If we found classes implementing the class, then it is an interface and we should use
        // those child classes. We return early here and don't include the class itself in the list.
        let implementers =
            gdb.find_classes_implementing(&ClassSearch::new(binder, None), Some(source))?;
        if !implementers.is_empty() {
            return Ok(implementers
                .into_iter()
                .map(|it| Endpoint {
                    binder: it.name,
                    iface: binder.clone(),
                    source: source.to_string(),
                })
                .collect());
        }

        // Otherwise it might be abstract
        let mut binders = vec![binder.clone()];
        binders.extend(
            gdb.find_child_classes_of(&ClassSearch::new(binder, None), Some(source))?
                .into_iter()
                .map(|it| it.name),
        );

        let mut out = Vec::new();
        for binder in binders {
            let ifaces = self.find_interfaces(gdb, &binder)?;
            if ifaces.is_empty() {
                log::warn!("binder {binder} returned by {service} serves no AIDL interface");
                continue;
            }
            for iface in ifaces {
                out.push(Endpoint {
                    binder: binder.clone(),
                    iface,
                    source: source.to_string(),
                });
            }
        }
        Ok(out)
    }

    /// `onBind` may be declared on a base class rather than the service itself
    fn find_on_bind(
        &self,
        gdb: &dyn GraphDatabase,
        class: &ClassName,
        source: &str,
    ) -> anyhow::Result<MethodSpec> {
        let mut candidates = vec![class.clone()];
        candidates.extend(
            gdb.find_parent_classes_of(class, source)?
                .into_iter()
                .map(|it| it.name),
        );

        for candidate in candidates {
            let search = MethodSearch::new(
                MethodSearchParams::ByFullSpec {
                    class: &candidate,
                    name: "onBind",
                    signature: "Landroid/content/Intent;",
                },
                None,
                None,
            );
            if let Some(found) = gdb.get_methods(&search)?.into_iter().next() {
                return Ok(found);
            }
        }
        anyhow::bail!("no onBind found on the class or its parents")
    }

    /// The AIDL interfaces a binder serves
    fn find_interfaces(
        &self,
        gdb: &dyn GraphDatabase,
        binder: &ClassName,
    ) -> anyhow::Result<Vec<ClassName>> {
        const IINTERFACE: &str = "Landroid/os/IInterface;";
        const IBINDER: &str = "Landroid/os/IBinder;";

        // The source is left open because the binder often lives in a different apk
        // than the service that returns it.
        let ifaces = gdb.find_interfaces_of(&ClassSearch::new(binder, None))?;
        if !ifaces
            .iter()
            .any(|it| it.name.get_smali_name() == IINTERFACE)
        {
            return Ok(Vec::new());
        }

        Ok(ifaces
            .into_iter()
            .map(|it| it.name)
            .filter(|it| {
                let name = it.get_smali_name();
                name != IINTERFACE && name != IBINDER
            })
            .collect())
    }

    /// Seed the reference typed parameters of every transaction method
    ///
    /// Keyed per method: a [TaintSource::Param] register only means anything
    /// against the signature it came from, so these cannot be pooled.
    fn seed(
        &self,
        gdb: &dyn GraphDatabase,
        endpoints: &[Endpoint],
    ) -> anyhow::Result<(Vec<MethodSpec>, HashMap<MethodId, Vec<TaintSource>>)> {
        let mut methods = Vec::new();
        let mut seeds: HashMap<MethodId, Vec<TaintSource>> = HashMap::new();

        for endpoint in endpoints {
            // The interface declares the transactions, the binder implements them
            let iface_search = MethodSearch::new(
                MethodSearchParams::ByClass {
                    class: &endpoint.iface,
                },
                None,
                None,
            );
            let transactions = gdb
                .get_methods(&iface_search)?
                .into_iter()
                .map(|it| (it.name, it.signature))
                .collect::<BTreeSet<_>>();

            let impl_search = MethodSearch::new(
                MethodSearchParams::ByClass {
                    class: &endpoint.binder,
                },
                Some(endpoint.source.as_str()),
                None,
            );

            for method in gdb.get_methods(&impl_search)? {
                if !transactions.contains(&(method.name.clone(), method.signature.clone())) {
                    continue;
                }
                // An abstract candidate (kept in case it implements some transactions
                // concretely) leaves the rest as bodiless declarations - nothing to seed.
                if !method.has_body() {
                    continue;
                }
                let params = complex_param_sources(&method.signature)
                    .map_err(|e| anyhow::anyhow!("bad signature {}: {e}", method.signature))?;
                if params.is_empty() {
                    continue;
                }
                seeds.insert(method.id, params);
                methods.push(method);
            }
        }

        Ok((methods, seeds))
    }
}
