use std::{
    borrow::Cow,
    env::{self, current_dir},
    fs::read_dir,
    io::{self, stdout, BufWriter, Write},
    ops::Deref,
    path::Path,
    str::FromStr,
};

use anyhow::bail;
use dtu::{
    db::{
        self,
        device::{
            db::SqlConnection,
            schema::{
                activities, apks, providers, receivers, services, system_service_methods,
                system_services,
            },
        },
        graph::{
            schema::{classes, methods, sources},
            GraphSqliteDatabase,
        },
        meta::get_default_metadb,
        DeviceDatabase, MetaDatabase,
    },
    diesel::prelude::*,
    DefaultContext,
};

use crate::command::{Completable, CompleteKind, FlagMap};

mod command;

#[derive(Clone, Copy)]
enum Shell {
    Fish,
    Zsh,
    Bash,
}

impl FromStr for Shell {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "fish" => Self::Fish,
            "zsh" => Self::Zsh,
            "bash" => Self::Bash,
            _ => bail!("invalid shell: {s}"),
        })
    }
}

struct CompleteContext {
    ctx: DefaultContext,
    kind: CompleteKind,
    incomplete: String,
    flag_map: FlagMap,
    shell: Shell,
}

impl Deref for CompleteContext {
    type Target = DefaultContext;
    fn deref(&self) -> &Self::Target {
        &self.ctx
    }
}

struct CompleteResult {
    completion: Cow<'static, str>,
    meta: Option<Cow<'static, str>>,
}

impl From<String> for CompleteResult {
    fn from(value: String) -> Self {
        Self::simple(value)
    }
}

impl<'a> From<Cow<'a, str>> for CompleteResult {
    fn from(value: Cow<'a, str>) -> Self {
        Self::from(value.into_owned())
    }
}

impl From<&str> for CompleteResult {
    fn from(value: &str) -> Self {
        Self::simple(String::from(value))
    }
}

impl From<(String, String)> for CompleteResult {
    fn from(value: (String, String)) -> Self {
        Self::new(value.0, Some(value.1))
    }
}

impl From<(String, Option<String>)> for CompleteResult {
    fn from(value: (String, Option<String>)) -> Self {
        Self::new(value.0, value.1)
    }
}

impl CompleteResult {
    fn new<T: Into<Cow<'static, str>>>(completion: T, meta: Option<T>) -> Self {
        Self {
            completion: completion.into(),
            meta: meta.map(|it| it.into()),
        }
    }

    fn simple<T: Into<Cow<'static, str>>>(completion: T) -> Self {
        Self::new(completion, None)
    }

    fn print<W: Write>(&self, shell: Shell, out: &mut BufWriter<W>) -> io::Result<()> {
        match &self.meta {
            None => self.print_simple(out),
            Some(meta) => match shell {
                Shell::Fish => self.print_fish(meta, out),
                Shell::Bash => self.print_simple(out),
                Shell::Zsh => self.print_zsh(meta, out),
            },
        }
    }

    fn print_simple<W: Write>(&self, out: &mut BufWriter<W>) -> io::Result<()> {
        out.write_all(self.completion.as_bytes())?;
        out.write_all(&[b'\n'])
    }

    fn print_fish<W: Write>(&self, meta: &str, out: &mut BufWriter<W>) -> io::Result<()> {
        out.write_all(self.completion.as_bytes())?;
        out.write_all(&[b'\t'])?;
        out.write_all(meta.as_bytes())?;
        out.write_all(&[b'\n'])
    }

    fn print_zsh<W: Write>(&self, meta: &str, out: &mut BufWriter<W>) -> io::Result<()> {
        out.write_all(self.completion.as_bytes())?;
        out.write_all(&[b'\t'])?;
        out.write_all(meta.as_bytes())?;
        out.write_all(&[b'\n'])
    }
}

type DBResult<T> = std::result::Result<T, db::Error>;

