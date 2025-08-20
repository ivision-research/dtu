use base64::Engine;
use clap::{self, Args};
use dtu::app::IntentString;
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;

use crate::parsers::{parse_intent_string, parse_parcel_string};
use crate::utils::get_app_server;
use dtu::app::server::{AppServer, ProviderUriBuilder};
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Args)]
pub struct Provider {
    /// The target ContentProvider's authority string
    #[arg(short, long)]
    authority: String,

    #[command(subcommand)]
    command: Subcommand,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    /// Perform a query on a `ContentProvider`
    #[command()]
    Query(Query),

    /// Call an arbitrary method on a `ContentProvider`
    #[command()]
    Call(Call),

    /// Perform an insert on a `ContentProvider`
    #[command()]
    Insert(Insert),

    /// Perform a delete on a `ContentProvider`
    #[command()]
    Delete(Delete),

    /// Read a file from a `ContentProvider`
    #[command()]
    ReadFile(ReadFile),

    /// Write a file on a `ContentProvider`
    #[command()]
    WriteFile(WriteFile),
}

impl Provider {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::AppSetup)?;
        match &self.command {
            Subcommand::Call(c) => c.run(&self.authority)?,
            Subcommand::Query(c) => c.run(&self.authority)?,
            Subcommand::Insert(c) => c.run(&self.authority)?,
            Subcommand::Delete(c) => c.run(&self.authority)?,
            Subcommand::ReadFile(c) => c.run(&self.authority)?,
            Subcommand::WriteFile(c) => c.run(&self.authority)?,
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct Delete {
    /// An optional path for the content URI
    #[arg(short, long)]
    path: Option<String>,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,

    /// Optional where for the query
    #[arg(short = 'S', long)]
    where_clause: Option<String>,

    /// Optional selection args for the query
    #[arg(short = 'A', long)]
    selection_args: Option<Vec<String>>,
}
impl Delete {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        if let Some(p) = &self.path {
            uri_builder.with_path(p);
        }
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }

        let uri = uri_builder.build();
        println!(
            "Deleted {} rows",
            srv.provider_delete(
                &uri,
                self.where_clause.as_ref().map(|it| it.as_str()),
                self.selection_args.as_ref().map(|it| it.as_slice())
            )?
        );
        Ok(())
    }
}

#[derive(Args)]
pub struct Insert {
    /// An optional path for the content URI
    #[arg(short, long)]
    path: Option<String>,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,

    /// ContentValue parameters, same format as an intent string
    #[arg(last = true)]
    content_values: Vec<String>,
}
impl Insert {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        if let Some(p) = &self.path {
            uri_builder.with_path(p);
        }
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }

        let data = if self.content_values.is_empty() {
            IntentString::default()
        } else {
            parse_intent_string(self.content_values.as_slice())?
        };

        let uri = uri_builder.build();
        println!(
            "{}",
            srv.provider_insert(&uri, &data)?
                .as_ref()
                .map(|it| it.as_str())
                .unwrap_or("null")
        );
        Ok(())
    }
}

#[derive(Args)]
pub struct Query {
    /// An optional path for the content URI
    #[arg(short, long)]
    path: Option<String>,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,

    /// Optional selection for the query
    #[arg(short = 'S', long)]
    selection: Option<String>,

    /// Optional selection args for the query
    #[arg(short = 'A', long)]
    selection_args: Option<Vec<String>>,

    /// Optional sort order for the query
    #[arg(short = 'o', long)]
    sort_order: Option<String>,

    /// Optional projection
    #[arg(short = 'p', long)]
    projection: Option<Vec<String>>,

    /// Bundle definition for query args. In the form of a Parcel string's
    /// `bund` type.
    #[arg(short = 'Q', long, last = true)]
    query_args: Option<Vec<String>>,
}

impl Query {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let qa = match &self.query_args {
            None => None,
            Some(it) => Some(parse_parcel_string(it.as_slice())?),
        };
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        if let Some(p) = &self.path {
            uri_builder.with_path(p);
        }
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }
        let uri = uri_builder.build();
        let res = srv.provider_query(
            &uri,
            self.projection.as_ref().map(|it| it.as_slice()),
            self.selection.as_ref().map(|it| it.as_str()),
            self.selection_args.as_ref().map(|it| it.as_slice()),
            qa.as_ref(),
            self.sort_order.as_ref().map(|it| it.as_str()),
        )?;
        println!("{}", res);
        Ok(())
    }
}

#[derive(Args)]
pub struct Call {
    /// The method name to call
    #[arg(short, long)]
    method: String,

    /// Arguments to the method
    #[arg(short, long)]
    args: Option<String>,

    /// An optional path for the content URI
    #[arg(short, long)]
    path: Option<String>,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,
}

impl Call {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        if let Some(p) = &self.path {
            uri_builder.with_path(p);
        }
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }
        let uri = uri_builder.build();
        let res =
            srv.provider_call(&uri, &self.method, self.args.as_ref().map(|it| it.as_str()))?;
        println!("{}", res);
        Ok(())
    }
}

#[derive(Args)]
pub struct ReadFile {
    /// The file path to read
    #[arg(short, long)]
    file: String,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,

    /// Decode the base64 returned from the server
    #[arg(short, long, default_value_t = false)]
    decode: bool,

    /// Write the decoded output to the given file
    #[arg(short, long)]
    out_file: Option<PathBuf>,
}

impl ReadFile {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        uri_builder.with_path(&self.file);
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }
        let uri = uri_builder.build();
        let res = srv.provider_read(&uri)?;

        if let Some(of) = &self.out_file {
            match res.data_raw() {
                Some(v) => fs::write(&of, &v)?,
                None => fs::write(&of, &[])?,
            }
        } else if self.decode {
            if let Some(data) = res.data_utf8() {
                println!("{}", data);
            }
        } else {
            println!("{}", res.data_b64());
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct WriteFile {
    /// The file path to read
    #[arg(short, long)]
    file: String,

    /// The data to write, can be base64 encoded
    #[arg(short, long)]
    data: String,

    /// Set if the data is base64 encoded
    #[arg(long)]
    b64: bool,

    /// Optional query string for the content URI
    #[arg(short, long)]
    query: Option<String>,
}

impl WriteFile {
    fn run(&self, authority: &str) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mut srv = get_app_server(&ctx)?;
        let mut uri_builder = ProviderUriBuilder::new(authority);
        uri_builder.with_path(&self.file);
        if let Some(q) = &self.query {
            uri_builder.with_query(q);
        }
        let data = if !self.b64 {
            let engine = base64::engine::general_purpose::STANDARD;
            Cow::Owned(engine.decode(&self.data)?)
        } else {
            Cow::Borrowed(self.data.as_bytes())
        };
        let uri = uri_builder.build();
        let res = srv.provider_write(&uri, data.as_ref())?;
        println!("{}", res);
        Ok(())
    }
}
