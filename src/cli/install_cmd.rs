//! `sentinel install` — generate systemd units and directories.
//!
//! Production deployment is systemd on a native host (IMPLEMENTATION.md §12).
//! The units generated here are hardened by default (IMPLEMENTATION.md §85):
//! Sentinel is a read-only monitoring daemon, and the unit should say so, so
//! that a bug in a probe cannot become a bug in the operating system.
//!
//! Nothing here writes a credential. The unit points at a file the operator
//! creates, so no secret passes through a command line or the process table.

use std::path::{Path, PathBuf};

use crate::config::{DEFAULT_CONFIG_PATH, DEFAULT_STATE_DIR};

/// The systemd unit for the controller.
pub fn controller_unit(binary: &Path, config: &Path) -> String {
    unit(
        "Cluster Sentinel controller",
        &format!("{} --config {} controller", binary.display(), config.display()),
        // The controller owns the database, so it needs to write there.
        &[DEFAULT_STATE_DIR],
    )
}

/// The systemd unit for the agent.
pub fn agent_unit(binary: &Path, config: &Path) -> String {
    unit(
        "Cluster Sentinel agent",
        &format!("{} --config {} agent", binary.display(), config.display()),
        &[DEFAULT_STATE_DIR],
    )
}

/// Build a hardened unit.
fn unit(description: &str, exec_start: &str, writable: &[&str]) -> String {
    format!(
        "\
[Unit]
Description={description}
Documentation=https://github.com/mizuno-lab/cluster-sentinel
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
ExecStart={exec_start}
Restart=always
RestartSec=5s

# The credential is read from a file, never from a command line or the
# environment of a unit an operator might paste into a bug report.
Environment=SENTINEL_TOKEN_FILE=/etc/sentinel/token

# A dedicated unprivileged user. Sentinel reads; it does not need to be root,
# and probes that cannot read something report UNSUPPORTED rather than failing.
User=sentinel
Group=sentinel

# Hardening (IMPLEMENTATION.md §85). Sentinel is a read-only monitoring daemon,
# so the unit says so: a bug in a probe must not become a bug in the OS.
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
ProtectClock=true
RestrictSUIDSGID=true
RestrictRealtime=true
RestrictNamespaces=true
LockPersonality=true
MemoryDenyWriteExecute=true
SystemCallArchitectures=native
CapabilityBoundingSet=
AmbientCapabilities=
{}

# Journald is the log destination (IMPLEMENTATION.md §59).
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
",
        writable
            .iter()
            .map(|path| format!("ReadWritePaths={path}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// The shell commands an operator runs to finish the install.
pub fn setup_instructions(role: &str, unit_path: &Path) -> String {
    format!(
        "\
Next steps:

  1. Create the service user and directories:

     sudo useradd --system --no-create-home --shell /usr/sbin/nologin sentinel
     sudo install -d -o sentinel -g sentinel -m 0750 {DEFAULT_STATE_DIR}
     sudo install -d -m 0755 /etc/sentinel

  2. Create the cluster credential, readable only by the service user:

     head -c 32 /dev/urandom | base64 | sudo tee /etc/sentinel/token > /dev/null
     sudo chown sentinel:sentinel /etc/sentinel/token
     sudo chmod 0400 /etc/sentinel/token

     The same credential goes on every host in the environment.

  3. Write {DEFAULT_CONFIG_PATH} and check it:

     sentinel config check

  4. Start the {role}:

     sudo systemctl daemon-reload
     sudo systemctl enable --now sentinel-{role}
     systemctl status sentinel-{role}

The unit was written to {}.
",
        unit_path.display()
    )
}

/// Run `sentinel install <role>`.
pub fn run(role: &str, output_dir: &Path, config: &Path, dry_run: bool) -> anyhow::Result<i32> {
    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/sentinel"));

    let (unit_name, contents) = match role {
        "controller" => ("sentinel-controller.service", controller_unit(&binary, config)),
        "agent" => ("sentinel-agent.service", agent_unit(&binary, config)),
        other => anyhow::bail!("unknown role {other:?}; expected \"controller\" or \"agent\""),
    };

    let unit_path = output_dir.join(unit_name);

    if dry_run {
        print!("{contents}");
        return Ok(0);
    }

    std::fs::create_dir_all(output_dir).map_err(|e| anyhow::anyhow!("cannot create {}: {e}", output_dir.display()))?;
    std::fs::write(&unit_path, &contents).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", unit_path.display()))?;

    println!("wrote {}", unit_path.display());
    println!();
    print!("{}", setup_instructions(role, &unit_path));

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> String {
        controller_unit(Path::new("/usr/local/bin/sentinel"), Path::new(DEFAULT_CONFIG_PATH))
    }

    #[test]
    fn the_unit_runs_the_right_subcommand() {
        assert!(
            controller().contains("ExecStart=/usr/local/bin/sentinel --config /etc/sentinel/config.toml controller")
        );

        let agent = agent_unit(Path::new("/usr/local/bin/sentinel"), Path::new(DEFAULT_CONFIG_PATH));
        assert!(agent.contains("agent"));
        assert!(!agent.contains("controller"));
    }

    #[test]
    fn every_hardening_directive_the_specification_asks_for_is_present() {
        // IMPLEMENTATION.md §85.
        let unit = controller();
        for directive in [
            "NoNewPrivileges=true",
            "PrivateTmp=true",
            "ProtectHome=true",
            "ProtectSystem=strict",
            "ProtectKernelTunables=true",
            "ProtectControlGroups=true",
            "RestrictSUIDSGID=true",
        ] {
            assert!(unit.contains(directive), "missing {directive}");
        }
    }

    #[test]
    fn the_daemon_runs_unprivileged_with_no_capabilities() {
        let unit = controller();
        assert!(unit.contains("User=sentinel"));
        assert!(!unit.contains("User=root"));
        assert!(
            unit.contains("CapabilityBoundingSet="),
            "no capabilities are needed to read"
        );
        assert!(unit.contains("AmbientCapabilities="));
    }

    #[test]
    fn only_the_state_directory_is_writable() {
        // ProtectSystem=strict makes everything read-only; this is the one
        // exception, and it should stay the only one.
        let unit = controller();
        assert_eq!(unit.matches("ReadWritePaths=").count(), 1);
        assert!(unit.contains(&format!("ReadWritePaths={DEFAULT_STATE_DIR}")));
    }

    #[test]
    fn no_credential_is_written_into_the_unit() {
        // A unit file gets pasted into bug reports and checked into
        // configuration management. It must never carry a secret.
        let unit = controller();
        assert!(unit.contains("SENTINEL_TOKEN_FILE="), "the unit points at a file");
        assert!(!unit.contains("SENTINEL_TOKEN="), "and never carries the value");
    }

    #[test]
    fn the_unit_restarts_and_logs_to_the_journal() {
        let unit = controller();
        assert!(
            unit.contains("Restart=always"),
            "monitoring that stays down helps nobody"
        );
        assert!(unit.contains("StandardOutput=journal"));
    }

    #[test]
    fn a_dry_run_prints_without_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            run("controller", dir.path(), Path::new(DEFAULT_CONFIG_PATH), true).expect("dry run"),
            0
        );
        assert!(!dir.path().join("sentinel-controller.service").exists());
    }

    #[test]
    fn installing_writes_the_unit() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            run("agent", dir.path(), Path::new(DEFAULT_CONFIG_PATH), false).expect("install"),
            0
        );

        let written = std::fs::read_to_string(dir.path().join("sentinel-agent.service")).expect("read");
        assert!(written.contains("Cluster Sentinel agent"));
    }

    #[test]
    fn an_unknown_role_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(run("fileserver", dir.path(), Path::new(DEFAULT_CONFIG_PATH), true).is_err());
    }

    #[test]
    fn the_instructions_never_tell_an_operator_to_run_sentinel_as_root() {
        let instructions = setup_instructions("controller", Path::new("/etc/systemd/system/x.service"));
        assert!(instructions.contains("useradd --system"));
        assert!(instructions.contains("chmod 0400"), "the credential must be protected");
        assert!(instructions.contains("sentinel config check"), "check before starting");
    }
}
