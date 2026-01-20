use anyhow::bail;
use std::borrow::Cow;

use clap::{self, Args};
use dtu::app::{
    render_into, AppTestStatus, TemplateRenderer, TestGeneric, TestProvider, TestServiceRaw,
    TestSystemServiceRaw,
};
use lazy_static::lazy_static;
use regex::Regex;

use crate::utils::{exec_open_file, prompt_choice};
use dtu::askama::DynTemplate;
use dtu::db;
use dtu::db::device::models::{SystemService, SystemServiceMethod};
use dtu::db::meta::db::APP_PKG_KEY;
use dtu::db::meta::models::InsertAppActivity;
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase};
use dtu::utils::ClassName;
use dtu::Context;

lazy_static! {
    static ref VALID_SIMPLE_CLASS: Regex = Regex::new("^[A-Z][a-zA-Z]+$").expect("invalid regex");
}

#[derive(Args)]
pub struct ServiceFile {
    /// The interface for the service AIDL
    #[arg(short = 'I', long)]
    iface: ClassName,

    /// The service class name as defined in the Manifest
    #[arg(short, long)]
    class: ClassName,

    /// The optional package if it differs from the class package
    #[arg(short = 'P', long)]
    pkg: Option<String>,

    /// The raw transaction id
    #[arg(long)]
    txn_id: i32,

    /// Button text, if not provided the class name will be used
    #[arg(short = 'T', long)]
    button_text: Option<String>,

    /// The name of the class to generate
    ///
    /// If this isn't set, the class and transaction id will be used
    #[arg(short, long)]
    name: Option<String>,

    /// Open the new file in your default $EDITOR
    #[arg(long)]
    open: bool,
}

impl ServiceFile {
    pub fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let raw_name = self.get_class_name();
        let class_name = ensure_valid_name(raw_name.as_ref());

        ensure_class_available(meta, class_name.as_ref())?;

        let service_pkg = self.pkg.as_ref().map(|it| it.as_str());
        let pkg = meta.get_key_value(APP_PKG_KEY)?;

        let template = TestServiceRaw {
            app_pkg: &pkg,
            class: class_name.as_ref(),
            txn_number: self.txn_id,
            service_class: &self.class,
            service_pkg,
            iface: &self.iface,
        };

        add_template_activity(
            ctx,
            meta,
            &template,
            class_name.as_ref(),
            &self.button_text,
            self.open,
        )
    }

    fn get_class_name(&self) -> Cow<'_, str> {
        if let Some(name) = self.name.as_ref() {
            return Cow::Borrowed(name.as_str());
        }

        let name = format!(
            "Test{}Transaction{}",
            self.class.get_simple_class_name(),
            self.txn_id
        );
        Cow::Owned(name)
    }
}

#[derive(Args)]
pub struct GenericFile {
    /// Button text, if not provided the class name will be used
    #[arg(short = 'T', long)]
    button_text: Option<String>,

    /// The name of the test
    ///
    /// "Test" will be appended to the name, making the full name "Test{name}"
    #[arg(short, long)]
    name: String,

    /// Open the new file in your default $EDITOR
    #[arg(long)]
    open: bool,
}

impl GenericFile {
    pub fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let class_name = format!("Test{}", ensure_valid_name(&self.name));
        ensure_class_available(meta, &class_name)?;

        let pkg = meta.get_key_value(APP_PKG_KEY)?;

        let template = TestGeneric {
            app_pkg: &pkg,
            class: class_name.as_str(),
        };

        add_template_activity(
            ctx,
            meta,
            &template,
            &class_name,
            &self.button_text,
            self.open,
        )
    }
}

#[derive(Args)]
pub struct ProviderFile {
    /// The provider authority
    #[arg(short, long)]
    authority: String,

    /// Button text, if not provided the class name will be used
    #[arg(short = 'T', long)]
    button_text: Option<String>,

    /// The name of the test
    ///
    /// "Test" will be appended to the name, making the full name "Test{name}"
    #[arg(short, long)]
    name: String,

    /// Open the new file in your default $EDITOR
    #[arg(long)]
    open: bool,
}

