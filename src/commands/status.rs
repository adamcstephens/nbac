//! Concurrent read-only probes: runtime, machine, guest sshd, remote store.
//! No retry loops, no recovery, no lock; sub-second when healthy.

use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use console::Style;

use nbac::config::Config;
use nbac::container::{Error as ContainerError, MachineStatus, Runtime};
use nbac::{generation, machine, transport};

enum Probe {
    Ok(String),
    Skip(String),
    Fail(String),
}

pub fn cmd_status(config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let runtime = Runtime::new(&config);
    let expected_tag = generation::image_tag(&config).ok();

    let (runtime_probe, (machine_probe, ssh_probe, store_probe)) = std::thread::scope(|s| {
        let services = s.spawn(|| probe_runtime(&runtime));
        let chain = s.spawn(|| probe_machine(&runtime, &config, expected_tag.as_deref()));
        (services.join().unwrap(), chain.join().unwrap())
    });

    let probes = [
        ("runtime", runtime_probe),
        ("machine", machine_probe),
        ("ssh", ssh_probe),
        ("store", store_probe),
    ];
    for (name, probe) in &probes {
        print_probe(name, probe);
    }
    if probes.iter().any(|(_, p)| matches!(p, Probe::Fail(_))) {
        std::process::exit(1);
    }
    Ok(())
}

fn probe_runtime(runtime: &Runtime) -> Probe {
    match runtime.system_status() {
        Ok(status) if status.status == "running" => Probe::Ok("running".into()),
        Ok(status) => Probe::Fail(format!("{} (run `nbac start`)", status.status)),
        Err(ContainerError::RuntimeDown { .. }) => {
            Probe::Fail("services not running (run `nbac start`)".into())
        }
        Err(e) => Probe::Fail(e.to_string()),
    }
}

/// One inspect, then sshd reachability and the remote store concurrently at
/// the IP it reports.
fn probe_machine(
    runtime: &Runtime,
    config: &Config,
    expected_tag: Option<&str>,
) -> (Probe, Probe, Probe) {
    let skip = |reason: &str| (Probe::Skip(reason.into()), Probe::Skip(reason.into()));
    let info = match runtime.machine_inspect_probe(&config.machine.name) {
        Ok(Some(info)) => info,
        Ok(None) => {
            let (ssh, store) = skip("no machine");
            return (
                Probe::Fail("not created (run `nbac setup`)".into()),
                ssh,
                store,
            );
        }
        Err(ContainerError::RuntimeDown { .. }) => {
            let (ssh, store) = skip("services not running");
            return (
                Probe::Skip("unknown (services not running)".into()),
                ssh,
                store,
            );
        }
        Err(e) => {
            let (ssh, store) = skip("inspect failed");
            return (Probe::Fail(e.to_string()), ssh, store);
        }
    };

    let mut detail = format!(
        "{} · {} cpus · {}",
        info.image.reference,
        info.cpus,
        format_memory(info.memory)
    );
    if let Some(tag) = expected_tag
        && !machine::reference_matches(&info.image.reference, tag)
    {
        detail.push_str(" · image outdated; the next connection recreates the machine");
    }

    if info.status != MachineStatus::Running {
        let (ssh, store) = skip("machine stopped");
        return (
            Probe::Ok(format!("stopped (starts on demand) · {detail}")),
            ssh,
            store,
        );
    }
    let Some(ip) = info.ip_address else {
        let (ssh, store) = skip("machine reports no IP");
        return (
            Probe::Fail(format!("running · {detail} · no IP address")),
            ssh,
            store,
        );
    };

    let (ssh, store) = std::thread::scope(|s| {
        let ssh = s.spawn(|| probe_ssh(&ip, config.ssh.port));
        let store = s.spawn(|| probe_store(config, &ip));
        (ssh.join().unwrap(), store.join().unwrap())
    });
    (Probe::Ok(format!("running · {detail}")), ssh, store)
}

fn probe_ssh(ip: &str, port: u16) -> Probe {
    if transport::reachable(ip, port) {
        Probe::Ok(format!("{ip}:{port} reachable"))
    } else {
        Probe::Fail(format!("{ip}:{port} unreachable"))
    }
}

/// One real SSH session running `nix store info` against the guest daemon:
/// proves the injected keys, the pinned host key, and the daemon socket end
/// to end. The ProxyCommand dials the machine directly so the proxy cold
/// path cannot trigger.
fn probe_store(config: &Config, ip: &str) -> Probe {
    let proxy = format!("/usr/bin/nc -G 3 {ip} {}", config.ssh.port);
    let output = super::ssh_command(config, &proxy)
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
        .arg(&config.ssh.host_alias)
        .args([
            "/nix/var/nix/profiles/default/bin/nix",
            "store",
            "info",
            "--store",
            "daemon",
        ])
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => Probe::Ok("nix daemon responsive".into()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let line = stderr
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("ssh failed");
            Probe::Fail(line.into())
        }
        Err(e) => Probe::Fail(format!("cannot run ssh: {e}")),
    }
}

fn print_probe(name: &str, probe: &Probe) {
    match probe {
        Probe::Ok(detail) => {
            let style = Style::new().green().bold();
            println!("{} {name:<8}{detail}", style.apply_to("✓"));
        }
        Probe::Skip(detail) => {
            let dim = Style::new().dim();
            println!("{} {name:<8}{}", dim.apply_to("-"), dim.apply_to(detail));
        }
        Probe::Fail(detail) => {
            let style = Style::new().red().bold();
            println!("{} {name:<8}{detail}", style.apply_to("✗"));
        }
    }
}

fn format_memory(bytes: u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    if bytes >= GIB && bytes.is_multiple_of(GIB) {
        format!("{}G", bytes / GIB)
    } else if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{}M", bytes / MIB)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_memory() {
        assert_eq!(format_memory(6 * 1024 * 1024 * 1024), "6G");
        assert_eq!(format_memory(512 * 1024 * 1024), "512M");
        assert_eq!(format_memory(1536 * 1024 * 1024), "1536M");
        assert_eq!(format_memory(1000), "1000B");
    }
}
