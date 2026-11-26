//! Reading a page for the browser on a thread of its own, so that the page
//! keeps up with the rest of the screen while the words come in, and the
//! reading stops when it is asked to.

use anyhow::{Context, Result};
use skyward_net::page::Page;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// How a page ended: read, or why it was not.
pub enum Ended {
    Read(Page),
    Failed(String),
    Stopped,
}

pub struct Worker {
    ended: Receiver<Ended>,
    /// Readable once the reading has ended.
    wake: OwnedFd,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(url: String, servers: skyward_net::http::Servers) -> Result<Worker> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error()).context("make the browser thread's wake pipe");
        }
        let (wake, signal) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, ended) = mpsc::channel();
        let stop_as = stop.clone();
        let thread = thread::Builder::new()
            .name("browse".into())
            .spawn(move || {
                let told = match skyward_net::page::get(servers, &url) {
                    Ok(page) => Ended::Read(page),
                    Err(e) if stop_as.load(Ordering::Relaxed) => {
                        let _ = e;
                        Ended::Stopped
                    }
                    Err(e) => Ended::Failed(format!("{e:#}")),
                };
                let _ = tx.send(told);
                unsafe { libc::write(signal.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
            })
            .context("start the browser thread")?;
        Ok(Worker { ended, wake, stop, thread: Some(thread) })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// How the reading ended, once it has.
    pub fn ended(&self) -> Option<Ended> {
        let mut drain = [0u8; 8];
        while unsafe { libc::read(self.wake.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        self.ended.try_recv().ok()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}