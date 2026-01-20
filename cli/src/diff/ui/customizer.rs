use std::collections::HashSet;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::Deref;

use ratatui::style::Style;
use ratatui::widgets::{Paragraph, Widget};

use dtu::db::device::models::{DiffedProvider, DiffedSystemServiceMethod};
use dtu::db::{ApkIPC, ApkIPCKind, DeviceDatabase, DeviceSqliteDatabase};

use crate::diff::{smali_sig_contains_class, smali_sig_looks_like_binder};
use crate::ui::widgets::{
    BlockBuilder, ClosureWidget, BG_COLOR, FG_COLOR, INTERESTING_COLOR, PURPLE,
};
use crate::utils::{find_fully_qualified_apk, invoke_dtu_clipboard, invoke_dtu_open_file};
use dtu::utils::{find_smali_file_for_class, path_must_str, ClassName};
use dtu::Context;

pub trait Customizer<E> {
    fn display(&self, item: &E) -> String;

    fn filter(&self, item: &E) -> bool {
        let _ = item;
        false
    }

    fn style(&self, item: &E) -> Option<Style> {
        let _ = item;
        None
    }

    fn get_popup(&self, item: &E) -> Option<ClosureWidget> {
        let _ = item;
        None
    }

    fn open_selection(&self, ctx: &dyn Context, item: &E) -> anyhow::Result<()> {
        let _ = item;
        let _ = ctx;
        anyhow::bail!("open not supported")
    }

    fn clipboard_selection(&self, ctx: &dyn Context, item: &E) -> anyhow::Result<()> {
        let _ = item;
        let _ = ctx;
        anyhow::bail!("clipbaord not supported")
    }
}

pub struct SystemServiceMethodCustomizer {
    db: DeviceSqliteDatabase,
    hidden_services: HashSet<i32>,
}

impl SystemServiceMethodCustomizer {
    pub fn new(db: DeviceSqliteDatabase, hidden_services: HashSet<i32>) -> Self {
        Self {
            db,
            hidden_services,
        }
    }
}

impl Customizer<DiffedSystemServiceMethod> for SystemServiceMethodCustomizer {
    fn display(&self, item: &DiffedSystemServiceMethod) -> String {
        item.to_string()
    }

    fn filter(&self, item: &DiffedSystemServiceMethod) -> bool {
        self.hidden_services.contains(&item.system_service_id)
    }

    fn style(&self, m: &DiffedSystemServiceMethod) -> Option<Style> {
        let sig = m.get_signature();
        let ret = m.get_return_type();
        if smali_sig_looks_like_binder(sig) || smali_sig_looks_like_binder(ret) {
            Some(Style::default().fg(PURPLE))
        } else if smali_sig_contains_class(sig) {
            Some(Style::default().fg(INTERESTING_COLOR))
        } else {
            None
        }
    }

    fn clipboard_selection(
        &self,
        ctx: &dyn Context,
        item: &DiffedSystemServiceMethod,
    ) -> anyhow::Result<()> {
        let db = DeviceSqliteDatabase::new(ctx)?;
        let service = db.get_system_service_by_id(item.method.system_service_id)?;

        let cmd = format!(
            "dtu call system-service -s '{}' -m '{}'",
            service.name, item.method.name
        );

        invoke_dtu_clipboard(ctx, &cmd)
    }

    fn open_selection(
        &self,
        ctx: &dyn Context,
        item: &DiffedSystemServiceMethod,
    ) -> anyhow::Result<()> {
        let impls = self.db.get_system_service_impls(item.system_service_id)?;

        let imp = if impls.len() == 0 {
            anyhow::bail!("no impls found for {}", item.name);
        } else if impls.len() == 1 {
            impls.get(0).unwrap().clone()
        } else {
            anyhow::bail!("multiple options available for {}", item.name);
        };

        let apk = if imp.is_from_framework() {
            None
        } else {
            Some(imp.apk_path())
        };

        let smali_file_path = find_smali_file_for_class(ctx, &imp.class_name, apk.as_ref())
            .ok_or_else(|| {
                anyhow::Error::msg(format!("failed to find smali file for {}", imp.class_name))
            })?;

        invoke_dtu_open_file(ctx, path_must_str(&smali_file_path), &item.name)?;
        Ok(())
    }

