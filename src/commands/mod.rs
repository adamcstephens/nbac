use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use clap::ValueEnum;

use nbac::config::Config;
use nbac::container::Runtime;
use nbac::{keys, lock, machine, ui};

#[derive(Clone, Copy, ValueEnum)]
pub enum LogKind {
    Boot,
    Idle,
}

pub fn cmd_setup(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config.runtime);
    ensure_services(&runtime)?;
    keys::ensure(&config)?;
    let _lock = lock::acquire(&config.state.lock_file())?;
    let tag = machine::ensure_image(&runtime, &config)?;
    machine::ensure_machine(&runtime, &config, &tag)?;
    ui::success(&format!(
        "machine {} is running image {tag}",
        config.machine.name
    ));
    Ok(())
}

pub fn cmd_start(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config.runtime);
    ensure_services(&runtime)?;
    let _lock = lock::acquire(&config.state.lock_file())?;
    let tag = machine::ensure_image(&runtime, &config)?;
    machine::ensure_machine(&runtime, &config, &tag)?;
    ui::success(&format!("machine {} is running", config.machine.name));
    Ok(())
}

pub fn cmd_stop(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let runtime = Runtime::new(&config.runtime);
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

    let runtime = Runtime::new(&config.runtime);
    ensure_services(&runtime)?;
    let _lock = lock::acquire(&config.state.lock_file())?;
    if let Some(info) = runtime.machine_inspect(name)? {
        if info.status == nbac::container::MachineStatus::Running {
            runtime.machine_stop(name)?;
        }
        runtime.machine_delete(name)?;
    }
    let tag = machine::ensure_image(&runtime, &config)?;
    machine::ensure_machine(&runtime, &config, &tag)?;
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

pub fn cmd_ssh(_config: &Path, _args: &[String]) -> Result<()> {
    bail!("`nbac ssh` is not implemented yet")
}

pub fn cmd_proxy(_config: &Path) -> Result<()> {
    bail!("`nbac proxy` is not implemented yet")
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
