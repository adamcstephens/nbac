//! Typed wrapper around the Apple `container` CLI, pinned to one supported
//! release series at a time.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config;

/// The `container` release series nbac supports; behavior of the CLI (argument
/// shapes, error strings, kernel handling) shifts between minor releases, but
/// patch releases within a series stay compatible.
pub const SUPPORTED_SERIES: &str = "1.2";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot run `{binary}`: {source}")]
    Spawn {
        binary: String,
        source: std::io::Error,
    },
    #[error("unsupported `container` version {found}; nbac supports {SUPPORTED_SERIES}.x")]
    UnsupportedVersion { found: String },
    #[error("container services are not running: {stderr}")]
    RuntimeDown { stderr: String },
    #[error("no default kernel is configured: {stderr}")]
    NoDefaultKernel { stderr: String },
    #[error("not found: {stderr}")]
    NotFound { stderr: String },
    #[error("machine is still booting: {stderr}")]
    Booting { stderr: String },
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
    asuser: Option<u32>,
}

impl Runtime {
    pub fn new(config: &config::Config) -> Self {
        Self {
            binary: resolve(&config.runtime.container_binary),
            asuser: asuser_uid(&config.state.dir),
        }
    }

    /// The `container` runtime is a per-user launchd agent reached over XPC,
    /// but nix-daemon runs distributed builds (and thus the SSH
    /// ProxyCommand) as root. Re-enter the owning user's bootstrap namespace
    /// and drop to that user; the state directory's owner is that user.
    fn command(&self, args: &[&str]) -> Command {
        match self.asuser {
            Some(uid) => {
                let mut cmd = Command::new("/bin/launchctl");
                cmd.args(["asuser", &uid.to_string()])
                    .args([
                        "/usr/bin/sudo",
                        "--user",
                        &format!("#{uid}"),
                        "--set-home",
                        "--non-interactive",
                        "--",
                    ])
                    .arg(&self.binary)
                    .args(args);
                cmd
            }
            None => {
                let mut cmd = Command::new(&self.binary);
                cmd.args(args);
                cmd
            }
        }
    }

    /// Preflight: `--version` is answered by the CLI itself (no services
    /// needed), so a mismatch surfaces before anything touches state.
    pub fn check_version(&self) -> Result<(), Error> {
        let output = self.output(&["--version"])?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        match parse_version(&stdout) {
            Some(found) if series(found) == SUPPORTED_SERIES => Ok(()),
            found => Err(Error::UnsupportedVersion {
                found: found.unwrap_or(stdout.trim()).to_string(),
            }),
        }
    }

    pub fn system_status(&self) -> Result<SystemStatus, Error> {
        self.output_json(&["system", "status", "--format", "json"])
    }

    pub fn system_start(&self) -> Result<(), Error> {
        self.output(&["system", "start"]).map(|_| ())
    }

    /// Download and install the default kernel builds and machines boot
    /// with. Streamed: it fetches a multi-hundred-megabyte archive.
    fn kernel_set_recommended(&self) -> Result<(), Error> {
        self.streamed(&["system", "kernel", "set", "--recommended"])
    }

    pub fn machine_inspect(&self, name: &str) -> Result<Option<MachineInfo>, Error> {
        first_machine(self.recovering(|rt| rt.output_json(&["machine", "inspect", name])))
    }

    /// Inspect without the service-start recovery: read-only probes must not
    /// mutate runtime state.
    pub fn machine_inspect_probe(&self, name: &str) -> Result<Option<MachineInfo>, Error> {
        first_machine(self.output_json(&["machine", "inspect", name]))
    }

