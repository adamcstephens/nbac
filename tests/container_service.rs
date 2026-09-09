use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use nbac::{config::Config, generation};

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    machine: String,
    image: String,
    container_binary: String,
}

impl Fixture {
    fn new() -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("nbac-e2e-{suffix}"));
        let config = root.join("config.toml");
        let machine = format!("nbac-e2e-{suffix}");
        let tag_prefix = format!("nbac-e2e-{suffix}");
        let container_binary =
            std::env::var("NBAC_CONTAINER_BINARY").unwrap_or_else(|_| "container".into());
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            &config,
            format!(
                r#"[image]
containerfile = "{}/images/Containerfile"
build_context = "{}/images"
tag_prefix = "{tag_prefix}"

[machine]
name = "{machine}"

[state]
dir = "{}/state"

[runtime]
container_binary = "{container_binary}"
"#,
                manifest.display(),
                manifest.display(),
                root.display(),
            ),
        )
        .unwrap();

        let image = generation::image_tag(&Config::load(&config).unwrap()).unwrap();
        Self {
            root,
            config,
            machine,
            image,
            container_binary,
        }
    }

    fn nbac(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nbac"));
        command.args(["--config", self.config.to_str().unwrap()]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new(&self.container_binary)
            .args(["machine", "stop", &self.machine])
            .output();
        let _ = Command::new(&self.container_binary)
            .args(["machine", "delete", &self.machine])
            .output();
        let _ = Command::new(&self.container_binary)
            .args(["image", "delete", "--force", &self.image])
            .output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed ({status}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        status = output.status,
        stdout = String::from_utf8_lossy(&output.stdout),
        stderr = String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[ignore = "requires a supported Apple container service; creates a temporary builder"]
fn setup_and_ssh_build_work_against_container_service() {
    let fixture = Fixture::new();

    let setup = fixture.nbac().arg("setup").output().unwrap();
    assert_success("nbac setup", &setup);

    let build = fixture
        .nbac()
        .args([
            "ssh",
            "/nix/var/nix/profiles/default/bin/nix",
            "build",
            "--store",
            "daemon",
            "--no-link",
            "--expr",
            r#"derivation { name = "nbac-e2e"; system = "aarch64-linux"; builder = "/bin/sh"; args = [ "-c" "echo ok > $out" ]; }"#,
        ])
        .output()
        .unwrap();
    assert_success("nbac ssh nix build", &build);
}
