use anyhow::bail;
use clap::{self, Args};
use dtu::{
    utils::{path_must_str, DevicePath},
    Context,
};

use crate::{parsers::DevicePathValueParser, utils::exec_open_file};

#[derive(Args)]
pub struct FindManifest {
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: DevicePath,

    /// Open the file in $EDITOR
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    open: bool,
}

impl FindManifest {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let path = ctx
            .get_apks_dir()?
            .join("decompiled")
            .join(&self.apk)
            .join("AndroidManifest.xml");

        let as_str = path_must_str(&path);

        if !path.exists() {
            bail!("failed to find the manifest at {}", as_str);
        }

        if self.open {
            exec_open_file(ctx, as_str)?;
        } else {
            println!("{as_str}");
        }

        Ok(())
    }
}
