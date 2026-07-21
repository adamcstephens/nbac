use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

use nbac::config::Config;
use nbac::container::{MachineStatus, Runtime};
use nbac::{inject, keys, lock, machine, transport, ui};

#[derive(Clone, Copy, ValueEnum)]
pub enum LogKind {
    Boot,
    Idle,
}

/// The cold path shared by setup, start, and the proxy: everything needed to
/// go from nothing to an SSH-ready machine, idempotently.
fn ensure_ready(runtime: &Runtime, config: &Config) -> Result<String> {
    ensure_services(runtime)?;
    let _lock = lock::acquire(&config.state.lock_file())?;
    keys::ensure(config)?;
    let tag = machine::ensure_image(runtime, config)?;
    machine::ensure_machine(runtime, config, &tag)?;
    inject::inject(runtime, config)?;
    machine::write_generation_marker(config, &tag)?;
    Ok(tag)
}

pub fn cmd_setup(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config);
    let tag = ensure_ready(&runtime, &config)?;
    ui::success(&format!(
        "machine {} is running image {tag}",
        config.machine.name
    ));
    Ok(())
}

pub fn cmd_start(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config);
    ensure_ready(&runtime, &config)?;
    ui::success(&format!("machine {} is running", config.machine.name));
    Ok(())
}

pub fn cmd_stop(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config);
    let _lock = lock::acquire(&config.state.lock_file())?;
    runtime.machine_stop(&config.machine.name)?;
    ui::success(&format!("machine {} stopped", config.machine.name));
    Ok(())
}

pub fn cmd_reset(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let name = &config.machine.name;
    ui::warn(&format!(
        "this destroys machine {name} and deletes its /nix store"
    ));
    if !confirm(&format!("recreate machine {name}?"))? {
        bail!("aborted");
    }

    let runtime = Runtime::new(&config);
    ensure_services(&runtime)?;
    {
        let _lock = lock::acquire(&config.state.lock_file())?;
        if let Some(info) = runtime.machine_inspect(name)? {
            if info.status == MachineStatus::Running {
                runtime.machine_stop(name)?;
            }
            runtime.machine_delete(name)?;
        }
    }
    let tag = ensure_ready(&runtime, &config)?;
    ui::success(&format!("machine {name} recreated with image {tag}"));
    Ok(())
}

fn ensure_services(runtime: &Runtime) -> Result<()> {
    match runtime.system_status() {
        Ok(status) if status.status == "running" => Ok(()),
        _ => {
            ui::step("starting container services");
            runtime.system_start()?;
            Ok(())
        }
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

pub fn cmd_status(_config: &Path) -> Result<()> {
    bail!("`nbac status` is not implemented yet")
}

pub fn cmd_ssh(config_path: &Path, args: &[String]) -> Result<()> {
    let config = Config::load(config_path)?;
    let exe = std::env::current_exe().context("cannot determine the nbac executable path")?;
    let proxy = format!(
        "\"{}\" --config \"{}\" proxy",
        exe.display(),
        config_path.display()
    );
    let mut cmd = Command::new("ssh");
    cmd.arg("-F")
        .arg("none")
        .arg("-o")
        .arg(format!("ProxyCommand={proxy}"))
        .arg("-o")
        .arg(format!(
            "IdentityFile={}",
            config.state.builder_key().display()
        ))
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg(format!(
            "UserKnownHostsFile={}",
            config.state.known_hosts().display()
        ))
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg(format!("User={}", config.ssh.user))
        .arg("-o")
        .arg(format!("Port={}", config.ssh.port))
        .arg(&config.ssh.host_alias)
        .args(args);
    Err(anyhow::Error::new(cmd.exec()).context("cannot exec ssh"))
}

pub fn cmd_proxy(config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let runtime = Runtime::new(&config);
    // A running machine with a matching marker can still lack a listening
    // sshd or hold a stale recorded IP (rebooted outside nbac); a failed
    // probe falls back to the cold path, whose injection starts sshd and
    // re-records the IP.
    let ip = match fast_path_ip(&runtime, &config)
        .filter(|ip| transport::reachable(ip, config.ssh.port))
    {
        Some(ip) => ip,
        None => {
            ensure_ready(&runtime, &config)?;
            std::fs::read_to_string(config.state.guest_ip())
                .context("injection did not record the guest IP")?
                .trim()
                .to_string()
        }
    };
    let err = transport::exec(&ip, config.ssh.port);
    Err(anyhow::Error::new(err).context("cannot exec the transport"))
}

/// One inspect, no lock, no polling: a running machine whose image matches
/// the stored generation marker can be connected to immediately, at the IP
/// the guest reported during the last injection.
fn fast_path_ip(runtime: &Runtime, config: &Config) -> Option<String> {
    let marker = std::fs::read_to_string(config.state.generation_marker()).ok()?;
    let tag = format!("{}:{}", config.image.tag_prefix, marker.trim());
    let info = runtime.machine_inspect(&config.machine.name).ok()??;
    (info.status == MachineStatus::Running
        && machine::reference_matches(&info.image.reference, &tag))
    .then(|| std::fs::read_to_string(config.state.guest_ip()).ok())?
    .map(|ip| ip.trim().to_string())
}

pub fn cmd_doctor(_config: &Path, _fix: bool) -> Result<()> {
    bail!("`nbac doctor` is not implemented yet")
}

pub fn cmd_test(_config: &Path) -> Result<()> {
    bail!("`nbac test` is not implemented yet")
}

pub fn cmd_gc(_config: &Path) -> Result<()> {
    bail!("`nbac gc` is not implemented yet")
}

pub fn cmd_logs(_config: &Path, _log: LogKind, _follow: bool, _lines: Option<u64>) -> Result<()> {
    bail!("`nbac logs` is not implemented yet")
}
