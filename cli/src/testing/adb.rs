use std::io;
use std::path::PathBuf;
use std::process::{Child, ExitStatus};

use crossbeam::channel::Receiver;

use mockall::mock;
use rstest::fixture;

use dtu;
use dtu::command::CmdOutput;

#[fixture]
pub fn mock_adb() -> MockAdb {
    MockAdb::new()
}

mock! {
    pub Adb {

    }

    impl dtu::adb::Adb for Adb {
        fn install(&self, apk: &str) -> dtu::Result<()>;
        fn uninstall(&self, package: &str) -> dtu::Result<()>;
        fn forward_tcp_port(&self, local: u16, remote: u16) -> io::Result<CmdOutput>;
        fn backup(&self, path: &PathBuf) -> io::Result<()>;
        fn reverse_tcp_port(&self, local: u16, remote: u16) -> io::Result<CmdOutput>;
        fn get_connected_devices(&self) -> dtu::Result<Vec<String>>;
        fn forward_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput>;
        fn reverse_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput>;
        fn pull(&self, device: &str, local: &str) -> io::Result<CmdOutput>;
        fn spawn_pull(&self, device: &str, local: &str) -> io::Result<Child>;
        fn push(&self, local: &str, device: &str) -> io::Result<CmdOutput>;
        fn spawn_push(&self, local: &str, device: &str) -> io::Result<Child>;
        fn shell(&self, shell_cmd: &str) -> io::Result<CmdOutput>;
        fn shell_streamed(
            &self,
            shell_cmd: &str,
            on_stdout: &mut dyn for<'a> FnMut(&'a [u8]) -> Result<(), anyhow::Error>,
            on_stderr: &mut dyn for<'a> FnMut(&'a [u8]) -> Result<(), anyhow::Error>,
            kill_child: Option<Receiver<()>>,
        ) -> io::Result<ExitStatus>;

        fn shell_split_streamed(
            &self,
            shell_cmd: &str,
            split_on: u8,
            on_stdout_line: &mut dyn for<'a> FnMut(&'a str) -> Result<(), anyhow::Error>,
            on_stderr_line: &mut dyn for<'a> FnMut(&'a str) -> Result<(), anyhow::Error>
        ) -> Result<ExitStatus, io::Error>;
    }
}