    fn get_popup(&self, item: &DiffedSystemServiceMethod) -> Option<ClosureWidget> {
        let system_service = self
            .db
            .get_system_service_by_id(item.system_service_id)
            .ok()?;
        let text = format!(
            "Service: {}\nTransaction ID: {}\nExists in diff: {}\nHash matches diff: {}\n",
            system_service.name, item.transaction_id, item.exists_in_diff, item.hash_matches_diff
        );
        let block = BlockBuilder::default()
            .with_style(Style::default().bg(FG_COLOR).fg(BG_COLOR))
            .with_text("Details")
            .build();
        let para = Paragraph::new(text).block(block);
        Some(ClosureWidget::new(Box::new(move |area, buf| {
            para.render(area, buf);
        })))
    }
}

pub struct ApkIPCCustomizer<U> {
    db: DeviceSqliteDatabase,
    hidden_apks: HashSet<i32>,

    marker: PhantomData<U>,
}

// Constrain the new call with the same clause as the Customizer impl to
// prevent some confusion
impl<U, T> ApkIPCCustomizer<U>
where
    T: ApkIPC + Display,
    U: Deref<Target = T>,
{
    pub fn new(db: DeviceSqliteDatabase, hidden_apks: HashSet<i32>) -> Self {
        Self {
            db,
            hidden_apks,
            marker: PhantomData::default(),
        }
    }
}

fn open_for_apk_and_class(
    ctx: &dyn Context,
    apk_id: i32,
    class_name: &ClassName,
    search: &str,
) -> anyhow::Result<()> {
    let db = DeviceSqliteDatabase::new(ctx)?;
    let apk = db.get_apk_by_id(apk_id)?;

    let apk_paths = find_fully_qualified_apk(ctx, &apk.name)?;

    if apk_paths.len() > 1 {
        anyhow::bail!("multiple apks found for {}", apk.name);
    }

    let apk_path = match apk_paths.get(0) {
        None => anyhow::bail!("failed to find path for {}", apk.name),
        Some(v) => v,
    };

    let smali_file_path =
        find_smali_file_for_class(ctx, &class_name, Some(&apk_path)).ok_or_else(|| {
            anyhow::Error::msg(format!(
                "failed to find smali file for {} in {}",
                class_name, apk.name
            ))
        })?;

    invoke_dtu_open_file(ctx, path_must_str(&smali_file_path), search)?;
    Ok(())
}

impl<U, T> Customizer<U> for ApkIPCCustomizer<U>
where
    T: ApkIPC + Display,
    U: Deref<Target = T>,
{
    fn display(&self, item: &U) -> String {
        match &item.get_generic_permission() {
            Some(p) => format!("{}\n   P => {}", item.deref(), p),
            None => item.to_string(),
        }
    }

    fn filter(&self, item: &U) -> bool {
        self.hidden_apks.contains(&item.get_apk_id())
    }

    fn style(&self, item: &U) -> Option<Style> {
        if !item.requires_permission() {
            Some(Style::default().fg(PURPLE))
        } else {
            None
        }
    }

    fn clipboard_selection(&self, ctx: &dyn Context, item: &U) -> anyhow::Result<()> {
        let cmd = match item.get_kind() {
            ApkIPCKind::Receiver => format!(
                "dtu broadcast -c '{}/{}'",
                item.get_package(),
                item.get_class_name()
            ),
            ApkIPCKind::Activity => format!(
                "dtu start-activity -c '{}/{}'",
                item.get_package(),
                item.get_class_name()
            ),
            ApkIPCKind::Service => {
                let apk = self.db.get_apk_by_id(item.get_apk_id())?;
                format!(
                    "dtu call app-service -A '{}' -c '{}'",
                    apk.name,
                    item.get_class_name()
                )
            }
            ApkIPCKind::Provider => {
                anyhow::bail!("providers handled elsewhere");
            }
        };

        invoke_dtu_clipboard(ctx, &cmd)
    }

    fn open_selection(&self, ctx: &dyn Context, item: &U) -> anyhow::Result<()> {
        let apk_id = item.get_apk_id();
        let class_name = item.get_class_name();
        let search = match item.get_kind() {
            ApkIPCKind::Service => "onBind",
            ApkIPCKind::Receiver => "onReceive",
            ApkIPCKind::Activity => "onCreate",
            ApkIPCKind::Provider => "call",
        };
        open_for_apk_and_class(ctx, apk_id, &class_name, search)
    }

    fn get_popup(&self, item: &U) -> Option<ClosureWidget> {
        let apk = self.db.get_apk_by_id(item.get_apk_id()).ok()?;
        let text = format!(
            "Source APK: {}\nApp package: {}\nClass: {}\nPackage: {}\nPermission: {}\n",
            apk.name,
            apk.app_name,
            item.get_class_name(),
            item.get_package(),
            item.get_generic_permission().unwrap_or("None")
        );
        let block = BlockBuilder::default()
            .with_style(Style::default().bg(FG_COLOR).fg(BG_COLOR))
            .with_text("Details")
            .build();
        let para = Paragraph::new(text).block(block);
        Some(ClosureWidget::new(Box::new(move |area, buf| {
            para.render(area, buf);
        })))
    }
}

