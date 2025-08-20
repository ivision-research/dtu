use crossbeam::channel::{Receiver, RecvError};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use log::Level::Debug;
use log::{debug, log_enabled};

// TODO For some testing purposes I'd like to create a CommandExecutor trait
//  or something. This isn't a high priority at the moment, but before actually
//  building out functionality I'd like it done.

pub struct CmdOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn err_on_status(status: ExitStatus) -> crate::Result<()> {
    if status.success() {
        return Ok(());
    }
    // TODO
    let code = status.code().expect("failed to unwrap exit status");

    Err(crate::Error::CommandError(code, String::new()))
}

impl CmdOutput {
    /// Converts to a `Result` object that is `Ok` only if the [ExitStatus] is
    /// success.
    pub fn err_on_status(self) -> crate::Result<Self> {
        if self.status.success() {
            return Ok(self);
        }

        // TODO
        let code = self.status.code().expect("failed to unwrap exit status");

        Err(crate::Error::CommandError(
            code,
            self.stderr_utf8_lossy().to_string(),
        ))
    }
}

/// Splits a string for use as a shell command
pub fn split<'a>(s: &'a str) -> Option<Vec<String>> {
    let mut escaped = false;
    let mut single_quote = false;
    let mut double_quote = false;

    let mut into = String::new();

    let mut sp = Vec::new();

    macro_rules! finish {
        () => {
            sp.push(into.clone());
            into.clear();
        };
    }

    for c in s.chars() {
        if escaped {
            escaped = false;
            into.push(c);
            continue;
        }

        match c {
            '\\' => {
                escaped = true;
            }

            '\'' if single_quote => {
                single_quote = false;
                finish!();
            }

            '\'' if !double_quote => {
                single_quote = true;
            }

            '"' if double_quote => {
                double_quote = false;
                finish!();
            }

            '"' if !single_quote => {
                double_quote = true;
            }

            _ => {
                if single_quote || double_quote || !c.is_whitespace() {
                    into.push(c);
                } else if into.len() > 0 {
                    finish!();
                }
            }
        }
    }

    if escaped | single_quote | double_quote {
        return None;
    }

    if into.len() > 0 {
        sp.push(into);
    }

    Some(sp)
}