impl CompleteContext {
    fn new(ctx: DefaultContext) -> anyhow::Result<Self> {
        let shell = match env::var("DTUC_SHELL") {
            Ok(v) => Shell::from_str(&v)?,
            _ => Shell::Bash,
        };
        // Remove leading quotes if they are passed to us
        let mut incomplete = env::var("DTUC_INCOMPLETE")?;
        incomplete = String::from(incomplete.trim_start_matches('\''));

        let skip_count = match env::var("DTUC_SKIP") {
            Ok(v) => str::parse::<usize>(&v).unwrap_or(2),
            Err(_) => 2,
        };

        let drop_count = match env::var("DTUC_DROP") {
            Ok(v) => str::parse::<usize>(&v).unwrap_or(0),
            Err(_) => 0,
        };

        let mut args = env::args().skip(skip_count).collect::<Vec<String>>();

        if drop_count > 0 {
            args.truncate(args.len() - drop_count);
        }
        let (kind, flag_map) = CompleteKind::find(args);

        Ok(Self {
            ctx,
            incomplete,
            shell,
            kind,
            flag_map,
        })
    }

    fn graph_conn_completion_like<OnEmpty, OnPartial, R>(
        &self,
        on_empty: OnEmpty,
        on_partial: OnPartial,
    ) -> anyhow::Result<()>
    where
        R: Into<CompleteResult> + Send,
        OnEmpty: FnOnce(&mut SqlConnection) -> QueryResult<Vec<R>> + Send,
        OnPartial: FnOnce(&mut SqlConnection, &str) -> QueryResult<Vec<R>> + Send,
    {
        let db = GraphSqliteDatabase::new(&self.ctx)?;
        let results = if self.incomplete.is_empty() {
            db.with_connection(on_empty)
        } else {
            db.with_connection(|c| on_partial(c, &self.incomplete))
        }?;

        self.show_results(results.into_iter().map(<R as Into<CompleteResult>>::into))
    }

    fn conn_simple<Get, R>(&self, get: Get) -> anyhow::Result<()>
    where
        R: Into<CompleteResult> + Send,
        Get: FnOnce(&mut SqlConnection) -> QueryResult<Vec<R>> + Send,
    {
        let db = DeviceDatabase::new(&self.ctx)?;
        let results = db.with_connection(get)?;
        self.show_results(results.into_iter().map(<R as Into<CompleteResult>>::into))
    }

    fn conn_completion_like<OnEmpty, OnPartial, R>(
        &self,
        on_empty: OnEmpty,
        on_partial: OnPartial,
    ) -> anyhow::Result<()>
    where
        R: Into<CompleteResult> + Send,
        OnEmpty: FnOnce(&mut SqlConnection) -> QueryResult<Vec<R>> + Send,
        OnPartial: FnOnce(&mut SqlConnection, &str) -> QueryResult<Vec<R>> + Send,
    {
        let db = DeviceDatabase::new(&self.ctx)?;
        let results = if self.incomplete.is_empty() {
            db.with_connection(on_empty)
        } else {
            db.with_connection(|c| on_partial(c, &self.incomplete))
        }?;

        self.show_results(results.into_iter().map(<R as Into<CompleteResult>>::into))
    }

    fn db_completion_like<OnEmpty, OnPartial, Extract, R>(
        &self,
        on_empty: OnEmpty,
        on_partial: OnPartial,
        extract: Extract,
    ) -> anyhow::Result<()>
    where
        OnEmpty: FnOnce(DeviceDatabase) -> DBResult<Vec<R>>,
        OnPartial: FnOnce(DeviceDatabase, &str) -> DBResult<Vec<R>>,
        Extract: Fn(R) -> CompleteResult,
    {
        let db = DeviceDatabase::new(&self.ctx)?;
        let results = if self.incomplete.is_empty() {
            on_empty(db)
        } else {
            on_partial(db, &self.incomplete)
        }?;

        self.show_results(results.into_iter().map(extract))
    }

    fn complete_system_service(self) -> anyhow::Result<()> {
        self.db_completion_like(
            |db| db.get_system_services(),
            |db, inc| db.get_system_services_name_like(&format!("{}%", inc)),
            |s| s.name.into(),
        )
    }