    pub fn machine_create(&self, machine: &config::Machine, image_tag: &str) -> Result<(), Error> {
        let args = create_args(machine, image_tag);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        self.recovering(|rt| rt.streamed(&argv))
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
        self.while_booting(|rt| {
            rt.recovering(|rt| {
                rt.output(&["machine", "run", "--root", "--name", name, "true"])
                    .map(|_| ())
            })
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
        arch: &str,
        containerfile: &Path,
        context: &Path,
        build_args: &[(&str, String)],
    ) -> Result<(), Error> {
        let containerfile = containerfile.display().to_string();
        let context = context.display().to_string();
        let mut args = vec![
            "build",
            "--file",
            &containerfile,
            "--arch",
            arch,
            "--tag",
            tag,
        ];
        let pairs: Vec<String> = build_args
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        for pair in &pairs {
            args.push("--build-arg");
            args.push(pair);
        }
        args.push(&context);
        self.recovering(|rt| rt.streamed(&args))
    }

    /// Run a shell script inside the machine, fed through stdin: the one
    /// guest exec the cold path uses to inject keys and settings. Returns
    /// the script's captured stdout.
    pub fn machine_run_piped(&self, name: &str, script: &str) -> Result<String, Error> {
        self.while_booting(|rt| rt.machine_run_piped_once(name, script))
    }

    fn machine_run_piped_once(&self, name: &str, script: &str) -> Result<String, Error> {
        self.recovering(|rt| {
            let args = [
                "machine",
                "run",
                "--root",
                "--name",
                name,
                "--interactive",
                "--",
                "sh",
            ];
            let mut child = rt
                .command(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|source| Error::Spawn {
                    binary: rt.binary.clone(),
                    source,
                })?;
            // A write error means the guest shell died early; the exit
            // status and stderr below carry the diagnosis.
            let _ = child.stdin.take().unwrap().write_all(script.as_bytes());
            let output = child.wait_with_output().map_err(|source| Error::Spawn {
                binary: rt.binary.clone(),
                source,
            })?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            } else {
                Err(classify(rt.command_line(&args), &output))
            }
        })
    }

    /// The one shared recovery routine: if a call fails because the
    /// `container` services are down or no default kernel is installed
    /// (fresh 1.2.x installs ship without one), fix that and retry, each
    /// remedy at most once.
    fn recovering<T>(&self, f: impl Fn(&Self) -> Result<T, Error>) -> Result<T, Error> {
        let mut started = false;
        let mut kernel_set = false;
        loop {
            match f(self) {
                Err(Error::RuntimeDown { .. }) if !started => {
                    started = true;
                    crate::ui::step("starting container services");
                    self.system_start()?;
                }
                Err(Error::NoDefaultKernel { .. }) if !kernel_set => {
                    kernel_set = true;
                    crate::ui::step("installing the recommended default kernel");
                    self.kernel_set_recommended()?;
                }
                result => return result,
            }
        }
    }

    /// Bounded backoff for execs that race a machine boot.
    fn while_booting<T>(&self, f: impl Fn(&Self) -> Result<T, Error>) -> Result<T, Error> {
        let mut delay = std::time::Duration::from_millis(200);
        for _ in 0..8 {
            match f(self) {
                Err(Error::Booting { .. }) => {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(std::time::Duration::from_secs(2));
                }
                result => return result,
            }
        }
        f(self)
    }

    /// Run to completion, capturing output and classifying failures.
    fn output(&self, args: &[&str]) -> Result<Output, Error> {
        let output = self
            .command(args)
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
    /// (image builds, machine creation) whose progress matters. Stderr is
    /// teed while streaming so failures can still be classified.
    fn streamed(&self, args: &[&str]) -> Result<(), Error> {
        let mut child = self
            .command(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Spawn {
                binary: self.binary.clone(),
                source,
            })?;
        let mut pipe = child.stderr.take().unwrap();
        let tee = std::thread::spawn(move || {
            let mut captured = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = std::io::stderr().write_all(&buf[..n]);
                captured.extend_from_slice(&buf[..n]);
            }
            captured
        });
        let status = child.wait().map_err(|source| Error::Spawn {
            binary: self.binary.clone(),
            source,
        })?;
        let stderr = tee.join().unwrap_or_default();
        if status.success() {
            return Ok(());
        }
        let output = Output {
            status,
            stdout: Vec::new(),
            stderr,
        };
        // The user already watched the output scroll by; don't repeat it in
        // the generic failure message.
        Err(match classify(self.command_line(args), &output) {
            Error::Failed {
                command, status, ..
            } => Error::Failed {
                command,
                status,
                stderr: "see output above".into(),
            },
            classified => classified,
        })
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

/// Argument vector for `container machine create`. Nested virtualization adds
/// `--virtualization` and a `--kernel` pointing at a KVM-enabled image; the
/// image reference stays last, after every option.
fn create_args(machine: &config::Machine, image_tag: &str) -> Vec<String> {
    let mut args = vec![
        "machine".into(),
        "create".into(),
        "--name".into(),
        machine.name.clone(),
        "--cpus".into(),
        machine.cpus.to_string(),
        "--memory".into(),
        machine.memory.clone(),
        "--arch".into(),
        machine.arch().into(),
        "--home-mount".into(),
        "none".into(),
    ];
    if machine.virtualization {
        args.push("--virtualization".into());
    }
    if let Some(kernel) = &machine.kernel {
        args.push("--kernel".into());
        args.push(kernel.display().to_string());
    }
    args.push(image_tag.into());
    args
}

fn first_machine(result: Result<Vec<MachineInfo>, Error>) -> Result<Option<MachineInfo>, Error> {
    match result {
        Ok(machines) => Ok(machines.into_iter().next()),
        Err(Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn asuser_uid(state_dir: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    if !rustix::process::geteuid().is_root() {
        return None;
    }
    let uid = std::fs::metadata(state_dir).ok()?.uid();
    (uid != 0).then_some(uid)
}

/// sudo's env_reset and the daemon's minimal PATH both lose the caller's
/// PATH, so pin the binary to an absolute path up front.
fn resolve(binary: &str) -> String {
    if binary.contains('/') {
        return binary.into();
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
        .map(|found| found.to_string_lossy().into_owned())
        .unwrap_or_else(|| binary.into())
}

/// `container --version` prints "container CLI version 1.2.1 (build: …)".
fn parse_version(stdout: &str) -> Option<&str> {
    let mut tokens = stdout.split_whitespace();
    tokens.find(|token| *token == "version")?;
    tokens.next()
}

/// The major.minor prefix of a version, which is what compatibility turns on.
fn series(version: &str) -> &str {
    match version.match_indices('.').nth(1) {
        Some((dot, _)) => &version[..dot],
        None => version,
    }
}

fn classify(command: String, output: &Output) -> Error {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let lower = stderr.to_lowercase();
    if lower.contains("default kernel not configured") {
        Error::NoDefaultKernel { stderr }
    } else if lower.contains("notfound") || lower.contains("not found") {
        Error::NotFound { stderr }
    } else if lower.contains("inappropriate ioctl") {
        // `machine run` against a machine that is mid-boot fails with this
        // until the guest console is up (observed on `container` 1.1.0).
        Error::Booting { stderr }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Machine;

    #[test]
    fn parses_container_version() {
        assert_eq!(
            parse_version("container CLI version 1.2.1 (build: release, commit: unspeci)\n"),
            Some("1.2.1")
        );
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn series_matches_patch_releases_only() {
        assert_eq!(series("1.2.0"), SUPPORTED_SERIES);
        assert_eq!(series("1.2.1"), SUPPORTED_SERIES);
        assert_ne!(series("1.3.0"), SUPPORTED_SERIES);
        assert_ne!(series("1.20.0"), SUPPORTED_SERIES);
    }

    #[test]
    fn classifies_missing_default_kernel_before_notfound() {
        use std::os::unix::process::ExitStatusExt;
        let output = Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: br#"Error: notFound: "default kernel not configured for architecture arm64, please use the `container system kernel set` command to configure it""#.to_vec(),
        };
        let error = classify("container build".into(), &output);
        assert!(matches!(error, Error::NoDefaultKernel { .. }));
    }

    #[test]
    fn create_args_default_has_no_virtualization() {
        let args = create_args(&Machine::default(), "nbac-builder:abc");
        assert!(!args.iter().any(|a| a == "--virtualization"));
        assert!(!args.iter().any(|a| a == "--kernel"));
        let arch = args.iter().position(|a| a == "--arch").unwrap();
        assert_eq!(args[arch + 1], "arm64");
        assert_eq!(args.last().unwrap(), "nbac-builder:abc");
    }

    #[test]
    fn create_args_rosetta_selects_amd64() {
        let machine = Machine {
            rosetta: true,
            ..Machine::default()
        };
        let args = create_args(&machine, "nbac-builder:abc");
        let arch = args.iter().position(|a| a == "--arch").unwrap();
        assert_eq!(args[arch + 1], "amd64");
        assert_eq!(args.last().unwrap(), "nbac-builder:abc");
    }

    #[test]
    fn create_args_passes_virtualization_and_kernel() {
        let machine = Machine {
            virtualization: true,
            kernel: Some("/nix/store/xxx-nbac-kernel".into()),
            ..Machine::default()
        };
        let args = create_args(&machine, "nbac-builder:abc");
        assert!(args.iter().any(|a| a == "--virtualization"));
        let kernel = args.iter().position(|a| a == "--kernel").unwrap();
        assert_eq!(args[kernel + 1], "/nix/store/xxx-nbac-kernel");
        assert_eq!(args.last().unwrap(), "nbac-builder:abc");
    }
}
