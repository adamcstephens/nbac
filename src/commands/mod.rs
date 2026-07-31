use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use nbac::config::Config;
use nbac::container::{MachineStatus, Runtime};
use nbac::{inject, keys, lock, machine, transport, ui};

mod status;
pub use status::cmd_status;

/// The cold path shared by setup, start, and the proxy: everything needed to
/// go from nothing to an SSH-ready machine, idempotently.
struct Ready {
    tag: String,
    guest_ip: String,
}

fn ensure_ready(runtime: &Runtime, config: &Config) -> Result<Ready> {
    ensure_services(runtime)?;
    let _lock = lock::acquire(&config.state.lock_file())?;
    keys::ensure(config)?;
    let tag = machine::ensure_image(runtime, config)?;
    machine::ensure_machine(runtime, config, &tag)?;
    let guest_ip = inject::inject(runtime, config)?;
    machine::write_generation_marker(config, &tag)?;
    Ok(Ready { tag, guest_ip })
}

pub fn cmd_setup(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config);
    let tag = ensure_ready(&runtime, &config)?.tag;
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
    let tag = ensure_ready(&runtime, &config)?.tag;
    ui::success(&format!("machine {name} recreated with image {tag}"));
    Ok(())
}

fn ensure_services(runtime: &Runtime) -> Result<()> {
    runtime.check_version()?;
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

/// An ssh invocation with the pinned identity and host key and the given
/// ProxyCommand; callers append the destination and command. The configured
/// port is not passed: every ProxyCommand embeds it.
fn ssh_command(config: &Config, proxy_command: &str) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-F")
        .arg("none")
        .arg("-o")
        .arg(format!("ProxyCommand={proxy_command}"))
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
        .arg(format!("User={}", config.ssh.user));
    cmd
}

pub fn cmd_ssh(config_path: &Path, args: &[String]) -> Result<()> {
    let config = Config::load(config_path)?;
    let exe = std::env::current_exe().context("cannot determine the nbac executable path")?;
    let proxy = format!(
        "\"{}\" --config \"{}\" proxy",
        exe.display(),
        config_path.display()
    );
    let mut cmd = ssh_command(&config, &proxy);
    cmd.arg(&config.ssh.host_alias).args(args);
    Err(anyhow::Error::new(cmd.exec()).context("cannot exec ssh"))
}

pub fn cmd_proxy(config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let runtime = Runtime::new(&config);
    // A running machine with a matching marker can still lack a listening
    // sshd (booted outside nbac, so nothing injected keys and released it),
    // and a wedged runtime can leave the inspect record stale; a failed
    // probe falls back to the cold path. The cold path connects to the IP
    // the injection exec read inside the guest, since inspect's record lags
    // the DHCP lease during early boot.
    let ip = match fast_path_ip(&runtime, &config)
        .filter(|ip| transport::reachable(ip, config.ssh.port))
    {
        Some(ip) => ip,
        None => {
            let ip = ensure_ready(&runtime, &config)?.guest_ip;
            if !transport::await_reachable(&ip, config.ssh.port) {
                bail!(
                    "guest {ip}:{} is unreachable from the host although sshd is up; \
                     the vmnet dataplane may be wedged (try `container system stop`, \
                     then reconnect)",
                    config.ssh.port
                );
            }
            ip
        }
    };
    let err = transport::exec(&ip, config.ssh.port);
    Err(anyhow::Error::new(err).context("cannot exec the transport"))
}

/// One inspect, no lock, no polling: a running machine whose image matches
/// the stored generation marker can be connected to immediately, at the IP
/// the same inspect reports.
fn fast_path_ip(runtime: &Runtime, config: &Config) -> Option<String> {
    let marker = std::fs::read_to_string(config.state.generation_marker()).ok()?;
    let tag = format!("{}:{}", config.image.tag_prefix, marker.trim());
    let info = runtime.machine_inspect(&config.machine.name).ok()??;
    (info.status == MachineStatus::Running
        && machine::reference_matches(&info.image.reference, &tag))
    .then_some(info.ip_address)?
}
