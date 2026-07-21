//! Typed wrapper around the Apple `container` CLI (verified against 1.1.0).

use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot run `{binary}`: {source}")]
    Spawn {
        binary: String,
        source: std::io::Error,
    },
    #[error("container services are not running: {stderr}")]
    RuntimeDown { stderr: String },
    #[error("not found: {stderr}")]
    NotFound { stderr: String },
    #[error("`{command}` failed ({status}): {stderr}")]
    Failed {
        command: String,
        status: ExitStatus,
        stderr: String,
    },
    #[error("cannot parse output of `{command}`: {source}")]
    Parse {
        command: String,
        source: serde_json::Error,
    },
}

#[derive(Debug, Deserialize)]
pub struct SystemStatus {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub id: String,
    pub status: MachineStatus,
    pub cpus: u32,
    pub memory: u64,
    pub image: MachineImage,
    pub ip_address: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MachineImage {
    pub reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MachineStatus {
    Running,
    Stopped,
    #[serde(other)]
    Unknown,
}

pub struct Runtime {
    binary: String,
}

impl Runtime {
    pub fn new(config: &config::Runtime) -> Self {
        Self {
            binary: config.container_binary.clone(),
        }
    }

    pub fn system_status(&self) -> Result<SystemStatus, Error> {
        self.output_json(&["system", "status", "--format", "json"])
    }

    pub fn system_start(&self) -> Result<(), Error> {
        self.output(&["system", "start"]).map(|_| ())
    }

    pub fn machine_inspect(&self, name: &str) -> Result<Option<MachineInfo>, Error> {
        let result: Result<Vec<MachineInfo>, Error> =
            self.recovering(|rt| rt.output_json(&["machine", "inspect", name]));
        match result {
            Ok(machines) => Ok(machines.into_iter().next()),
            Err(Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn machine_create(&self, machine: &config::Machine, image_tag: &str) -> Result<(), Error> {
        self.recovering(|rt| {
            rt.streamed(&[
                "machine",
                "create",
                "--name",
                &machine.name,
                "--cpus",
                &machine.cpus.to_string(),
                "--memory",
                &machine.memory,
                "--home-mount",
                "none",
                image_tag,
            ])
        })
    }

    pub fn machine_set(&self, machine: &config::Machine) -> Result<(), Error> {
        self.recovering(|rt| {
            rt.output(&[
                "machine",
                "set",
                "--name",
                &machine.name,
                &format!("cpus={}", machine.cpus),
                &format!("memory={}", machine.memory),
            ])
            .map(|_| ())
        })
    }

    /// There is no `machine start`; running a trivial command boots the
    /// machine if necessary.
    pub fn machine_boot(&self, name: &str) -> Result<(), Error> {
        self.recovering(|rt| {
            rt.output(&["machine", "run", "--root", "--name", name, "true"])
                .map(|_| ())
        })
    }

    pub fn machine_stop(&self, name: &str) -> Result<(), Error> {
        self.recovering(|rt| rt.output(&["machine", "stop", name]).map(|_| ()))
    }

    pub fn machine_delete(&self, name: &str) -> Result<(), Error> {
        self.recovering(|rt| rt.output(&["machine", "delete", name]).map(|_| ()))
    }

    pub fn image_exists(&self, tag: &str) -> Result<bool, Error> {
        match self.recovering(|rt| rt.output(&["image", "inspect", tag])) {
            Ok(_) => Ok(true),
            Err(Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn build_image(
        &self,
        tag: &str,
        containerfile: &Path,
        context: &Path,
    ) -> Result<(), Error> {
        self.recovering(|rt| {
            rt.streamed(&[
                "build",
                "--file",
                &containerfile.display().to_string(),
                "--tag",
                tag,
                &context.display().to_string(),
            ])
        })
    }

    /// The one shared recovery routine: if a call fails because the
    /// `container` services are down, start them and retry once.
    fn recovering<T>(&self, f: impl Fn(&Self) -> Result<T, Error>) -> Result<T, Error> {
        match f(self) {
            Err(Error::RuntimeDown { .. }) => {
                crate::ui::step("starting container services");
                self.system_start()?;
                f(self)
            }
            result => result,
        }
    }

    /// Run to completion, capturing output and classifying failures.
    fn output(&self, args: &[&str]) -> Result<Output, Error> {
        let output = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| Error::Spawn {
                binary: self.binary.clone(),
                source,
            })?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(classify(self.command_line(args), &output))
        }
    }

    /// Run with stdout/stderr streamed to the user, for long operations
    /// (image builds, machine creation) whose progress matters.
    fn streamed(&self, args: &[&str]) -> Result<(), Error> {
        let status = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .status()
            .map_err(|source| Error::Spawn {
                binary: self.binary.clone(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Failed {
                command: self.command_line(args),
                status,
                stderr: "see output above".into(),
            })
        }
    }

    fn output_json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, Error> {
        let output = self.output(args)?;
        serde_json::from_slice(&output.stdout).map_err(|source| Error::Parse {
            command: self.command_line(args),
            source,
        })
    }

    fn command_line(&self, args: &[&str]) -> String {
        format!("{} {}", self.binary, args.join(" "))
    }
}

fn classify(command: String, output: &Output) -> Error {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let lower = stderr.to_lowercase();
    if lower.contains("notfound") || lower.contains("not found") {
        Error::NotFound { stderr }
    } else if lower.contains("xpc") || lower.contains("apiserver") || lower.contains("interrupted")
    {
        Error::RuntimeDown { stderr }
    } else {
        Error::Failed {
            command,
            status: output.status,
            stderr,
        }
    }
}