pub struct ProviderCustomizer {
    db: DeviceSqliteDatabase,
    hidden_apks: HashSet<i32>,
}

impl ProviderCustomizer {
    pub fn new(db: DeviceSqliteDatabase, hidden_apks: HashSet<i32>) -> Self {
        Self { db, hidden_apks }
    }
}

impl Customizer<DiffedProvider> for ProviderCustomizer {
    fn display(&self, item: &DiffedProvider) -> String {
        match &item.permission {
            Some(p) => format!("{} - {}", item, p),
            None => {
                let read_perm = match &item.read_permission {
                    Some(it) => it.as_str(),
                    None => "",
                };
                let write_perm = match &item.write_permission {
                    Some(it) => it.as_str(),
                    None => "",
                };
                if !read_perm.is_empty() && !write_perm.is_empty() {
                    if read_perm != write_perm {
                        format!("{}\n   R => {}\n   W => {}", item, read_perm, write_perm)
                    } else {
                        format!("{}\n   R/W => {}", item, read_perm)
                    }
                } else if !read_perm.is_empty() {
                    format!("{}\n   R => {}", item, read_perm)
                } else if !write_perm.is_empty() {
                    format!("{}\n   W => {}", item, write_perm)
                } else {
                    item.to_string()
                }
            }
        }
    }

    fn filter(&self, item: &DiffedProvider) -> bool {
        self.hidden_apks.contains(&item.apk_id)
    }

    fn style(&self, item: &DiffedProvider) -> Option<Style> {
        let has_perm = item.permission.is_some()
            || item.read_permission.is_some()
            || item.write_permission.is_some();
        if !has_perm {
            Some(Style::default().fg(PURPLE))
        } else {
            None
        }
    }

    fn clipboard_selection(&self, ctx: &dyn Context, item: &DiffedProvider) -> anyhow::Result<()> {
        let cmd = format!("dtu provider -a '{}'", item.provider.authorities);
        invoke_dtu_clipboard(ctx, &cmd)
    }

    fn open_selection(&self, ctx: &dyn Context, item: &DiffedProvider) -> anyhow::Result<()> {
        let apk_id = item.apk_id;
        let class_name = item.get_class_name();
        open_for_apk_and_class(ctx, apk_id, &class_name, "call")
    }

    fn get_popup(&self, item: &DiffedProvider) -> Option<ClosureWidget> {
        let apk = self.db.get_apk_by_id(item.apk_id).ok()?;
        let text = format!(
            "Source APK: {}\nApp package: {}\nAuthorities: {}\nName: {}\nPermission: {}\nWrite Permission: {}\nRead Permission: {}\n",
            apk.name,
            apk.app_name,
            item.authorities,
            item.name,
            item.permission
                .as_ref()
                .map(|it| it.as_str())
                .unwrap_or("None"),
                        item.read_permission
                .as_ref()
                .map(|it| it.as_str())
                .unwrap_or("None"),
            item.write_permission
                .as_ref()
                .map(|it| it.as_str())
                .unwrap_or("None")
        );
        let block = BlockBuilder::default()
            .with_style(Style::default().bg(FG_COLOR).fg(BG_COLOR))
            .with_text("Details")
            .build();
        let para = Paragraph::new(text).block(block);
        Some(ClosureWidget::new(Box::new(move |area, buf| {
            para.render(area, buf);
        })))
    }
}
