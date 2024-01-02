// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use anyhow::{bail, Result};
use std::{
    ffi::OsStr,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    sync::{Arc, RwLock},
    thread::spawn,
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const NEWLINE_BYTE: u8 = b'\n';

use tauri::async_runtime::{block_on as block_on_task, channel, Receiver, Sender};

use os_pipe::{pipe, PipeReader, PipeWriter};
use serde::Serialize;
use shared_child::SharedChild;
use tauri::utils::platform;

/// Payload for the [`CommandEvent::Terminated`] command event.
#[derive(Debug, Clone, Serialize)]
pub struct TerminatedPayload {
    /// Exit code of the process.
    pub code: Option<i32>,
    /// If the process was terminated by a signal, represents that signal.
    pub signal: Option<i32>,
}

/// A event sent to the command callback.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CommandEvent {
    /// Stderr bytes until a newline (\n) or carriage return (\r) is found.
    Stderr(Vec<u8>),
    /// Stdout bytes until a newline (\n) or carriage return (\r) is found.
    Stdout(Vec<u8>),
    /// An error happened waiting for the command to finish or converting the stdout/stderr bytes to an UTF-8 string.
    Error(String),
    /// Command process terminated.
    Terminated(TerminatedPayload),
}

/// The type to spawn commands.
#[derive(Debug)]
pub struct Command(StdCommand);

/// Spawned child process.
#[derive(Debug)]
pub struct CommandChild {
    inner: Arc<SharedChild>,
    stdin_writer: PipeWriter,
}

impl CommandChild {
    /// Writes to process stdin.
    pub fn write(&mut self, buf: &[u8]) -> Result<()> {
        self.stdin_writer.write_all(buf)?;
        Ok(())
    }

    /// Sends a kill signal to the child.
    pub fn kill(self) -> Result<()> {
        self.inner.kill()?;
        Ok(())
    }

    /// Returns the process pid.
    pub fn pid(&self) -> u32 {
        self.inner.id()
    }
}

/// Describes the result of a process after it has terminated.
#[derive(Debug)]
pub struct ExitStatus {
    code: Option<i32>,
}

impl ExitStatus {
    /// Returns the exit code of the process, if any.
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    /// Returns true if exit status is zero. Signal termination is not considered a success, and success is defined as a zero exit status.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// The output of a finished process.
#[derive(Debug)]
pub struct Output {
    /// The status (exit code) of the process.
    pub status: ExitStatus,
    /// The data that the process wrote to stdout.
    pub stdout: Vec<u8>,
    /// The data that the process wrote to stderr.
    pub stderr: Vec<u8>,
}

fn relative_command_path(command: &Path) -> Result<PathBuf> {
    match platform::current_exe()?.parent() {
        #[cfg(windows)]
        Some(exe_dir) => Ok(exe_dir.join(command).with_extension("exe")),
        #[cfg(not(windows))]
        Some(exe_dir) => Ok(exe_dir.join(command)),
        None => bail!("failed to get current exe dir"),
    }
}

impl From<Command> for StdCommand {
    fn from(cmd: Command) -> StdCommand {
        cmd.0
    }
}

impl Command {
    pub(crate) fn new<S: AsRef<OsStr>>(program: S) -> Self {
        let mut command = StdCommand::new(program);

        command.stdout(Stdio::piped());
        command.stdin(Stdio::piped());
        command.stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        Self(command)
    }

    pub(crate) fn new_sidecar<S: AsRef<Path>>(program: S) -> Result<Self> {
        Ok(Self::new(relative_command_path(program.as_ref())?))
    }

    /// Appends an argument to the command.
    #[must_use]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.0.arg(arg);
        self
    }

