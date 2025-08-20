use clap::{self, Args, Subcommand};

mod service_file;
use service_file::ServiceFile;

mod permission;
use permission::Permission;

mod smali_file;
use smali_file::SmaliFile;

mod interface_impl;
use interface_impl::InterfaceImpl;

mod children;
use children::Children;

#[derive(Args)]
pub struct Find {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find service related smali files
    #[command()]
    ServiceFile(ServiceFile),

    /// Find a permission
    #[command()]
    Permission(Permission),

    /// Find a smali file
    #[command()]
    SmaliFile(SmaliFile),

    /// Find interface implementations
    #[command()]
    InterfaceImpl(InterfaceImpl),

    /// Find child classes
    #[command()]
    Children(Children),
}

impl Find {
    pub fn run(&self) -> anyhow::Result<()> {
        match &self.command {
            Command::ServiceFile(c) => c.run(),
            Command::Permission(c) => c.run(),
            Command::SmaliFile(c) => c.run(),
            Command::InterfaceImpl(c) => c.run(),
            Command::Children(c) => c.run(),
        }
    }
}