impl ProviderFile {
    pub fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let class_name = format!("Test{}", ensure_valid_name(&self.name));
        ensure_class_available(meta, &class_name)?;

        let db = DeviceSqliteDatabase::new(ctx)?;
        match db.get_provider_containing_authority(&self.authority) {
            Err(db::Error::NotFound) => bail!("invalid authority {}", self.authority),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }

        let pkg = meta.get_key_value(APP_PKG_KEY)?;

        let template = TestProvider {
            app_pkg: &pkg,
            class: class_name.as_str(),
            authority: self.authority.as_str(),
        };

        add_template_activity(
            ctx,
            meta,
            &template,
            &class_name,
            &self.button_text,
            self.open,
        )
    }
}

#[derive(Args)]
pub struct SystemServiceFile {
    /// The system service to create a test file for
    #[arg(short, long)]
    service: String,

    /// An optional interface for the system service.
    ///
    /// Only required if for some reason the passed service is not in the
    /// database or no interface was discovered for the service
    #[arg(short = 'I', long)]
    interface: Option<ClassName>,

    /// The system service method name, if None, --txn-id must be supplied
    #[arg(short, long)]
    method: Option<String>,

    /// The raw transaction id
    #[arg(long)]
    txn_id: Option<i32>,

    /// Button text, if not provided the class name will be used
    #[arg(short = 'T', long)]
    button_text: Option<String>,

    /// The name of the class to generate
    ///
    /// This will default to some combination of the method and class
    #[arg(short, long)]
    name: Option<String>,

    /// Open the new file in your default $EDITOR
    #[arg(long)]
    open: bool,
}

impl SystemServiceFile {
    pub fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let db = DeviceSqliteDatabase::new(ctx)?;

        let service = match db.get_system_service_by_name(&self.service) {
            Ok(v) => Some(v),
            Err(db::Error::NotFound) if self.interface.is_some() => None,
            Err(e) => return Err(e.into()),
        };

        if service.is_none() && self.txn_id.is_none() {
            bail!(
                "the service `{}` is not in the database, must provide --txn-id",
                self.service
            );
        }

        if self.method.is_none() && self.txn_id.is_none() {
            bail!("need either -m/--method or --txn-id");
        }

        let class_name = self.get_class_name();
        ensure_class_available(meta, class_name.as_ref())?;
        let db = DeviceSqliteDatabase::new(ctx)?;

        let iface = match &self.interface {
            Some(v) => v,
            None => match service.as_ref().unwrap().iface.as_ref() {
                Some(v) => v,
                None => bail!(
                    "no known interface for service {}, use -I if one is known",
                    self.service
                ),
            },
        };

        let txn_number = self.get_transaction_id(&db, &service)?;
        let method_name = self.get_method_name();
        let pkg = meta.get_key_value(APP_PKG_KEY)?;

        let template = TestSystemServiceRaw {
            app_pkg: &pkg,
            class: class_name.as_ref(),
            txn_number,
            service: &self.service,
            method: method_name.as_ref(),
            iface,
        };

