// This could use a custom allocator or maybe some sort of mem slab class for
// the backing storage for Strings, but we don't have that yet

use crate::db::graph::SetupEvent;
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::{ensure_dir_exists, open_file};
use crossbeam::channel::{bounded, Receiver, Sender};
use csv;
use itertools::Itertools;
use rayon::prelude::*;
use smalisa::instructions::InvArgs;
use smalisa::{AccessFlag, Lexer, Line, LineParse, Parser, Primitive, RawLiteral};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::iter::Iterator;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use walkdir::{DirEntry, WalkDir};

use super::{Error, Result};

fn get_csv_writer_file(out_dir: &PathBuf, path: &str) -> Result<csv::Writer<BufWriter<File>>> {
    let full_path = out_dir.join(path);
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
struct CallInfo {
    source_class: String,
    source_method: String,
    source_args: String,
    target_class: String,
    target_method: String,
    target_args: String,
}

struct Channels {
    classes: Sender<ClassInfo>,
    supers: Sender<SuperInfo>,
    ifaces: Sender<InterfaceInfo>,
    methods: Sender<MethodInfo>,
    calls: Sender<CallInfo>,
    strings: Sender<String>,
}

impl Channels {
    fn send_class(&self, class: &str, access_flags: AccessFlag) {
        let _ = self.classes.send(ClassInfo {
            name: class.to_string(),
            access_flags: access_flags.bits(),
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

    fn send_string(&self, s: &str) {
        let _ = self.strings.send(s.into());
    }
}

fn launch_writers(
    out_dir: &PathBuf,
    classes: Receiver<ClassInfo>,
    supers: Receiver<SuperInfo>,
    ifaces: Receiver<InterfaceInfo>,
    methods: Receiver<MethodInfo>,
    calls: Receiver<CallInfo>,
    strings: Receiver<String>,
) -> Result<Vec<JoinHandle<()>>> {
    let mut handles = Vec::with_capacity(5);

    let mut classes_file = get_csv_writer_file(out_dir, "classes.csv")?;
    let mut supers_file = get_csv_writer_file(out_dir, "supers.csv")?;
    let mut iface_file = get_csv_writer_file(out_dir, "interfaces.csv")?;
    let mut methods_file = get_csv_writer_file(out_dir, "methods.csv")?;
    let mut calls_file = get_csv_writer_file(out_dir, "calls.csv")?;

    let strings_path = out_dir.join("strings.txt");
    let strings_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&strings_path)?;

    let mut strings_writer = BufWriter::new(strings_file);

    let mut handle = std::thread::spawn(move || {
        for class in classes.iter().unique() {
            if let Err(e) =
                classes_file.write_record(&[&class.name, &class.access_flags.to_string()])
            {
                log::error!("failed to write class {} to csv: {}", class.name, e);
            }
        }
        let _ = classes_file.flush();
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        for string in strings.iter().unique() {
            if let Err(e) = strings_writer.write(string.as_bytes()) {
                log::error!("failed to write string: {}", e);
            }
            let _ = strings_writer.write(&[b'\n']);
        }

        if let Err(e) = strings_writer.flush() {
            log::error!("failed to flush strings writer: {}", e);
        }
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        for sup in supers.iter().unique() {
            if let Err(e) = supers_file.write_record(&[&sup.child, &sup.parent]) {
                log::error!(
                    "failed to write super relation {} : {} to csv: {}",
                    sup.child,
                    sup.parent,
                    e
                );
            }
        }
        let _ = supers_file.flush();
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        for iface in ifaces.iter().unique() {
            if let Err(e) = iface_file.write_record(&[&iface.class, &iface.interface]) {
                log::error!(
                    "failed to write interface relation {} : {} to csv: {}",
                    iface.class,
                    iface.interface,
                    e
                );
            }
        }
        let _ = iface_file.flush();
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
        for m in methods.iter().unique() {
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
        let _ = methods_file.flush();
    });
    handles.push(handle);

    handle = std::thread::spawn(move || {
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
        let _ = calls_file.flush();
    });
    handles.push(handle);

    return Ok(handles);
}

fn write_analysis_files_internal<M, FF, CF>(
    out_dir: &PathBuf,
    monitor: &M,
    cancel_check: &TaskCancelCheck,
    input_dir: &PathBuf,
    file_ignore_func: FF,
    class_ignore_func: CF,
) -> Result<()>
where
    M: EventMonitor<SetupEvent> + ?Sized,
    FF: Fn(&DirEntry) -> bool + Send + Sync,
    CF: Fn(&str) -> bool + Send + Sync,
{
    let (class_tx, class_rx) = bounded(128);
    let (super_tx, super_rx) = bounded(128);
    let (iface_tx, iface_rx) = bounded(128);
    let (method_tx, method_rx) = bounded(128);
    let (call_tx, call_rx) = bounded(128);
    let (string_tx, string_rx) = bounded(128);

    let channels = Arc::new(Channels {
        classes: class_tx,
        supers: super_tx,
        ifaces: iface_tx,
        methods: method_tx,
        calls: call_tx,
        strings: string_tx,
    });

    let handles = launch_writers(
        out_dir, class_rx, super_rx, iface_rx, method_rx, call_rx, string_rx,
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

    monitor.on_event(SetupEvent::SmalisaStart {
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
                monitor.on_event(SetupEvent::SmalisaFileStarted { path: path.clone() });
                let success = match handle_entry(Arc::clone(&channels), &ent, &class_ignore_func) {
                    Err(e) => {
                        log::error!("{}", e);
                        false
                    }
                    _ => true,
                };
                monitor.on_event(SetupEvent::SmalisaFileComplete { path, success })
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
    input_dir: &PathBuf,
    out_dir: &PathBuf,
    file_ignore_func: FF,
    class_ignore_func: CF,
) -> Result<()>
where
    M: EventMonitor<SetupEvent> + ?Sized,
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
        for f in [
            "classes.csv",
            "supers.csv",
            "interfaces.csv",
            "methods.csv",
            "calls.csv",
            "strings.txt",
        ] {
            let path = out_dir.join(f);
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
        }
    }
    res
}

fn direntry_is_smali_file(ent: &DirEntry) -> bool {
    ent.file_type().is_file() && ent.path().extension().map_or(false, |ext| ext == "smali")
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

fn handle_entry<F>(channels: Arc<Channels>, ent: &DirEntry, class_ignore_func: &F) -> Result<()>
where
    F: Fn(&str) -> bool + Send + Sync,
{
    let path = ent.path();
    let file = open_file(path)?;
    let mut class = "";
    let mut calling_method_args = "";
    let mut calling_method_name = "";
    let lexer = Lexer::new(&file);
    let mut parser = Parser::new(lexer);
    let mut line = Line::Empty;

    // It's worth doing some deduping in this method since there are often duplicates and it makes
    // things a little easier on the global deduplication
    let mut seen_calls: HashSet<SeenCall> = HashSet::new();
    let mut seen_strings: HashSet<&str> = HashSet::new();

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
                    if seen_strings.insert(s) {
                        channels.send_string(s);
                    }
                }
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
            }
            Line::InstructionInvocation(ref inv) => {
                if inv.is_call() {
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
                    if seen_strings.insert(s) {
                        channels.send_string(s);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}
