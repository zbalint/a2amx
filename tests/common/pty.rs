//! PTY harness for black-box tests of the `a2amx` binary.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use a2amx::emulator::{Emulator, Size};
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::{self, ChildEvent, EventedPty};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::termios::{Winsize, tcsetwinsize};

const COLS: u16 = 80;
const ROWS: u16 = 24;
const READ_CHUNK: usize = 8192;

#[derive(Debug)]
enum ReadEvent {
    Bytes(Vec<u8>),
    Eof,
}

/// A real alacritty PTY running the checked-in `a2amx` executable.
///
/// The reader owns a blocking clone of the PTY master and feeds every byte to
/// an A2AMX emulator, so assertions observe the same screen model as a human
/// terminal rather than matching an implementation's escape sequence layout.
pub struct PtyHarness {
    pty: Option<Arc<Mutex<tty::Pty>>>,
    writer: Option<File>,
    events: mpsc::Receiver<ReadEvent>,
    reader: Option<JoinHandle<()>>,
    emulator: Emulator,
    raw_output: Vec<u8>,
    eof: bool,
    exit_code: Option<i32>,
}

impl PtyHarness {
    pub fn spawn(args: &[String], home: &Path) -> anyhow::Result<Self> {
        Self::spawn_with_env(args, home, &[])
    }

    pub fn spawn_with_env(
        args: &[String],
        home: &Path,
        environment: &[(&str, &str)],
    ) -> anyhow::Result<Self> {
        Self::spawn_program(env!("CARGO_BIN_EXE_a2amx"), args, home, environment)
    }

    pub fn spawn_program(
        program: &str,
        args: &[String],
        home: &Path,
        environment: &[(&str, &str)],
    ) -> anyhow::Result<Self> {
        let mut options = tty::Options {
            shell: Some(tty::Shell::new(program.to_owned(), args.to_vec())),
            ..tty::Options::default()
        };
        options
            .env
            .insert("A2AMX_HOME".to_owned(), home.to_string_lossy().into_owned());
        for (key, value) in environment {
            options.env.insert((*key).to_owned(), (*value).to_owned());
        }
        let pty = tty::new(
            &options,
            WindowSize {
                num_lines: ROWS,
                num_cols: COLS,
                cell_width: 0,
                cell_height: 0,
            },
            0,
        )?;

        let master = pty.file().try_clone()?;
        let writer = pty.file().try_clone()?;
        let flags = fcntl_getfl(&master)?;
        fcntl_setfl(&master, flags & !OFlags::NONBLOCK)?;
        // The PTY's open file description is shared by these clones. Clearing
        // O_NONBLOCK on the reader also makes writes deterministic for the
        // small chunks this harness sends.
        let writer_flags = fcntl_getfl(&writer)?;
        fcntl_setfl(&writer, writer_flags & !OFlags::NONBLOCK)?;

        let (tx, events) = mpsc::channel();
        let reader = thread::Builder::new()
            .name("a2amx-test-pty-reader".to_owned())
            .spawn(move || read_pty(master, tx))?;

        Ok(Self {
            pty: Some(Arc::new(Mutex::new(pty))),
            writer: Some(writer),
            events,
            reader: Some(reader),
            emulator: Emulator::new(Size {
                cols: COLS,
                rows: ROWS,
            }),
            raw_output: Vec::new(),
            eof: false,
            exit_code: None,
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("PTY writer is closed"))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        let pty = self
            .pty
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("PTY is closed"))?;
        let pty = pty
            .lock()
            .map_err(|_| anyhow::anyhow!("PTY lock poisoned"))?;
        tcsetwinsize(
            pty.file(),
            Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        drop(pty);
        self.emulator.resize(Size { cols, rows });
        Ok(())
    }

    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_ready();
            if self.screen_text().contains(text) {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow::anyhow!(
                    "timed out waiting for {text:?}; screen was {:?}",
                    self.screen_text()
                ));
            }
            self.recv_one((deadline - now).min(Duration::from_millis(50)));
        }
    }

    pub fn screen_text(&mut self) -> String {
        self.drain_ready();
        let screen = self.emulator.screen();
        let mut text = String::new();
        for row in 0..screen.size.rows {
            let start = usize::from(row) * usize::from(screen.size.cols);
            let end = start + usize::from(screen.size.cols);
            let line: String = screen.cells[start..end]
                .iter()
                .map(|cell| cell.ch)
                .collect();
            text.push_str(line.trim_end_matches(' '));
            if row + 1 < screen.size.rows {
                text.push('\n');
            }
        }
        text
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> anyhow::Result<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_ready();
            self.poll_child();
            if self.exit_code.is_some() && self.eof {
                return self
                    .exit_code
                    .ok_or_else(|| anyhow::anyhow!("PTY child exit status disappeared"));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow::anyhow!("timed out waiting for PTY child exit"));
            }
            self.recv_one((deadline - now).min(Duration::from_millis(50)));
        }
    }

    pub fn raw_output(&mut self) -> &[u8] {
        self.drain_ready();
        &self.raw_output
    }

    fn recv_one(&mut self, timeout: Duration) {
        if let Ok(event) = self.events.recv_timeout(timeout) {
            self.apply(event);
        }
        self.poll_child();
    }

    fn drain_ready(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.apply(event);
        }
        self.poll_child();
    }

    fn apply(&mut self, event: ReadEvent) {
        match event {
            ReadEvent::Bytes(bytes) => {
                self.raw_output.extend_from_slice(&bytes);
                let _ = self.emulator.feed(&bytes);
            }
            ReadEvent::Eof => self.eof = true,
        }
    }

    fn poll_child(&mut self) {
        let Some(pty) = self.pty.as_ref() else {
            return;
        };
        let Ok(mut pty) = pty.lock() else {
            return;
        };
        if let Some(ChildEvent::Exited(status)) = pty.next_child_event() {
            self.exit_code = status.map(|status| {
                if let Some(code) = status.code() {
                    code
                } else {
                    128 + status.signal().unwrap_or(0)
                }
            });
        }
    }
}

impl Drop for PtyHarness {
    fn drop(&mut self) {
        drop(self.writer.take());
        if let Some(pty) = self.pty.take() {
            if let Ok(pty) = Arc::try_unwrap(pty) {
                if let Ok(pty) = pty.into_inner() {
                    drop(pty);
                }
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_pty(mut master: File, tx: mpsc::Sender<ReadEvent>) {
    let mut bytes = [0u8; READ_CHUNK];
    loop {
        match master.read(&mut bytes) {
            Ok(0) => {
                let _ = tx.send(ReadEvent::Eof);
                return;
            }
            Ok(len) => {
                if tx.send(ReadEvent::Bytes(bytes[..len].to_vec())).is_err() {
                    return;
                }
            }
            Err(err) if err.raw_os_error() == Some(5) => {
                let _ = tx.send(ReadEvent::Eof);
                return;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = tx.send(ReadEvent::Eof);
                return;
            }
        }
    }
}