    fn complete_system_service_method(self) -> anyhow::Result<()> {
        let Some(system_service_name) = self.flag_map.get_flag("service", "s") else {
            bail!("need -s/--service for system service method")
        };
        self.conn_completion_like(
            |c| {
                system_service_methods::table
                    .inner_join(system_services::table)
                    .select(system_service_methods::name)
                    .filter(system_services::name.eq(&system_service_name))
                    .get_results::<String>(c)
            },
            |c, inc| {
                system_service_methods::table
                    .inner_join(system_services::table)
                    .select(system_service_methods::name)
                    .filter(system_services::name.eq(&system_service_name))
                    .filter(system_service_methods::name.like(&format!("{}%", inc)))
                    .get_results::<String>(c)
            },
        )
    }

    fn complete_activity(self) -> anyhow::Result<()> {
        let Some((pkg, name)) = self.incomplete.split_once('/') else {
            return self.complete_component_pkg();
        };

        self.conn_simple(|c| {
            Ok(activities::table
                .inner_join(apks::table)
                .select(activities::class_name)
                .filter(apks::app_name.eq(pkg))
                .filter(activities::class_name.like(&format!("{name}%")))
                .get_results::<String>(c)?
                .into_iter()
                .map(|it| format!("{pkg}/{it}"))
                .collect::<Vec<String>>())
        })
    }

    fn complete_service(self) -> anyhow::Result<()> {
        let Some((pkg, name)) = self.incomplete.split_once('/') else {
            return self.complete_component_pkg();
        };

        self.conn_simple(|c| {
            Ok(services::table
                .inner_join(apks::table)
                .select(services::class_name)
                .filter(apks::app_name.eq(pkg))
                .filter(services::class_name.like(&format!("{name}%")))
                .get_results::<String>(c)?
                .into_iter()
                .map(|it| format!("{pkg}/{it}"))
                .collect::<Vec<String>>())
        })
    }

    fn complete_receiver(self) -> anyhow::Result<()> {
        let Some((pkg, name)) = self.incomplete.split_once('/') else {
            return self.complete_component_pkg();
        };

        self.conn_simple(|c| {
            Ok(receivers::table
                .inner_join(apks::table)
                .select(receivers::class_name)
                .filter(apks::app_name.eq(pkg))
                .filter(receivers::class_name.like(&format!("{name}%")))
                .get_results::<String>(c)?
                .into_iter()
                .map(|it| format!("{pkg}/{it}"))
                .collect::<Vec<String>>())
        })
    }

    fn complete_provider(self) -> anyhow::Result<()> {
        let Some((pkg, name)) = self.incomplete.split_once('/') else {
            return self.complete_component_pkg();
        };

        self.conn_simple(|c| {
            Ok(providers::table
                .inner_join(apks::table)
                .select(providers::name)
                .filter(apks::app_name.eq(pkg))
                .filter(providers::name.like(&format!("{name}%")))
                .get_results::<String>(c)?
                .into_iter()
                .map(|it| format!("{pkg}/{it}"))
                .collect::<Vec<String>>())
        })
    }

    fn complete_provider_authority(self) -> anyhow::Result<()> {
        let db = DeviceDatabase::new(&self.ctx)?;
        let auths = db.with_connection(|c| -> QueryResult<Vec<CompleteResult>> {
            Ok(providers::table
                .select(providers::authorities)
                .get_results::<String>(c)?
                .into_iter()
                .flat_map(|it| {
                    it.split(';')
                        .map(CompleteResult::from)
                        .collect::<Vec<CompleteResult>>()
                })
                .collect::<Vec<CompleteResult>>())
        })?;
        self.filter_and_show_results(auths.into_iter())
    }

    fn complete_component_pkg(&self) -> anyhow::Result<()> {
        self.conn_completion_like(
            |c| apks::table.select(apks::app_name).get_results::<String>(c),
            |c, inc| {
                apks::table
                    .select(apks::app_name)
                    .filter(apks::app_name.like(&format!("{inc}%")))
                    .get_results::<String>(c)
            },
        )
    }

    fn filter_by_and_show_results<T: Iterator<Item = CompleteResult>>(
        &self,
        filter: &str,
        results: T,
    ) -> anyhow::Result<()> {
        let mut out = BufWriter::new(stdout().lock());
        for r in results {
            if r.completion.starts_with(filter) {
                r.print(self.shell, &mut out)?;
            }
        }
        Ok(())
    }

