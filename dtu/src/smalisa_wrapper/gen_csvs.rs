// This could use a custom allocator or maybe some sort of mem slab class for
// the backing storage for Strings, but we don't have that yet

use crate::db::graph::models::FieldAccessOp;
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::{ensure_dir_exists, open_file, OS_PATH_SEP_CHAR};
use crossbeam::channel::{bounded, Receiver, Sender};
use csv;
use itertools::Itertools;
use rayon::prelude::*;
use smalisa::instructions::{InvArgs, Invocation};
use smalisa::{AccessFlag, Field, Lexer, Line, LineParse, Parser, Primitive, RawLiteral};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::BufWriter;
use std::iter::Iterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use walkdir::{DirEntry, WalkDir};

use super::{Error, Result};

pub enum Event {
    /// Fired when starting Smalisa
    Start { total_files: usize },
    /// Fired when Smalisa starts a given file
    FileStarted { path: String },
    /// Fired when Smalisa completes a given file
    FileComplete { path: String, success: bool },
    /// Fired when all files have been passed through Smalisa
    Done { success: bool },
}

fn get_csv_writer_file(out_dir: &Path, kind: CSV) -> Result<csv::Writer<BufWriter<File>>> {
    let full_path = kind.in_path(out_dir);
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&full_path)?;

    let wr = BufWriter::new(file);

    Ok(csv::WriterBuilder::new().has_headers(false).from_writer(wr))
}

impl From<csv::Error> for Error {
    fn from(value: csv::Error) -> Self {
        Self::Generic(value.to_string())
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
#[repr(u8)]
pub enum CSV {
    Classes,
    Methods,
    Supers,
    Interfaces,
    Calls,
    ClassFields,
    MethodFieldAccess,
    Strings,
    MethodStrings,
}

impl CSV {
    pub const fn all() -> &'static [CSV] {
        // This is a big cheesey, but the order matters here because we use this for the database
        // loading code. I could just make the order matter _there_, but meh I like this function
        // because it's obvious to me when it needs to change.
        &[
            CSV::Classes,
            // These requre Classes
            CSV::Supers,
            CSV::Interfaces,
            CSV::ClassFields,
            CSV::Methods,
            // This requires Methods
            CSV::Calls,
            // This requires Methods and ClassFields
            CSV::MethodFieldAccess,
            CSV::Strings,
            // This requires Strings
            CSV::MethodStrings,
        ]
    }

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Classes => "classes.csv",
            Self::Methods => "methods.csv",
            Self::Supers => "supers.csv",
            Self::Interfaces => "interfaces.csv",
            Self::Calls => "calls.csv",
            Self::ClassFields => "class_fields.csv",
            Self::MethodFieldAccess => "method_field_access.csv",
            Self::Strings => "strings.csv",
            Self::MethodStrings => "method_strings.csv",
        }
    }

    pub fn in_path(self, dir: &Path) -> PathBuf {
        dir.join(self.file_name())
    }

    pub fn in_dir(self, dir: &str) -> String {
        let mut s = String::from(dir);
        s.push(OS_PATH_SEP_CHAR);
        s.push_str(self.file_name());
        s
    }
}

impl fmt::Display for CSV {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file_name())
    }
}

