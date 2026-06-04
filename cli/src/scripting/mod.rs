use std::borrow::Cow;
use std::fs::create_dir_all;
use std::io;
use std::path::{Component, Components, PathBuf};

use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::utils::{path_must_str, proj_home_relative, ClassName, DevicePath};
use dtu::{Context, DefaultContext};

use crate::parsers::DevicePathValueParser;

#[derive(Args)]
pub struct Scripting {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(alias = "spm")]
    /// Extract metadata from a path to a smali file and print to stdout
    SmaliPathMeta(SmaliPathMeta),

    /// Attempt to extract DTU_PROJECT_HOME from a path based on heuristics
    ExtractProjectHome(ExtractProjectHome),

    /// Retrieve a valid graph source for an APK
    #[command()]
    ApkSource(ApkSource),

    /// Unsquash a squashed path
    #[command()]
    Unsquash(Unsquash),

    /// Squash a device path
    #[command()]
    Squash(Squash),

    /// Ensure a class is in Java form
    #[command()]
    JavaClass(JavaClass),

    /// Ensure a class is in smali form
    #[command()]
    SmaliClass(SmaliClass),

    /// Get a cache dir in the dtu project
    #[command()]
    CacheDir(CacheDir),
}

impl Scripting {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::SmaliPathMeta(c) => c.run(),
            Command::Squash(c) => c.run(),
            Command::Unsquash(c) => c.run(),
            Command::SmaliClass(c) => c.run(),
            Command::JavaClass(c) => c.run(),
            Command::CacheDir(c) => c.run(),
            Command::ExtractProjectHome(c) => c.run(),
            Command::ApkSource(c) => c.run(),
        }
    }
}

#[derive(Args)]
struct ApkSource {
    #[arg(value_parser = DevicePathValueParser)]
    path: DevicePath,
}

impl ApkSource {
    fn run(self) -> anyhow::Result<()> {
        println!("{}", self.path.as_squashed_str());
        return Ok(());
    }
}

#[derive(Args)]
struct ExtractProjectHome {
    #[arg()]
    path: String,
}

impl ExtractProjectHome {
    fn run(self) -> anyhow::Result<()> {
        if let Some((pre, _)) = self.path.split_once("dtu_out") {
            println!("{pre}");
            return Ok(());
        }

        let pb = PathBuf::from(&self.path);
        // First check if we have a directory and dtu_out is in it before bailing
        if !pb.is_dir() || !pb.join("dtu_out").exists() {
            bail!("can't deduce DTU_PROJECT_HOME from {}", self.path);
        }
        println!("{}", self.path);
        return Ok(());
    }
}

#[derive(Args)]
struct CacheDir {
    #[arg()]
    owner: String,
}

impl CacheDir {
    fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let cache = ctx.get_project_cache_dir()?;
        let path = cache.join(self.owner);
        if !path.exists() {
            create_dir_all(&path)?;
        }
        println!("{}", path_must_str(&path));
        Ok(())
    }
}

#[derive(Args)]
struct SmaliClass {
    #[arg()]
    class: String,
}

impl SmaliClass {
    fn run(self) -> anyhow::Result<()> {
        let class = ClassName::new(self.class);
        println!("{}", class.get_smali_name());
        Ok(())
    }
}

#[derive(Args)]
struct JavaClass {
    #[arg()]
    class: String,
}

impl JavaClass {
    fn run(self) -> anyhow::Result<()> {
        let class = ClassName::new(self.class);
        println!("{}", class.get_java_name());
        Ok(())
    }
}

#[derive(Args)]
struct Squash {
    #[arg()]
    path: String,
}

impl Squash {
    fn run(self) -> anyhow::Result<()> {
        let squashed = DevicePath::new(self.path).get_squashed_string();
        println!("{squashed}");
        Ok(())
    }
}

#[derive(Args)]
struct Unsquash {
    #[arg()]
    path: String,
}

impl Unsquash {
    fn run(self) -> anyhow::Result<()> {
        let unsquashed = DevicePath::from_squashed(self.path).get_device_string();
        println!("{unsquashed}");
        Ok(())
    }
}

#[derive(Args)]
struct SmaliPathMeta {
    #[arg()]
    path: PathBuf,

    #[arg(short, long)]
    json: bool,
}

impl SmaliPathMeta {
    fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();

        match self.path.extension() {
            Some(x) if x == "smali" => {}
            _ => return Err(self.invalid_path("not a smali file")),
        }

        let Some(rel) = proj_home_relative(&ctx, &self.path) else {
            bail!(
                "can't make {} relative to the project home",
                path_must_str(&self.path)
            );
        };

        let rel = rel.with_extension("");

        let mut parts = rel.components();

        self.must_component_match(&mut parts, "dtu_out")?;
        self.must_component_match(&mut parts, "smali")?;

        let next = self.must_component(&mut parts)?;

        let mut apk_name: Option<String> = None;

        let source = if next == "framework" {
            Cow::Borrowed("framework")
        } else if next == "apks" {
            let apk = self.must_component(&mut parts)?;
            let p = DevicePath::from_squashed(apk);
            apk_name = Some(String::from(p.device_file_name()));
            Cow::Owned(String::from(apk))
        } else {
            return Err(self.invalid_path("not in smali/framework or smali/apks"));
        };

        let mut class_parts = Vec::new();

        for part in parts {
            let Component::Normal(x) = part else {
                return Err(self.invalid_path("invalid component type"));
            };

            let Some(x) = x.to_str() else {
                return Err(self.invalid_path("non-utf8"));
            };
            class_parts.push(x);
        }

        let class = format!("L{};", class_parts.join("/"));

        #[derive(serde::Serialize)]
        struct JsonOutput<'a> {
            source: &'a str,
            class: String,
            apk: Option<String>,
        }

        if self.json {
            serde_json::to_writer(
                io::stdout(),
                &JsonOutput {
                    source: &source,
                    class,
                    apk: apk_name,
                },
            )?;
            return Ok(());
        }

        println!("{source}\n{class}");
        match &apk_name {
            Some(v) => println!("{v}"),
            None => print!("\n"),
        }
        Ok(())
    }

    fn invalid_path(&self, reason: &str) -> anyhow::Error {
        anyhow::Error::msg(format!(
            "path {} is not a valid smali path: {reason}",
            path_must_str(&self.path)
        ))
    }

    fn must_component<'a>(&self, it: &mut Components<'a>) -> anyhow::Result<&'a str> {
        match it.next() {
            Some(Component::Normal(x)) => match x.to_str() {
                None => Err(self.invalid_path("non-utf8 name")),
                Some(v) => Ok(v),
            },
            _ => Err(self.invalid_path("no more components")),
        }
    }

    fn must_component_match(&self, it: &mut Components, name: &str) -> anyhow::Result<()> {
        match it.next() {
            Some(Component::Normal(x)) if x == name => Ok(()),
            _ => Err(self.invalid_path(&format!("component does not match {}", name))),
        }
    }
}