    fn filter_and_show_results<T: Iterator<Item = CompleteResult>>(
        &self,
        results: T,
    ) -> anyhow::Result<()> {
        self.filter_by_and_show_results(&self.incomplete, results)
    }

    fn show_results<T: Iterator<Item = CompleteResult>>(&self, results: T) -> anyhow::Result<()> {
        let mut out = BufWriter::new(stdout().lock());
        for r in results {
            r.print(self.shell, &mut out)?;
        }
        Ok(())
    }

    //fn complete_system_service_method(self, ssm: SystemServiceMethod) -> anyhow::Result<()> {
    //    self.db_complete_selector_startswith(
    //        |db| {
    //            let svc = db.get_system_service_by_name(&ssm.service)?;
    //            db.get_system_service_methods_by_service_id(svc.id)
    //        },
    //        |e| e.name,
    //    )
    //}

    fn complete(self) -> anyhow::Result<()> {
        match &self.kind {
            CompleteKind::Uncompletable => Ok(()),
            CompleteKind::SystemService => self.complete_system_service(),
            CompleteKind::SystemServiceMethod => self.complete_system_service_method(),
            CompleteKind::GraphSource => self.complete_graph_source(),
            CompleteKind::GraphMethod => self.complete_graph_method(),
            CompleteKind::GraphClass => self.complete_graph_class(),
            CompleteKind::GraphSignature => self.complete_graph_signature(),

            CompleteKind::Provider => self.complete_provider(),
            CompleteKind::Activity => self.complete_activity(),
            CompleteKind::Receiver => self.complete_receiver(),
            CompleteKind::Service => self.complete_service(),

            CompleteKind::ProviderAuthority => self.complete_provider_authority(),

            CompleteKind::TestName => self.complete_test_name(),

            CompleteKind::Dir => self.complete_path(false),
            CompleteKind::File => self.complete_path(true),

            CompleteKind::Apk => self.complete_apk(),
            CompleteKind::List(l) => self.dump_list(l),
            _ => bail!("can't complete kind {:?}", self.kind),
        }
    }

    fn complete_path_empty(self, include_files: bool) -> anyhow::Result<()> {
        let cwd = current_dir()?;
        let completions = read_dir(&cwd)?.filter_map(|it| {
            let ent = it.ok()?;
            let md = ent.metadata().ok()?;
            if !include_files && md.is_file() {
                return None;
            }
            let p = ent.path();
            let mut name = Cow::Borrowed(p.file_name()?.to_str()?);
            if p.is_dir() {
                name = Cow::Owned(format!("{name}/"));
            }
            Some(CompleteResult::from(name))
        });

        self.show_results(completions)
    }

    fn complete_path(self, include_files: bool) -> anyhow::Result<()> {
        if self.incomplete.is_empty() {
            return self.complete_path_empty(include_files);
        }

        let mut in_cwd = false;

        let p = Path::new(&self.incomplete);
        let base = if p.is_dir() {
            Cow::Borrowed(p)
        } else {
            match p.parent() {
                Some(v) if v.is_dir() => Cow::Borrowed(v),
                _ => {
                    in_cwd = true;
                    Cow::Owned(current_dir()?)
                }
            }
        };

        let completions = read_dir(&base)?.filter_map(|it| {
            let ent = it.ok()?;
            let md = ent.metadata().ok()?;
            if !include_files && md.is_file() {
                return None;
            }
            let p = ent.path();
            let mut path = Cow::Borrowed(if in_cwd {
                p.file_name()?.to_str()?
            } else {
                p.to_str()?
            });

            if p.is_dir() {
                path = Cow::Owned(format!("{path}/"));
            }

            if path.starts_with(&self.incomplete) {
                Some(CompleteResult::from(path))
            } else {
                None
            }
        });
        self.show_results(completions)
    }