impl AsRef<str> for CSV {
    fn as_ref(&self) -> &str {
        (*self).file_name()
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ClassInfo {
    name: String,
    access_flags: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct SuperInfo {
    child: String,
    parent: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct InterfaceInfo {
    class: String,
    interface: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MethodInfo {
    class: String,
    name: String,
    args: String,
    ret: String,
    access_flags: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ClassField {
    class: String,
    name: String,
    ty: String,
    access_flags: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MethodFieldAccess {
    class: String,
    name: String,
    ty: String,
    method_class: String,
    method: String,
    method_args: String,
    op: FieldAccessOp,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CallInfo {
    source_class: String,
    source_method: String,
    source_args: String,
    target_class: String,
    target_method: String,
    target_args: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MethodString {
    method: String,
    method_args: String,
    string: String,
    class: String,
}

struct SendChannels {
    classes: Sender<ClassInfo>,
    supers: Sender<SuperInfo>,
    ifaces: Sender<InterfaceInfo>,
    methods: Sender<MethodInfo>,
    calls: Sender<CallInfo>,
    strings: Sender<String>,
    method_strings: Sender<MethodString>,
    class_fields: Sender<ClassField>,
    method_field_access: Sender<MethodFieldAccess>,
}

impl SendChannels {
    fn send_class(&self, class: &str, access_flags: AccessFlag) {
        let _ = self.classes.send(ClassInfo {
            name: class.to_string(),
            access_flags: access_flags.bits(),
        });
    }

    fn send_string(&self, s: &str) {
        let _ = self.strings.send(s.into());
    }

    fn send_field(&self, class: &str, field: &Field) {
        let ty = match field.ty.as_smali_str() {
            // If we can't get a smali format for any reason we don't really want to go forward.
            // This really shouldn't happen so I don't think it's a big deal.
            None => return,
            Some(v) => v.as_ref().into(),
        };

        let _ = self.class_fields.send(ClassField {
            class: class.into(),
            name: field.name.into(),
            ty,
            access_flags: field.access.bits(),
        });
    }

    fn send_super(&self, child: &str, parent: &str) {
        let _ = self.supers.send(SuperInfo {
            child: child.to_string(),
            parent: parent.to_string(),
        });
    }

    fn send_interface(&self, class: &str, iface: &str) {
        let _ = self.ifaces.send(InterfaceInfo {
            class: class.to_string(),
            interface: iface.to_string(),
        });
    }

    fn send_method(
        &self,
        class: &str,
        name: &str,
        args: &str,
        ret: Cow<'_, str>,
        access_flags: AccessFlag,
    ) {
        let _ = self.methods.send(MethodInfo {
            class: class.to_string(),
            name: name.to_string(),
            args: args.to_string(),
            ret: ret.to_string(),
            access_flags: access_flags.bits(),
        });
    }

    fn send_call(
        &self,
        src_class: &str,
        src_method: &str,
        src_args: &str,
        target_class: &str,
        target_method: &str,
        target_args: &str,
    ) {
        let _ = self.calls.send(CallInfo {
            target_class: target_class.to_string(),
            target_method: target_method.to_string(),
            target_args: target_args.to_string(),
            source_class: src_class.to_string(),
            source_method: src_method.to_string(),
            source_args: src_args.to_string(),
        });
    }

    fn send_method_string(&self, string: &str, method: &str, method_args: &str, class: &str) {
        let _ = self.method_strings.send(MethodString {
            string: string.into(),
            method: method.into(),
            method_args: method_args.into(),
            class: class.into(),
        });
    }
}

fn launch_writers(
    out_dir: &Path,
    classes: Receiver<ClassInfo>,
    supers: Receiver<SuperInfo>,
    ifaces: Receiver<InterfaceInfo>,
    methods: Receiver<MethodInfo>,
    calls: Receiver<CallInfo>,
    strings: Receiver<String>,
    method_strings: Receiver<MethodString>,
    class_fields: Receiver<ClassField>,
    method_field_access: Receiver<MethodFieldAccess>,
) -> Result<Vec<JoinHandle<()>>> {
    let mut handles = Vec::with_capacity(CSV::all().len());

    let mut classes_file = get_csv_writer_file(out_dir, CSV::Classes)?;
    let mut supers_file = get_csv_writer_file(out_dir, CSV::Supers)?;
    let mut iface_file = get_csv_writer_file(out_dir, CSV::Interfaces)?;
    let mut methods_file = get_csv_writer_file(out_dir, CSV::Methods)?;
    let mut calls_file = get_csv_writer_file(out_dir, CSV::Calls)?;
    let mut fields_file = get_csv_writer_file(out_dir, CSV::ClassFields)?;
    let mut field_access_file = get_csv_writer_file(out_dir, CSV::MethodFieldAccess)?;
    let mut strings_file = get_csv_writer_file(out_dir, CSV::Strings)?;
    let mut method_strings_file = get_csv_writer_file(out_dir, CSV::MethodStrings)?;

    let mut handle = std::thread::spawn(move || {
        // No need to deduplicate, we only send classes via the `.class` smali directive and we're
        // assuming our own directory structure meaning there can only ever be one class of a given
        // name per source.
        for class in classes {
            if let Err(e) =
                classes_file.write_record(&[&class.name, &class.access_flags.to_string()])
            {
                log::error!("failed to write class {} to csv: {}", class.name, e);
            }
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // No need to deduplicate here because field names must be unique and classes are
        // already unique
        for field in class_fields {
            if let Err(e) = fields_file.write_record(&[
                &field.class,
                &field.name,
                &field.ty,
                &field.access_flags.to_string(),
            ]) {
                log::error!("failed to write field: {}", e);
            }
        }

        if let Err(e) = fields_file.flush() {
            log::error!("failed toflush field file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // Worth deduplicating here because the same method can access a field multiple times
        for access in method_field_access.iter().unique() {
            if let Err(e) = field_access_file.write_record(&[
                &access.class,
                &access.name,
                &access.ty,
                &access.method_class,
                &access.method,
                &access.method_args,
                access.op.as_ref(),
            ]) {
                log::error!("failed to write field access: {}", e);
            }
        }

        if let Err(e) = field_access_file.flush() {
            log::error!("failed to flush field_access_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // Worth deduplicating here because the same string can appear a ton in the same source
        for s in strings.iter().unique() {
            if let Err(e) = strings_file.write_record(&[&s]) {
                log::error!("failed to write string: {}", e);
            }
        }
        if let Err(e) = strings_file.flush() {
            log::error!("failed to flush strings_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // Worth deduplicating here because the same method can use the same string multiple times
        for ms in method_strings.iter().unique() {
            if let Err(e) = method_strings_file.write_record(&[
                &ms.string,
                &ms.method,
                &ms.method_args,
                &ms.class,
            ]) {
                log::error!("failed to write method string reference: {}", e);
            }
        }
        if let Err(e) = method_strings_file.flush() {
            log::error!("failed to flush method_strings_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // No need to deduplicate here because Java only allows a single super for a class
        for sup in supers {
            if let Err(e) = supers_file.write_record(&[&sup.child, &sup.parent]) {
                log::error!(
                    "failed to write super relation {} : {} to csv: {}",
                    sup.child,
                    sup.parent,
                    e
                );
            }
        }
        if let Err(e) = supers_file.flush() {
            log::error!("failed to flush supers_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // No need to deduplicate here because smali wouldn't include the same interface for the
        // same class multiple times
        for iface in ifaces {
            if let Err(e) = iface_file.write_record(&[&iface.class, &iface.interface]) {
                log::error!(
                    "failed to write interface relation {} : {} to csv: {}",
                    iface.class,
                    iface.interface,
                    e
                );
            }
        }
        if let Err(e) = iface_file.flush() {
            log::error!("failed to flush iface_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // No need to deduplicate here because we're storing all the info needed to uniquely
        // identify a method
        for m in methods {
            if let Err(e) = methods_file.write_record(&[
                &m.class,
                &m.name,
                &m.args,
                &m.ret,
                &m.access_flags.to_string(),
            ]) {
                log::error!(
                    "failed to write method {}->{}({}) to csv: {}",
                    m.class,
                    m.name,
                    m.args,
                    e
                );
            }
        }
        if let Err(e) = methods_file.flush() {
            log::error!("failed to flush methods_file: {e}");
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        // Worth deduplicating here because the same method may call the another method multiple
        // times
        for c in calls.iter().unique() {
            if let Err(e) = calls_file.write_record(&[
                &c.source_class,
                &c.source_method,
                &c.source_args,
                &c.target_class,
                &c.target_method,
                &c.target_args,
            ]) {
                log::error!(
                    "failed to write call relation {}->{}({}) calls {}->{}({}) to csv: {}",
                    c.source_class,
                    c.source_method,
                    c.source_args,
                    c.target_class,
                    c.target_method,
                    c.target_args,
                    e
                );
            }
        }

        if let Err(e) = calls_file.flush() {
            log::error!("failed to flush calls_file: {e}");
        }
    });
    handles.push(handle);

    return Ok(handles);
}

fn write_analysis_files_internal<M, FF, CF>(
    out_dir: &Path,
    monitor: &M,
    cancel_check: &TaskCancelCheck,
    input_dir: &Path,
    file_ignore_func: FF,
    class_ignore_func: CF,
) -> Result<()>
where
    M: EventMonitor<Event> + ?Sized,
    FF: Fn(&DirEntry) -> bool + Send + Sync,
    CF: Fn(&str) -> bool + Send + Sync,
{
    let (class_tx, class_rx) = bounded(128);
    let (super_tx, super_rx) = bounded(128);
    let (iface_tx, iface_rx) = bounded(128);
    let (method_tx, method_rx) = bounded(128);
    let (call_tx, call_rx) = bounded(128);
    let (string_tx, string_rx) = bounded(128);
    let (method_string_tx, method_string_rx) = bounded(128);
    let (fields_tx, fields_rx) = bounded(128);
    let (field_access_tx, field_access_rx) = bounded(128);

    let channels = Arc::new(SendChannels {
        classes: class_tx,
        supers: super_tx,
        ifaces: iface_tx,
        methods: method_tx,
        calls: call_tx,
        strings: string_tx,
        method_strings: method_string_tx,
        class_fields: fields_tx,
        method_field_access: field_access_tx,
    });

    let handles = launch_writers(
        out_dir,
        class_rx,
        super_rx,
        iface_rx,
        method_rx,
        call_rx,
        string_rx,
        method_string_rx,
        fields_rx,
        field_access_rx,
    )?;

    let entry_filter = |e: &walkdir::Result<DirEntry>| -> bool {
        e.as_ref().map_or(false, |it| {
            direntry_is_smali_file(it) && !file_ignore_func(it)
        })
    };

    let total_count = WalkDir::new(input_dir)
        .into_iter()
        .filter(entry_filter)
        .count();

    monitor.on_event(Event::Start {
        total_files: total_count,
    });

    let iter = WalkDir::new(input_dir).into_iter();

    iter.filter(entry_filter)
        .par_bridge()
        .into_par_iter()
        .for_each(|d| {
            if let Ok(ent) = d {
                if cancel_check.was_cancelled() {
                    return;
                }
                let path = ent.path().to_string_lossy().to_string();
                monitor.on_event(Event::FileStarted { path: path.clone() });
                let success = match handle_entry(Arc::clone(&channels), &ent, &class_ignore_func) {
                    Err(e) => {
                        log::error!("{}", e);
                        false
                    }
                    _ => true,
                };
                monitor.on_event(Event::FileComplete { path, success })
            }
        });
    if cancel_check.was_cancelled() {
        return Err(Error::Cancelled);
    }
    drop(channels);
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

/// Writes the CSV files required for a graph import
pub fn write_analysis_files<M, FF, CF>(
    monitor: &M,
    cancel_check: &TaskCancelCheck,
    input_dir: &Path,
    out_dir: &Path,
    file_ignore_func: FF,
    class_ignore_func: CF,
) -> Result<()>
where
    M: EventMonitor<Event> + ?Sized,
    FF: Fn(&DirEntry) -> bool + Send + Sync,
    CF: Fn(&str) -> bool + Send + Sync,
{
    ensure_dir_exists(out_dir)?;

    let res = write_analysis_files_internal(
        out_dir,
        monitor,
        cancel_check,
        input_dir,
        file_ignore_func,
        class_ignore_func,
    );

    if res.is_err() {
        for csv in CSV::all() {
            let path = csv.in_path(out_dir);
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
        }
    }

    monitor.on_event(Event::Done {
        success: res.is_ok(),
    });

    res
}

fn direntry_is_smali_file(ent: &DirEntry) -> bool {
    ent.file_type().is_file() && ent.path().extension().map_or(false, |ext| ext == "smali")
}

#[derive(PartialEq, Eq, Hash)]
struct SeenMethodString<'a> {
    method: &'a str,
    method_args: &'a str,
    string: &'a str,
}

#[derive(PartialEq, Eq, Hash)]
struct SeenCall<'a> {
    class: &'a str,
    name: &'a str,
    args: &'a str,
}

const IGNORE_SUPERS: &'static [&'static str] = &["Ljava/lang/Object;"];

fn should_ignore_super(name: &str) -> bool {
    for s in IGNORE_SUPERS {
        if name.starts_with(s) {
            return true;
        }
    }
    false
}

fn on_field_access<F>(
    chan: &Sender<MethodFieldAccess>,
    class_ignore_func: &F,
    class: &str,
    method: &str,
    method_args: &str,
    inv: &Invocation,
) where
    F: Fn(&str) -> bool + Send + Sync,
{
    let fref = match inv.args() {
        InvArgs::OneRegField(_, v) => v,
        InvArgs::TwoRegField(_, _, v) => v,
        _ => return,
    };

    if class_ignore_func(fref.class) {
        return;
    }

    let op = if inv.sets_field() {
        FieldAccessOp::Write
    } else {
        FieldAccessOp::Read
    };

    let ty = match fref.ty.as_smali_str() {
        // Match the behavior in send_field
        None => return,
        Some(v) => v.as_ref().into(),
    };

    let it = MethodFieldAccess {
        class: fref.class.into(),
        name: fref.name.into(),
        method_class: class.into(),
        method: method.into(),
        method_args: method_args.into(),
        ty,
        op,
    };

    _ = chan.send(it);
}

fn handle_entry<F>(channels: Arc<SendChannels>, ent: &DirEntry, class_ignore_func: &F) -> Result<()>
where
    F: Fn(&str) -> bool + Send + Sync,
{
    let path = ent.path();
    let file = open_file(path)?;
    let mut class = "";
    let mut calling_method_args = "";
    let mut calling_method_name = "";
    let lexer = Lexer::new_buffered(&file);
    let mut parser = Parser::new(lexer);
    let mut line = Line::Empty;

    // It's worth doing some deduping in this method since there are often duplicates and it makes
    // things a little easier on the global deduplication
    let mut seen_calls: HashSet<SeenCall> = HashSet::new();
    let mut seen_method_strings: HashSet<SeenMethodString> = HashSet::new();

    macro_rules! send_str {
        ($s:expr) => {
            channels.send_string($s);
            if !calling_method_name.is_empty() {
                let seen = SeenMethodString {
                    method: calling_method_name,
                    method_args: calling_method_args,
                    string: $s,
                };
                if seen_method_strings.insert(seen) {
                    channels.send_method_string(
                        $s,
                        calling_method_name,
                        calling_method_args,
                        class,
                    );
                }
            }
        };
    }

    loop {
        let res = parser.parse_line_into(&mut line);
        if let Err(perr) = res {
            if perr.is_eof() {
                break;
            }
            return Err(Error::from(perr));
        }
        match line {
            Line::Class(flags, clazz) => {
                if class_ignore_func(clazz) {
                    return Ok(());
                }
                class = clazz;
                channels.send_class(clazz, flags);
            }

            Line::Super(sup) => {
                if !should_ignore_super(sup) {
                    channels.send_super(class, sup);
                }
            }
            Line::Interface(iface) => {
                channels.send_interface(class, iface);
            }
            Line::Field(ref field) => {
                if let RawLiteral::String(s) = field.raw_value {
                    send_str!(s);
                }

                channels.send_field(class, field);
            }
            Line::MethodHeader(ref mh) => {
                calling_method_name = mh.name;
                calling_method_args = mh.args;
                channels.send_method(
                    class,
                    mh.name,
                    mh.args,
                    mh.return_type
                        .as_smali_str()
                        .unwrap_or(Cow::Borrowed(Primitive::Void.as_smali_str())),
                    mh.access,
                );
            }
            Line::MethodEnd => {
                seen_calls.clear();
                seen_method_strings.clear();
            }
            Line::InstructionInvocation(ref inv) => {
                if inv.sets_field() || inv.gets_field() {
                    on_field_access(
                        &channels.method_field_access,
                        class_ignore_func,
                        class,
                        calling_method_name,
                        calling_method_args,
                        inv,
                    );
                } else if inv.is_call() {
                    if let InvArgs::VarRegMethod(_, mref) = inv.args() {
                        let target_class = mref.full_class_str();

                        if target_class == "Ljava/lang/Object;" && mref.name == "<init>" {
                            continue;
                        }

                        if class_ignore_func(mref.class) {
                            continue;
                        }

                        let call = SeenCall {
                            class: mref.class,
                            name: mref.name,
                            args: mref.args,
                        };

                        if !seen_calls.insert(call) {
                            continue;
                        }

                        channels.send_call(
                            class,
                            calling_method_name,
                            calling_method_args,
                            target_class.as_ref(),
                            mref.name,
                            mref.args,
                        );
                    }
                } else if let InvArgs::RegStr(_, s) = inv.args() {
                    send_str!(s);
                }
            }
            _ => {}
        }
    }

    Ok(())
}
