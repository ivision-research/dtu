use clap::{self, Args};
use dtu::utils::{ClassName, DevicePath};
use dtu::DefaultContext;

use crate::parsers::DevicePathValueParser;
use crate::utils::find_smali_file;

#[derive(Args)]
pub struct SmaliFile {
    /// Set the APK that the file belongs to, otherwise the framework is assumed
    ///
    /// Note that if this is not set and no file is found, this command will
    /// attempt to find the file in APK directories unless `--no-fallback` is set
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Don't fallback to searching APK paths if `--apk` is not set
    #[arg(long)]
    no_fallback: bool,

    /// The class name (smali or Java) of the file to open
    #[arg(short, long)]
    class: ClassName,
}

impl SmaliFile {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let fname = find_smali_file(&ctx, &self.class, &self.apk, !self.no_fallback)?;
        println!("{}", fname);
        Ok(())
    }
}
