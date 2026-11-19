//! The own password, changed as GNOME Settings changes it (run-passwd.c): `passwd` (through `tools::system_tool`)
//! answers PAM's prompts. It runs on a pseudo-terminal of its own (setsid, the PTY as its controlling terminal) with
//! LC_ALL=C, so the prompts and messages are PAM's English ones; [`Transcript`] reads them and says what to type.
//! The passwords never reach a log: PAM turns echo off at its prompts, and the transcript stays in this process.
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What to do next, from the transcript so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Type the current password.
    Current,
    /// Type the new password.
    New,
    /// Type the new password again.
    Retype,
    /// passwd has finished (or must be stopped) with this outcome.
    Done(Outcome),
}

/// How a password change ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Changed,
    /// PAM refused the current password.
    WrongCurrent,
    /// PAM's quality checks refused the new password (pam_pwquality, pam_unix's length and history rules).
    TooWeak,
    /// The minimum password age has not passed.
    TooSoon,
    /// passwd ended without a change and without one of the reasons above.
    Unchanged,
}

/// PAM's side of the conversation, read as it arrives (the prompts end without a newline).
#[derive(Default)]
pub(crate) struct Transcript {
    text: String,
    consumed: usize,
    current: u32,
    new: u32,
    retype: u32,
    weak: bool,
    done: Option<Outcome>,
}

impl Transcript {
    /// Add output; the steps it asks for, in order.
    pub(crate) fn feed(&mut self, chunk: &str) -> Vec<Step> {
        self.text.push_str(chunk);
        let mut steps = Vec::new();
        while self.done.is_none() {
            let rest = &self.text[self.consumed..];
            if let Some(end) = rest.find('\n') {
                let line = rest[..end].trim().to_ascii_lowercase();
                self.consumed += end + 1;
                if let Some(outcome) = self.message(&line) {
                    self.done = Some(outcome);
                    steps.push(Step::Done(outcome));
                }
                continue;
            }
            let prompt = rest.trim().to_ascii_lowercase();
            if !prompt.ends_with(':') {
                break;
            }
            self.consumed = self.text.len();
            let step = self.prompt(&prompt);
            if let Step::Done(outcome) = step {
                self.done = Some(outcome);
            }
            steps.push(step);
        }
        steps
    }

    /// The outcome when passwd's output ended without a final message (it exited).
    pub(crate) fn finish(&mut self, exit_ok: bool) -> Outcome {
        if let Some(done) = self.done {
            return done;
        }
        let rest = self.text[self.consumed..].trim().to_ascii_lowercase();
        if let Some(outcome) = self.message(&rest) {
            return outcome;
        }
        match (exit_ok, self.retype > 0) {
            (true, true) => Outcome::Changed,
            _ if self.weak => Outcome::TooWeak,
            _ => Outcome::Unchanged,
        }
    }

    fn message(&mut self, line: &str) -> Option<Outcome> {
        if line.is_empty() {
            return None;
        }
        if line.contains("password updated successfully")
            || line.contains("tokens updated successfully")
        {
            return Some(Outcome::Changed);
        }
        if line.contains("you must wait longer") {
            return Some(Outcome::TooSoon);
        }
        if line.contains("authentication token manipulation error")
            || line.contains("authentication failure")
        {
            // before the new password was asked for, PAM refused the current one
            return Some(if self.new == 0 {
                Outcome::WrongCurrent
            } else if self.weak {
                Outcome::TooWeak
            } else {
                Outcome::Unchanged
            });
        }
        const WEAK: &[&str] = &[
            "bad password",
            "you must choose a longer password",
            "you must choose a shorter password",
            "too simple",
            "too similar",
            "too short",
            "palindrome",
            "case changes only",
            "already used",
            "is the same as the old one",
            "no password has been supplied",
            "dictionary",
        ];
        if WEAK.iter().any(|w| line.contains(w)) {
            self.weak = true;
            return None;
        }
        if line.contains("exhausted maximum number of retries")
            || line.contains("password unchanged")
            || line.contains("has not been changed")
            || line.contains("is unchanged")
        {
            return Some(if self.weak {
                Outcome::TooWeak
            } else if self.new == 0 {
                Outcome::WrongCurrent
            } else {
                Outcome::Unchanged
            });
        }
        if line.contains("passwords do not match") {
            return Some(Outcome::Unchanged);
        }
        None
    }

