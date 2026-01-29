use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::db::device::models::SystemService;
use dtu::db::DeviceDatabase;
use dtu::utils::{
    find_files_for_class, find_smali_file_for_class, try_proj_home_relative, ClassName,
};
use dtu::Context;
use std::path::PathBuf;

use crate::parsers::SystemServiceValueParser;
use crate::utils::{exec_open_file, prompt_choice, vec_to_single};

#[derive(Args)]
pub struct ServiceFile {
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: SystemService,

    /// Open the file in $EDITOR (or $DTU_EDITOR if set)
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    open: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Impl;

#[derive(Args)]
struct Interface;

#[derive(Args)]
struct Stub;

#[derive(Args)]
struct Proxy;

#[derive(Subcommand)]
enum Command {
    Impl,
    Interface,
    Stub,
    Proxy,
}

impl ServiceFile {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let class_file = match &self.command {
            Command::Impl => self.find_impl(ctx)?,
            Command::Stub => self.get_class_file(ctx, &self.iface_appended("Stub")?)?,
            Command::Proxy => self.get_class_file(ctx, &self.iface_appended("Stub$Proxy")?)?,
            Command::Interface => self.get_class_file(ctx, self.get_iface()?)?,
        };
        if !self.open {
            let class_file = try_proj_home_relative(ctx, &class_file);
            let class_file = class_file.to_str().expect("valid paths");
            println!("{}", class_file);
            return Ok(());
        }
        let class_file = class_file.to_str().expect("valid paths");
        exec_open_file(ctx, class_file)?;
        Ok(())
    }

    fn find_impl(&self, ctx: &dyn Context) -> anyhow::Result<PathBuf> {
        let db = DeviceDatabase::new(ctx)?;
        let impls = db.get_system_service_impls(self.service.id)?;

        let imp = if impls.len() == 0 {
            bail!("no impls found for {}", self.service.name);
        } else if impls.len() == 1 {
            impls.get(0).unwrap().clone()
        } else {
            prompt_choice(
                &impls,
                &format!("Multiple implementations found for {}", self.service.name),
                "Choice: ",
            )?
            .clone()
        };

        let apk = if imp.is_from_framework() {
            None
        } else {
            Some(imp.apk_path())
        };

        Ok(
            find_smali_file_for_class(&ctx, &imp.class_name, apk.as_ref()).ok_or_else(|| {
                anyhow::Error::msg(format!("failed to find smali file for {}", imp.class_name))
            })?,
        )
    }

    fn iface_appended(&self, name: &str) -> anyhow::Result<ClassName> {
        self.get_iface().map(|it| {
            let simple_name = it.get_simple_class_name();
            let new_name = format!("{}${}", simple_name, name);
            it.with_new_simple_class_name(&new_name)
        })
    }

    fn get_class_file(&self, ctx: &dyn Context, class: &ClassName) -> anyhow::Result<PathBuf> {
        let files = find_files_for_class(ctx, class)
            .iter()
            .map(|it| it.to_str().expect("sane paths").to_string())
            .collect::<Vec<String>>();
        Ok(vec_to_single(
            &files,
            &format!("Multiple implementations found for {}", class),
            "Choice: ",
        )
        .map(|it| PathBuf::from(it))?)
    }

    fn get_iface(&self) -> anyhow::Result<&ClassName> {
        match &self.service.iface {
            None => bail!("no interface for {}", self.service.name),
            Some(v) => Ok(v),
        }
    }
}
