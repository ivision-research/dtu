use std::borrow::Cow;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;

use anyhow::bail;
use clap::{self, Args};
use dtu::adb::{Adb, ExecAdb};
use dtu::command::quote;
use dtu::{Context, DefaultContext};
use promptly::{prompt, prompt_default};

#[cfg_attr(debug_assertions, derive(Default))]
/// Generate an .envrc file that defines all relevant environmental variables
/// for other commands.
#[derive(Args)]
pub struct GenEnvrc {
    /// Prompt before writing the .envrc file or on multiple devices
    #[arg(
        short = 'P',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    prompt: bool,

    /// Sets the project home - defaults to the current working directory
    #[arg(short, long)]
    project_home: Option<String>,

    /// Sets the Android device serial for adb, will be found programmatically
    /// if not set.
    #[arg(short, long)]
    android_serial: Option<String>,
}

struct EnvrcWriter<'a> {
    project_home: Cow<'a, str>,
    android_serial: Cow<'a, str>,
}

impl<'a> EnvrcWriter<'a> {
    fn write_envrc<W: Write>(&self, to: &mut W) -> anyhow::Result<()> {
        write!(
            to,
            "export DTU_PROJECT_HOME={}\n",
            quote(self.project_home.as_ref())
        )?;

        write!(
            to,
            "export ANDROID_SERIAL={}\n",
            quote(self.android_serial.as_ref())
        )?;

        Ok(())
    }
}

impl GenEnvrc {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let adb = ExecAdb::builder(&ctx).build();
        let project_home = self.get_project_home(&ctx)?;
        let android_serial = self.get_device_serial(&ctx, &adb)?;

        let w = EnvrcWriter {
            project_home,
            android_serial,
        };
        if self.prompt {
            if !self.prompt_accepted(&w) {
                eprintln!("not writing .envrc");
                return Ok(());
            }
        }

        let mut f = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(".envrc")?;

        w.write_envrc(&mut f)
    }

    fn prompt_accepted(&self, w: &EnvrcWriter) -> bool {
        let mut vec = Vec::new();
        w.write_envrc(&mut vec).expect("write to vec");

        let as_string = String::from_utf8_lossy(vec.as_slice());

        println!(
            "Will write:\n\n{}\n{}{}\n",
            "-".repeat(10),
            as_string,
            "-".repeat(10)
        );
        let res: bool = match prompt_default("Ok?", true) {
            Ok(v) => v,
            Err(_) => false,
        };
        res
    }

    fn get_project_home(&self, ctx: &dyn Context) -> anyhow::Result<Cow<str>> {
        if let Some(ph) = self.project_home.as_ref() {
            return Ok(Cow::Borrowed(ph.as_str()));
        }

        if let Some(d) = ctx.maybe_get_env("DTU_PROJECT_HOME") {
            return Ok(Cow::Owned(d));
        }

        let cwd = env::current_dir()?;
        Ok(Cow::Owned(cwd.to_string_lossy().to_string()))
    }

    fn get_device_serial(&self, ctx: &dyn Context, adb: &dyn Adb) -> anyhow::Result<Cow<str>> {
        if let Some(ser) = self.android_serial.as_ref() {
            return Ok(Cow::Borrowed(ser.as_str()));
        }

        if let Some(env) = ctx.maybe_get_env("ANDROID_SERIAL") {
            return Ok(Cow::Owned(env));
        }

        let devices = adb.get_connected_devices()?;
        let count = devices.len();

        if count == 1 {
            return Ok(Cow::Owned(devices.get(0).unwrap().clone()));
        }

        if !self.prompt {
            bail!("multiple adb devices and no -P/--prompt");
        }

        println!("Multiple ADB devices found, please select one:");

        for (i, d) in devices.iter().enumerate() {
            println!("({}) {}", i, d);
        }

        loop {
            let sel: usize = match prompt("Device number: ") {
                Ok(ans) => ans,
                Err(_) => bail!("failed to set android device"),
            };

            if sel >= count {
                eprintln!("invalid sel {}", sel);
                continue;
            }

            return Ok(Cow::Owned(devices.get(sel).unwrap().clone()));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;

    use crate::testing::{mock_adb, mock_context, MockAdb, MockContext};

    #[rstest]
    fn test_get_device_serial_from_adb(mut mock_context: MockContext, mut mock_adb: MockAdb) {
        mock_context.expect_maybe_get_env().returning(|_| None);
        mock_adb
            .expect_get_connected_devices()
            .returning(|| Ok(vec![String::from("test")]));

        let genv: GenEnvrc = Default::default();

        assert_eq!(
            genv.get_device_serial(&mock_context, &mock_adb)
                .expect("failed to get project home"),
            Cow::<str>::Owned(String::from("test"))
        );
    }

    #[rstest]
    fn test_get_device_serial_from_env(mut mock_context: MockContext, mock_adb: MockAdb) {
        mock_context
            .expect_maybe_get_env()
            .returning(|_| Some(String::from("test")));
        let genv: GenEnvrc = Default::default();

        assert_eq!(
            genv.get_device_serial(&mock_context, &mock_adb)
                .expect("failed to get project home"),
            Cow::<str>::Owned(String::from("test"))
        );
    }

    #[rstest]
    fn test_get_project_from_env(mut mock_context: MockContext) {
        mock_context
            .expect_maybe_get_env()
            .returning(|_| Some(String::from("test")));
        let genv: GenEnvrc = Default::default();

        assert_eq!(
            genv.get_project_home(&mock_context)
                .expect("failed to get project home"),
            Cow::<str>::Owned(String::from("test"))
        );
    }
}
