use std::path::Path;

use anyhow::{Result, bail};
use clap::ValueEnum;

#[derive(Clone, Copy, ValueEnum)]
pub enum LogKind {
    Boot,
    Idle,
}

pub fn cmd_setup(_config: &Path) -> Result<()> {
    bail!("`nbac setup` is not implemented yet")
}

pub fn cmd_status(_config: &Path) -> Result<()> {
    bail!("`nbac status` is not implemented yet")
}

pub fn cmd_start(_config: &Path) -> Result<()> {
    bail!("`nbac start` is not implemented yet")
}

pub fn cmd_stop(_config: &Path) -> Result<()> {
    bail!("`nbac stop` is not implemented yet")
}

pub fn cmd_reset(_config: &Path) -> Result<()> {
    bail!("`nbac reset` is not implemented yet")
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