    fn complete_test_name(self) -> anyhow::Result<()> {
        let meta = get_default_metadb(&self.ctx)?;
        let results = meta
            .get_app_activities()?
            .into_iter()
            .map(|it| CompleteResult::from(it.name));

        self.filter_and_show_results(results)
    }

    fn complete_apk(self) -> anyhow::Result<()> {
        self.conn_completion_like(
            |c| apks::table.select(apks::name).get_results::<String>(c),
            |c, inc| {
                apks::table
                    .select(apks::name)
                    .filter(apks::name.like(&format!("{inc}%")))
                    .get_results::<String>(c)
            },
        )
    }

    fn complete_graph_class(self) -> anyhow::Result<()> {
        self.graph_conn_completion_like(
            |_| Ok(Vec::<String>::new()),
            |c, pinc| {
                let inc = if pinc.starts_with('L') {
                    Cow::Borrowed(pinc)
                } else {
                    Cow::Owned(format!("L{}", pinc.replace('.', "/")))
                };

                classes::table
                    .filter(classes::name.like(&format!("{}%", inc)))
                    .select(classes::name)
                    .limit(100)
                    .get_results::<String>(c)
            },
        )
    }
    fn complete_graph_signature(self) -> anyhow::Result<()> {
        let Some(cname) = self.flag_map.get_flag("class", "c") else {
            bail!("need -c/--class for graph method")
        };

        let Some(method) = self
            .flag_map
            .get_first_flag(&[("method", "m"), ("name", "n")])
        else {
            bail!("need a method for method signature");
        };

        self.graph_conn_completion_like(
            |c| {
                methods::table
                    .inner_join(classes::table)
                    .select(methods::args)
                    .filter(methods::name.eq(&method))
                    .filter(classes::name.eq(&cname))
                    .get_results::<String>(c)
            },
            |c, inc| {
                methods::table
                    .inner_join(classes::table)
                    .select(methods::args)
                    .filter(methods::name.eq(&method))
                    .filter(classes::name.eq(&cname))
                    .filter(methods::args.like(&format!("{}%", inc)))
                    .get_results::<String>(c)
            },
        )
    }

    fn complete_graph_method(self) -> anyhow::Result<()> {
        let Some(cname) = self.flag_map.get_flag("class", "c") else {
            bail!("need -c/--class for graph method")
        };

        self.graph_conn_completion_like(
            |c| {
                methods::table
                    .inner_join(classes::table)
                    .select(methods::name)
                    .filter(classes::name.eq(&cname))
                    .get_results::<String>(c)
            },
            |c, inc| {
                methods::table
                    .inner_join(classes::table)
                    .select(methods::name)
                    .filter(classes::name.eq(&cname))
                    .filter(methods::name.like(&format!("{}%", inc)))
                    .get_results::<String>(c)
            },
        )
    }

    fn complete_graph_source(self) -> anyhow::Result<()> {
        self.graph_conn_completion_like(
            |c| {
                sources::table
                    .select(sources::name)
                    .get_results::<String>(c)
            },
            |c, inc| {
                sources::table
                    .select(sources::name)
                    .filter(sources::name.like(&format!("%{inc}%")))
                    .get_results::<String>(c)
            },
        )
    }

    fn dump_list(&self, results: &[Completable]) -> anyhow::Result<()> {
        let mut crs = Vec::new();

        for it in results {
            match it {
                Completable::Flag(f) => {
                    if !f.long.is_empty() && f.long.starts_with(&self.incomplete) {
                        crs.push(CompleteResult::new(f.long, Some(f.help)));
                    }
                    if !f.short.is_empty() && f.short.starts_with(&self.incomplete) {
                        crs.push(CompleteResult::new(f.short, Some(f.help)));
                    }
                }
                Completable::Simple(s) => {
                    if s.name.starts_with(&self.incomplete) {
                        crs.push(CompleteResult::new(s.name, Some(s.help)));
                    }
                }
            }
        }

        self.show_results(crs.into_iter())
    }
}

fn main() -> anyhow::Result<()> {
    let debug = env::var("DTUC_DEBUG").is_ok();

    let ctx = CompleteContext::new(DefaultContext::new())?;
    let res = ctx.complete();
    if debug {
        return res;
    }
    Ok(())
}