    /// Appends arguments to the command.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.0.args(args);
        self
    }

    /// Clears the entire environment map for the child process.
    #[must_use]
    pub fn env_clear(mut self) -> Self {
        self.0.env_clear();
        self
    }

    /// Inserts or updates an explicit environment variable mapping.
    #[must_use]
    pub fn env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.env(key, value);
        self
    }

    /// Adds or updates multiple environment variable mappings.
    #[must_use]
    pub fn envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.envs(envs);
        self
    }

    /// Sets the working directory for the child process.
    #[must_use]
    pub fn current_dir<P: AsRef<Path>>(mut self, current_dir: P) -> Self {
        self.0.current_dir(current_dir);
        self
    }

    /// Spawns the command.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tauri_plugin_shell::{process::CommandEvent, ShellExt};
    /// tauri::Builder::default()
    ///   .setup(|app| {
    ///     let handle = app.handle().clone();
    ///     tauri::async_runtime::spawn(async move {
    ///       let (mut rx, mut child) = handle.shell().command("cargo")
    ///         .args(["tauri", "dev"])
    ///         .spawn()
    ///         .expect("Failed to spawn cargo");
    ///
    ///       let mut i = 0;
    ///       while let Some(event) = rx.recv().await {
    ///         if let CommandEvent::Stdout(line) = event {
    ///           println!("got: {}", String::from_utf8(line).unwrap());
    ///           i += 1;
    ///           if i == 4 {
    ///             child.write("message from Rust\n".as_bytes()).unwrap();
    ///             i = 0;
    ///           }
    ///         }
    ///       }
    ///     });
    ///     Ok(())
    /// });
    /// ```
    pub fn spawn(self) -> Result<(Receiver<CommandEvent>, CommandChild)> {
        let mut command: StdCommand = self.into();
        let (stdout_reader, stdout_writer) = pipe()?;
        let (stderr_reader, stderr_writer) = pipe()?;
        let (stdin_reader, stdin_writer) = pipe()?;
        command.stdout(stdout_writer);
        command.stderr(stderr_writer);
        command.stdin(stdin_reader);

        let shared_child = SharedChild::spawn(&mut command)?;
        let child = Arc::new(shared_child);
        let child_ = child.clone();
        let guard = Arc::new(RwLock::new(()));

        let (tx, rx) = channel(1);

        spawn_pipe_reader(
            tx.clone(),
            guard.clone(),
            stdout_reader,
            CommandEvent::Stdout,
        );
        spawn_pipe_reader(
            tx.clone(),
            guard.clone(),
            stderr_reader,
            CommandEvent::Stderr,
        );

        spawn(move || {
            let _ = match child_.wait() {
                Ok(status) => {
                    let _l = guard.write().unwrap();
                    block_on_task(async move {
                        tx.send(CommandEvent::Terminated(TerminatedPayload {
                            code: status.code(),
                            #[cfg(windows)]
                            signal: None,
                            #[cfg(unix)]
                            signal: status.signal(),
                        }))
                        .await
                    })
                }
                Err(e) => {
                    let _l = guard.write().unwrap();
                    block_on_task(async move { tx.send(CommandEvent::Error(e.to_string())).await })
                }
            };
        });

        Ok((
            rx,
            CommandChild {
                inner: child,
                stdin_writer,
            },
        ))
    }

    /// Executes a command as a child process, waiting for it to finish and collecting its exit status.
    /// Stdin, stdout and stderr are ignored.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tauri_plugin_shell::ShellExt;
    /// tauri::Builder::default()
    ///   .setup(|app| {
    ///     let status = tauri::async_runtime::block_on(async move { app.shell().command("which").args(["ls"]).status().await.unwrap() });
    ///     println!("`which` finished with status: {:?}", status.code());
    ///     Ok(())
    ///   });
    /// ```
    pub async fn status(self) -> Result<ExitStatus> {
        let (mut rx, _child) = self.spawn()?;
        let mut code = None;
        #[allow(clippy::collapsible_match)]
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Terminated(payload) = event {
                code = payload.code;
            }
        }
        Ok(ExitStatus { code })
    }

    /// Executes the command as a child process, waiting for it to finish and collecting all of its output.
    /// Stdin is ignored.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tauri_plugin_shell::ShellExt;
    /// tauri::Builder::default()
    ///   .setup(|app| {
    ///     let output = tauri::async_runtime::block_on(async move { app.shell().command("echo").args(["TAURI"]).output().await.unwrap() });
    ///     assert!(output.status.success());
    ///     assert_eq!(String::from_utf8(output.stdout).unwrap(), "TAURI");
    ///     Ok(())
    ///   });
    /// ```
    pub async fn output(self) -> Result<Output> {
        let (mut rx, _child) = self.spawn()?;

        let mut code = None;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Terminated(payload) => {
                    code = payload.code;
                }
                CommandEvent::Stdout(line) => {
                    stdout.extend(line);
                    stdout.push(NEWLINE_BYTE);
                }
                CommandEvent::Stderr(line) => {
                    stderr.extend(line);
                    stderr.push(NEWLINE_BYTE);
                }
                CommandEvent::Error(_) => {}
            }
        }
        Ok(Output {
            status: ExitStatus { code },
            stdout,
            stderr,
        })
    }
}

