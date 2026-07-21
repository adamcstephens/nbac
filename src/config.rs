use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub machine: Machine,
    pub image: Image,
    #[serde(default)]
    pub ssh: Ssh,
    pub state: State,
    #[serde(default)]
    pub idle: Idle,
    #[serde(default)]
    pub runtime: Runtime,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Machine {
    pub name: String,
    pub cpus: u32,
    pub memory: String,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            name: "nbac".into(),
            cpus: 4,
            memory: "6G".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub containerfile: PathBuf,
    pub build_context: Option<PathBuf>,
    #[serde(default = "default_tag_prefix")]
    pub tag_prefix: String,
}

fn default_tag_prefix() -> String {
    "nbac-builder".into()
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ssh {
    pub user: String,
    pub port: u16,
    pub host_alias: String,
}

impl Default for Ssh {
    fn default() -> Self {
        Self {
            user: "builder".into(),
            port: 22,
            host_alias: "nbac".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Idle {
    pub enable: bool,
    pub timeout_seconds: u64,
}

impl Default for Idle {
    fn default() -> Self {
        Self {
            enable: true,
            timeout_seconds: 300,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Runtime {
    pub container_binary: String,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            container_binary: "container".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_example() {
        let config: Config = toml::from_str(
            r#"
            [machine]
            name = "nbac"
            cpus = 4
            memory = "6G"

            [image]
            containerfile = "/nix/store/aaa-Containerfile"
            build_context = "/nix/store/aaa-context"
            tag_prefix = "nbac-builder"

            [ssh]
            user = "builder"
            port = 22
            host_alias = "nbac"

            [state]
            dir = "/Users/adam/.local/state/nbac"

            [idle]
            enable = true
            timeout_seconds = 300

            [runtime]
            container_binary = "container"
            "#,
        )
        .unwrap();

        assert_eq!(config.machine.cpus, 4);
        assert_eq!(config.image.tag_prefix, "nbac-builder");
        assert_eq!(config.ssh.user, "builder");
        assert!(config.idle.enable);
    }

    #[test]
    fn minimal_config_uses_defaults() {
        let config: Config = toml::from_str(
            r#"
            [image]
            containerfile = "/etc/nbac/Containerfile"

            [state]
            dir = "/var/lib/nbac"
            "#,
        )
        .unwrap();

        assert_eq!(config.machine.name, "nbac");
        assert_eq!(config.machine.memory, "6G");
        assert!(config.image.build_context.is_none());
        assert_eq!(config.ssh.port, 22);
        assert_eq!(config.idle.timeout_seconds, 300);
        assert_eq!(config.runtime.container_binary, "container");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = toml::from_str::<Config>(
            r#"
            [image]
            containerfile = "/etc/nbac/Containerfile"
            containerfle = "typo"

            [state]
            dir = "/var/lib/nbac"
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("containerfle"));
    }
}