impl From<Output> for CmdOutput {
    fn from(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

/// Quotes a string with single quotes
pub fn quote(s: &str) -> String {
    let mut new = String::with_capacity(s.len() + 2);
    new.push('\'');
    for c in s.chars() {
        if c == '\'' {
            new.push_str("'\"'\"'");
        } else {
            new.push(c);
        }
    }
    new.push('\'');
    new
}

impl CmdOutput {
    #[inline]
    pub fn ok(&self) -> bool {
        self.status.success()
    }

    #[inline]
    pub fn stdout_contains(&self, needle: &str) -> bool {
        self.stdout_utf8_lossy().contains(needle)
    }

    #[inline]
    pub fn stderr_contains(&self, needle: &str) -> bool {
        self.stderr_utf8_lossy().contains(needle)
    }

    #[inline]
    pub fn stdout_utf8_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    #[inline]
    pub fn stderr_utf8_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

pub type OutputCallback<'a> = dyn FnMut(&[u8]) -> anyhow::Result<()> + 'a;
pub type LineCallback<'a> = dyn FnMut(&str) -> anyhow::Result<()> + 'a;

/// Run a command passing the stdout and stderr to the given closures.
///
/// The passed [kill_child] channel can be used to kill the child, otherwise
/// this function runs until the child finishes
pub fn run_cmd_streamed<C, S>(
    cmd: C,
    args: &[S],
    on_stdout: &mut OutputCallback,
    on_stderr: &mut OutputCallback,
    kill_child: Option<Receiver<()>>,
) -> io::Result<ExitStatus>
where
    C: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    if log_enabled!(Debug) {
        log_cmd(&cmd, args);
    }
    let mut child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;

    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();

    let child = Arc::new(Mutex::new(child));

    let mut callback_error = None;

    let waited = if let Some(chan) = kill_child {
        let waited = Arc::new(AtomicBool::new(false));
        let inner_waited = Arc::clone(&waited);
        let child_clone = Arc::clone(&child);
        std::thread::spawn(move || match chan.recv() {
            Ok(_) => {
                if !inner_waited.load(Ordering::Relaxed) {
                    if let Err(e) = child_clone.lock().expect("poisoned").kill() {
                        log::error!("failed to kill child: {}", e);
                    }
                }
            }
            Err(RecvError) => {}
        });
        Some(waited)
    } else {
        None
    };

    // This blocks reading the pipes until the child goes away
    read2(out, err, &mut |is_out, data, _eof| {
        let callback_result = if is_out {
            on_stdout(data.as_slice())
        } else {
            on_stderr(data.as_slice())
        };
        if let Err(e) = callback_result {
            callback_error = Some(e);
        }
    })?;

    let res = { child.lock().expect("poisoned").wait() };
    if let Some(w) = waited {
        w.store(true, Ordering::Relaxed);
    }
    res
}

pub fn run_cmd_split_streamed<C, S>(
    cmd: C,
    args: &[S],
    split_on: u8,
    on_stdout_line: &mut LineCallback,
    on_stderr_line: &mut LineCallback,
) -> io::Result<ExitStatus>
where
    C: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    if log_enabled!(Debug) {
        log_cmd(&cmd, args);
    }

    let mut child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;

    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();

    let mut callback_error = None;
    let mut stdout_pos = 0;
    let mut stderr_pos = 0;

    read2(out, err, &mut |is_out, data, eof| {
        let pos = if is_out {
            &mut stdout_pos
        } else {
            &mut stderr_pos
        };
        let idx = if eof {
            data.len()
        } else {
            match data[*pos..].iter().rposition(|b| *b == split_on) {
                Some(i) => *pos + i + 1,
                None => {
                    *pos = data.len();
                    return;
                }
            }
        };

        let all_data = &data[..idx];

        for slice in all_data.split(|b| *b == split_on) {
            let as_string = String::from_utf8_lossy(slice);
            if callback_error.is_some() {
                break;
            }
            let callback_result = if is_out {
                on_stdout_line(&as_string)
            } else {
                on_stderr_line(&as_string)
            };
            if let Err(e) = callback_result {
                callback_error = Some(e);
                break;
            }
        }

        data.drain(..idx);
        *pos = 0;
    })?;

    child.wait()
}

pub fn spawn_cmd<C, S>(cmd: C, args: &[S]) -> io::Result<Child>
where
    C: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    if log_enabled!(Debug) {
        log_cmd(&cmd, args);
    }
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub fn run_cmd<C, S>(cmd: C, args: &[S]) -> io::Result<CmdOutput>
where
    C: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    if log_enabled!(Debug) {
        log_cmd(&cmd, args);
    }
    Command::new(cmd)
        .args(args)
        .output()
        .map(|output| output.into())
}

pub fn log_cmd<C, S>(cmd: &C, args: &[S])
where
    C: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    let nargs = args.len();
    if nargs > 0 {
        let mut args_string = String::new();
        for (i, e) in args.iter().enumerate() {
            args_string.push_str(&e.as_ref().to_string_lossy());
            if i < nargs - 1 {
                args_string.push(' ');
            }
        }
        debug!(
            "Running command: `{} {}`",
            cmd.as_ref().to_string_lossy(),
            args_string
        );
    } else {
        debug!("Running command: `{}`", cmd.as_ref().to_string_lossy());
    }
}

// Shamelessly stolen from cargo_utils

use self::imp::read2;

#[cfg(unix)]
mod imp {
    use std::io;
    use std::io::prelude::*;
    use std::mem;
    use std::os::unix::prelude::*;
    use std::process::{ChildStderr, ChildStdout};