fn spawn_pipe_reader<F: Fn(Vec<u8>) -> CommandEvent + Send + Copy + 'static>(
    tx: Sender<CommandEvent>,
    guard: Arc<RwLock<()>>,
    pipe_reader: PipeReader,
    wrapper: F,
) {
    spawn(move || {
        let _lock = guard.read().unwrap();
        let mut reader = BufReader::new(pipe_reader);

        loop {
            let mut buf = Vec::new();
            match tauri::utils::io::read_line(&mut reader, &mut buf) {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }
                    let tx_ = tx.clone();
                    let _ = block_on_task(async move { tx_.send(wrapper(buf)).await });
                }
                Err(e) => {
                    let tx_ = tx.clone();
                    let _ =
                        block_on_task(
                            async move { tx_.send(CommandEvent::Error(e.to_string())).await },
                        );
                }
            }
        }
    });
}

// tests for the commands functions.
#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn test_cmd_spawn_output() {
        let cmd = Command::new("cat").args(["test/test.txt"]);
        let (mut rx, _) = cmd.spawn().unwrap();

        tauri::async_runtime::block_on(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(payload) => {
                        assert_eq!(payload.code, Some(0));
                    }
                    CommandEvent::Stdout(line) => {
                        assert_eq!(String::from_utf8(line).unwrap(), "This is a test doc!");
                    }
                    _ => {}
                }
            }
        });
    }

    #[cfg(not(windows))]
    #[test]
    fn test_cmd_spawn_raw_output() {
        let cmd = Command::new("cat").args(["test/test.txt"]);
        let (mut rx, _) = cmd.spawn().unwrap();

        tauri::async_runtime::block_on(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(payload) => {
                        assert_eq!(payload.code, Some(0));
                    }
                    CommandEvent::Stdout(line) => {
                        assert_eq!(String::from_utf8(line).unwrap(), "This is a test doc!");
                    }
                    _ => {}
                }
            }
        });
    }

    #[cfg(not(windows))]
    #[test]
    // test the failure case
    fn test_cmd_spawn_fail() {
        let cmd = Command::new("cat").args(["test/"]);
        let (mut rx, _) = cmd.spawn().unwrap();

        tauri::async_runtime::block_on(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(payload) => {
                        assert_eq!(payload.code, Some(1));
                    }
                    CommandEvent::Stderr(line) => {
                        assert_eq!(
                            String::from_utf8(line).unwrap(),
                            "cat: test/: Is a directory\n"
                        );
                    }
                    _ => {}
                }
            }
        });
    }

    #[cfg(not(windows))]
    #[test]
    // test the failure case (raw encoding)
    fn test_cmd_spawn_raw_fail() {
        let cmd = Command::new("cat").args(["test/"]);
        let (mut rx, _) = cmd.spawn().unwrap();

        tauri::async_runtime::block_on(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Terminated(payload) => {
                        assert_eq!(payload.code, Some(1));
                    }
                    CommandEvent::Stderr(line) => {
                        assert_eq!(
                            String::from_utf8(line).unwrap(),
                            "cat: test/: Is a directory\n"
                        );
                    }
                    _ => {}
                }
            }
        });
    }

    #[cfg(not(windows))]
    #[test]
    fn test_cmd_output_output() {
        let cmd = Command::new("cat").args(["test/test.txt"]);
        let output = tauri::async_runtime::block_on(cmd.output()).unwrap();

        assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "This is a test doc!\n"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn test_cmd_output_output_fail() {
        let cmd = Command::new("cat").args(["test/"]);
        let output = tauri::async_runtime::block_on(cmd.output()).unwrap();

        assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "cat: test/: Is a directory\n\n"
        );
    }
}