    fn prompt(&mut self, prompt: &str) -> Step {
        if prompt.contains("current")
            || (prompt.contains("password")
                && !prompt.contains("new")
                && self.new == 0
                && self.current == 0)
        {
            // a second "Current password:" means the first was refused
            if self.current > 0 {
                return Step::Done(Outcome::WrongCurrent);
            }
            self.current += 1;
            return Step::Current;
        }
        if prompt.contains("retype") || prompt.contains("re-enter") || prompt.contains("again") {
            if self.retype > 0 {
                return Step::Done(Outcome::Unchanged);
            }
            self.retype += 1;
            return Step::Retype;
        }
        if prompt.contains("new") && prompt.contains("password") {
            // PAM asks again after refusing the new password: stop there instead of retrying
            if self.new > 0 {
                return Step::Done(if self.weak {
                    Outcome::TooWeak
                } else {
                    Outcome::Unchanged
                });
            }
            self.new += 1;
            return Step::New;
        }
        Step::Done(Outcome::Unchanged)
    }
}

/// How long the whole change may take (PAM waits two seconds after a failure; a hung helper is stopped).
const OVERALL: Duration = Duration::from_secs(60);

/// A pseudo-terminal pair (openpty).
fn pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (-1, -1);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) })
}

fn write_all(fd: &OwnedFd, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let n = unsafe { libc::write(fd.as_raw_fd(), data.as_ptr().cast(), data.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        data = &data[n as usize..];
    }
    Ok(())
}

/// Run `program` (passwd) on its own pseudo-terminal and answer its prompts with `current` and `new`.
pub(crate) fn change(program: &Path, current: &str, new: &str) -> io::Result<Outcome> {
    let (master, slave) = pty()?;
    let mut command = Command::new(program);
    command
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    unsafe {
        command.pre_exec(|| {
            // a session of its own, with the PTY as its controlling terminal (PAM reads the terminal)
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    drop(command); // closes the parent's copies of the slave side: EIO on the master once passwd has exited
    let end = Instant::now() + OVERALL;
    let mut transcript = Transcript::default();
    let mut buf = [0u8; 4096];
    let mut outcome = None;
    while outcome.is_none() && Instant::now() < end {
        let mut pfd = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let left = end
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(1000) as i32;
        let n = unsafe { libc::poll(&mut pfd, 1, left) };
        if n < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            if child.try_wait()?.is_some() {
                break;
            }
            continue;
        }
        let read = unsafe { libc::read(master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if read <= 0 {
            break; // EIO: the slave side is closed, passwd has exited
        }
        for step in transcript.feed(&String::from_utf8_lossy(&buf[..read as usize])) {
            match step {
                Step::Current => write_all(&master, format!("{current}\n").as_bytes())?,
                Step::New | Step::Retype => write_all(&master, format!("{new}\n").as_bytes())?,
                Step::Done(o) => outcome = Some(o),
            }
        }
    }
    // a refused password is not retried: passwd is stopped where it would ask again
    let wait_end = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if outcome.is_some_and(|o| o != Outcome::Changed)
                || Instant::now() >= wait_end
                || Instant::now() >= end =>
            {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let exit_ok = status.is_some_and(|s| s.success());
    Ok(match outcome {
        Some(Outcome::Changed) if !exit_ok && status.is_some() => Outcome::Unchanged,
        Some(o) => o,
        None => transcript.finish(exit_ok),
    })
}