    pub fn read2(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        unsafe {
            libc::fcntl(out_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(err_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }

        let mut out_done = false;
        let mut err_done = false;
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut fds: [libc::pollfd; 2] = unsafe { mem::zeroed() };
        fds[0].fd = out_pipe.as_raw_fd();
        fds[0].events = libc::POLLIN;
        fds[1].fd = err_pipe.as_raw_fd();
        fds[1].events = libc::POLLIN;
        let mut nfds = 2;
        let mut errfd = 1;

        while nfds > 0 {
            // wait for either pipe to become readable using `select`
            let r = unsafe { libc::poll(fds.as_mut_ptr(), nfds, -1) };
            if r == -1 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }

            // Read as much as we can from each pipe, ignoring EWOULDBLOCK or
            // EAGAIN. If we hit EOF, then this will happen because the underlying
            // reader will return Ok(0), in which case we'll see `Ok` ourselves. In
            // this case we flip the other fd back into blocking mode and read
            // whatever's leftover on that file descriptor.
            let handle = |res: io::Result<_>| match res {
                Ok(_) => Ok(true),
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        Ok(false)
                    } else {
                        Err(e)
                    }
                }
            };
            if !err_done && fds[errfd].revents != 0 && handle(err_pipe.read_to_end(&mut err))? {
                err_done = true;
                nfds -= 1;
            }
            data(false, &mut err, err_done);
            if !out_done && fds[0].revents != 0 && handle(out_pipe.read_to_end(&mut out))? {
                out_done = true;
                fds[0].fd = err_pipe.as_raw_fd();
                errfd = 0;
                nfds -= 1;
            }
            data(true, &mut out, out_done);
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::prelude::*;
    use std::process::{ChildStderr, ChildStdout};
    use std::slice;

    use miow::iocp::{CompletionPort, CompletionStatus};
    use miow::pipe::NamedPipe;
    use miow::Overlapped;
    use winapi::shared::winerror::ERROR_BROKEN_PIPE;

    struct Pipe<'a> {
        dst: &'a mut Vec<u8>,
        overlapped: Overlapped,
        pipe: NamedPipe,
        done: bool,
    }

    pub fn read2(
        out_pipe: ChildStdout,
        err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let port = CompletionPort::new(1)?;
        port.add_handle(0, &out_pipe)?;
        port.add_handle(1, &err_pipe)?;

        unsafe {
            let mut out_pipe = Pipe::new(out_pipe, &mut out);
            let mut err_pipe = Pipe::new(err_pipe, &mut err);

            out_pipe.read()?;
            err_pipe.read()?;

            let mut status = [CompletionStatus::zero(), CompletionStatus::zero()];

            while !out_pipe.done || !err_pipe.done {
                for status in port.get_many(&mut status, None)? {
                    if status.token() == 0 {
                        out_pipe.complete(status);
                        data(true, out_pipe.dst, out_pipe.done);
                        out_pipe.read()?;
                    } else {
                        err_pipe.complete(status);
                        data(false, err_pipe.dst, err_pipe.done);
                        err_pipe.read()?;
                    }
                }
            }

            Ok(())
        }
    }

    impl<'a> Pipe<'a> {
        unsafe fn new<P: IntoRawHandle>(p: P, dst: &'a mut Vec<u8>) -> Pipe<'a> {
            Pipe {
                dst,
                pipe: NamedPipe::from_raw_handle(p.into_raw_handle()),
                overlapped: Overlapped::zero(),
                done: false,
            }
        }

        unsafe fn read(&mut self) -> io::Result<()> {
            let dst = slice_to_end(self.dst);
            match self.pipe.read_overlapped(dst, self.overlapped.raw()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                        self.done = true;
                        Ok(())
                    } else {
                        Err(e)
                    }
                }
            }
        }

        unsafe fn complete(&mut self, status: &CompletionStatus) {
            let prev = self.dst.len();
            self.dst.set_len(prev + status.bytes_transferred() as usize);
            if status.bytes_transferred() == 0 {
                self.done = true;
            }
        }
    }

    unsafe fn slice_to_end(v: &mut Vec<u8>) -> &mut [u8] {
        if v.capacity() == 0 {
            v.reserve(16);
        }
        if v.capacity() == v.len() {
            v.reserve(1);
        }
        slice::from_raw_parts_mut(v.as_mut_ptr().add(v.len()), v.capacity() - v.len())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_quote() {
        assert_eq!(&quote("simple"), "'simple'");
        assert_eq!(&quote("with'tick"), "'with'\"'\"'tick'");
    }

    #[test]
    fn test_split() {
        assert_eq!(
            split("simple whitespace split").unwrap().as_slice(),
            &["simple", "whitespace", "split"]
        );
        assert_eq!(
            split("'quoted split\\' with escapes' and \"double quotes\" \\\\")
                .unwrap()
                .as_slice(),
            &["quoted split\' with escapes", "and", "double quotes", "\\"]
        );
    }
}