        add_template_activity(
            ctx,
            meta,
            &template,
            class_name.as_ref(),
            &self.button_text,
            self.open,
        )
    }

    fn get_displayable_service_name(&self) -> Cow<'_, str> {
        ensure_valid_name(&self.service)
    }

    fn get_method_name(&self) -> Cow<'_, str> {
        if let Some(method) = self.method.as_ref() {
            Cow::Borrowed(method.as_str())
        } else {
            Cow::Owned(format!("TransactionNumber{}", self.txn_id.unwrap()))
        }
    }

    fn get_transaction_id(
        &self,
        db: &DeviceSqliteDatabase,
        service: &Option<SystemService>,
    ) -> anyhow::Result<i32> {
        if let Some(id) = self.txn_id {
            if id < 1 {
                bail!("invalid transaction id {}", id);
            }
            return Ok(id);
        }

        let svc = match service.as_ref() {
            None => bail!(
                "service {} not found in database and --txn-id not set",
                self.service
            ),
            Some(v) => v,
        };

        let name = self.method.as_ref().unwrap().as_str();

        let method = db.get_system_service_methods_by_service_id(svc.id)?;
        let methods = method
            .iter()
            .filter(|it| it.name == name)
            .collect::<Vec<&SystemServiceMethod>>();
        let count = methods.len();
        if count == 0 {
            bail!("no method named {} found on service {}", name, self.service);
        } else if count == 1 {
            return Ok(methods.get(0).unwrap().transaction_id);
        }

        Ok(prompt_choice(
            &methods,
            &format!("Multiple methods named {} found for {}", name, self.service),
            "Choice: ",
        )?
        .transaction_id)
    }

    fn get_class_name(&self) -> Cow<'_, str> {
        if let Some(name) = &self.name {
            return ensure_valid_name(&name);
        }

        let method_identifier = self
            .method
            .as_ref()
            .map(|it| ensure_valid_name(it))
            .unwrap_or_else(|| Cow::Owned(format!("Transaction{}", self.txn_id.unwrap())));

        let mut new_name = self.get_displayable_service_name().to_string();
        new_name.push_str(method_identifier.as_ref());
        Cow::Owned(new_name)
    }
}

fn add_template_activity(
    ctx: &dyn Context,
    meta: &impl MetaDatabase,
    template: &dyn DynTemplate,
    class_name: &str,
    button_text: &Option<String>,
    open: bool,
) -> anyhow::Result<()> {
    let name = format!("app/src/main/kotlin/c/arve/{}.kt", class_name);
    render_into(&ctx, class_name, name.as_str(), template)?;

    let button_text = button_text
        .as_ref()
        .map(|it| Cow::Borrowed(it.as_str()))
        .unwrap_or_else(|| Cow::Borrowed(class_name));
    let button_android_id = format!("btn{}", class_name);

    let new_act = InsertAppActivity {
        name: class_name,
        button_text: button_text.as_ref(),
        button_android_id: button_android_id.as_str(),
        status: AppTestStatus::Experimenting,
    };
    meta.add_app_activity(&new_act)?;
    let pkg = meta.get_key_value(APP_PKG_KEY)?;
    let template = TemplateRenderer::new(ctx, meta, &pkg);
    template.update()?;

    if !open {
        return Ok(());
    }

    let full_path = ctx.get_test_app_dir()?.join(name);
    let full_path_string = full_path.to_str().expect("valid paths");
    exec_open_file(&ctx, full_path_string)?;

    Ok(())
}

fn ensure_class_available(meta: &dyn MetaDatabase, class_name: &str) -> anyhow::Result<()> {
    if meta.app_activity_name_taken(class_name)? {
        bail!("test name {} already taken", class_name);
    }
    Ok(())
}

fn ensure_valid_name(name: &str) -> Cow<'_, str> {
    if VALID_SIMPLE_CLASS.is_match(name) {
        return Cow::Borrowed(name);
    }

    let mut new_name = String::with_capacity(name.len());

    let mut should_upper = false;

    let mut chars = name.chars();

    let first = chars.next().unwrap();
    new_name.push(first.to_ascii_uppercase());

    for c in chars {
        match c {
            '.' | '_' => should_upper = true,
            'a'..='z' | 'A'..='Z' => {
                if should_upper {
                    new_name.push(c.to_ascii_uppercase());
                    should_upper = false;
                } else {
                    new_name.push(c);
                }
            }
            _ => {}
        }
    }

    Cow::Owned(new_name)
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_ensure_valid_name() {
        let valid = "AzAZname";
        assert_eq!(ensure_valid_name(valid), Cow::Borrowed(valid));
        let name = "snake_case_name";
        assert_eq!(ensure_valid_name(name).as_ref(), "SnakeCaseName");
        let name = "dotted.name";
        assert_eq!(ensure_valid_name(name).as_ref(), "DottedName");
        let name = "dotted.an!!d_s#nake_cas@e.name21";
        assert_eq!(ensure_valid_name(name).as_ref(), "DottedAndSnakeCaseName");
    }
}
