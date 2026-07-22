//! Runtime injection: the single guest exec that writes the SSH keys and
//! idle settings into the machine and (re)starts sshd.

use anyhow::{Context, Result};

use crate::config::Config;
use crate::container::Runtime;

pub fn inject(runtime: &Runtime, config: &Config) -> Result<()> {
    let script = script(config)?;
    runtime
        .machine_run_piped(&config.machine.name, &script)
        .context("key injection failed")
}

fn script(config: &Config) -> Result<String> {
    let read = |path: std::path::PathBuf| {
        std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {} (run `nbac setup`?)", path.display()))
    };
    let authorized_keys = read(config.state.builder_key_pub())?;
    let host_key = read(config.state.host_key())?;
    let host_key_pub = read(config.state.host_key_pub())?;
    let runtime_toml = format!(
        "enable = {}\ntimeout_seconds = {}\n",
        config.idle.enable, config.idle.timeout_seconds
    );

    let user = &config.ssh.user;
    let port = config.ssh.port;
    Ok(format!(
        r#"set -eu
install -d -m 700 "/home/{user}/.ssh"
cat > "/home/{user}/.ssh/authorized_keys" <<'NBAC_EOF'
{authorized_keys}NBAC_EOF
chmod 600 "/home/{user}/.ssh/authorized_keys"
chown -R "{user}:{user}" "/home/{user}/.ssh"
umask 077
cat > /etc/ssh/ssh_host_ed25519_key <<'NBAC_EOF'
{host_key}NBAC_EOF
cat > /etc/ssh/ssh_host_ed25519_key.pub <<'NBAC_EOF'
{host_key_pub}NBAC_EOF
cat > /etc/nbac/runtime.toml <<'NBAC_EOF'
{runtime_toml}NBAC_EOF
for _ in $(seq 1 100); do
    s6-svok /run/service/sshd && break
    sleep 0.1
done
s6-svc -ru /run/service/sshd
for _ in $(seq 1 100); do
    [ -n "$(ss -Htln "sport = :{port}")" ] && break
    sleep 0.1
done
if [ -z "$(ss -Htln "sport = :{port}")" ]; then
    echo "sshd is not listening on port {port}" >&2
    exit 1
fi
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_embeds_keys_and_settings() {
        let dir = std::env::temp_dir().join(format!("nbac-test-inject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Containerfile"), "FROM alpine\n").unwrap();
        let config: Config = toml::from_str(&format!(
            r#"
            [image]
            containerfile = "{0}/Containerfile"

            [state]
            dir = "{0}"

            [idle]
            enable = true
            timeout_seconds = 120
            "#,
            dir.display()
        ))
        .unwrap();

        std::fs::write(config.state.builder_key_pub(), "ssh-ed25519 AAAA builder\n").unwrap();
        std::fs::write(config.state.host_key(), "PRIVATE\n").unwrap();
        std::fs::write(config.state.host_key_pub(), "ssh-ed25519 BBBB host\n").unwrap();

        let script = script(&config).unwrap();
        assert!(script.contains("ssh-ed25519 AAAA builder\nNBAC_EOF"));
        assert!(script.contains("ssh-ed25519 BBBB host\nNBAC_EOF"));
        assert!(script.contains("timeout_seconds = 120"));
        assert!(script.contains("/home/builder/.ssh"));
    }

    #[test]
    fn missing_keys_are_reported() {
        let dir =
            std::env::temp_dir().join(format!("nbac-test-inject-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Containerfile"), "FROM alpine\n").unwrap();
        let config: Config = toml::from_str(&format!(
            r#"
            [image]
            containerfile = "{0}/Containerfile"

            [state]
            dir = "{0}"
            "#,
            dir.display()
        ))
        .unwrap();

        let err = script(&config).unwrap_err();
        assert!(err.to_string().contains("builder_ed25519.pub"));
    }
}
