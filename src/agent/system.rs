//! What the agent can learn about the machine it runs on.
//!
//! Everything that touches the real system goes through [`SystemInspector`].
//! That indirection exists so capability discovery can be tested against a
//! fileserver, a GPU node and a bare login node on a laptop with none of
//! those — a test suite that can only assert what the developer's machine
//! happens to have is worth very little.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountInfo {
    /// What is mounted (`server:/export`, `/dev/sda1`).
    pub source: String,
    /// Where it is mounted.
    pub target: String,
    /// Filesystem type (`nfs4`, `ext4`, `zfs`).
    pub fstype: String,
    /// Mount options, as reported.
    pub options: String,
}

impl MountInfo {
    /// Whether this is a network filesystem of the NFS family.
    pub fn is_nfs(&self) -> bool {
        self.fstype == "nfs" || self.fstype == "nfs4" || self.fstype.starts_with("nfs")
    }

    /// The server component of an NFS source, if there is one.
    pub fn nfs_server(&self) -> Option<&str> {
        if !self.is_nfs() {
            return None;
        }
        // `server:/export`, and IPv6 as `[::1]:/export`.
        let (server, _) = self.source.rsplit_once(":/")?;
        Some(server.trim_start_matches('[').trim_end_matches(']'))
    }

    /// Whether the mount is read-only.
    pub fn is_read_only(&self) -> bool {
        self.options.split(',').any(|o| o == "ro")
    }
}

/// A summary of the machine's resources, for comparison against what a
/// scheduler has been configured to expect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareSummary {
    /// Logical CPUs.
    pub cpus: Option<u32>,
    /// Usable memory in MiB.
    pub memory_mb: Option<u64>,
    /// GPUs found.
    pub gpus: Option<u32>,
    /// Kernel release.
    pub kernel: Option<String>,
}

/// The agent's window onto the local system.
pub trait SystemInspector: Send + Sync {
    /// Short host name.
    fn hostname(&self) -> Option<String>;
    /// Fully qualified name, when resolvable.
    fn fqdn(&self) -> Option<String>;
    /// Linux boot id; changes on reboot.
    fn boot_id(&self) -> Option<String>;
    /// Addresses this host answers on.
    fn addresses(&self) -> Vec<String>;
    /// Whether a path exists.
    fn path_exists(&self, path: &Path) -> bool;
    /// Locate an executable on `PATH`.
    fn which(&self, program: &str) -> Option<PathBuf>;
    /// Currently mounted filesystems.
    fn mounts(&self) -> Vec<MountInfo>;
    /// Hardware summary.
    fn hardware(&self) -> HardwareSummary;
}

/// The real Linux implementation.
#[derive(Debug, Clone, Default)]
pub struct LinuxInspector;

impl LinuxInspector {
    /// An inspector reading the live system.
    pub fn new() -> Self {
        Self
    }
}

impl SystemInspector for LinuxInspector {
    fn hostname(&self) -> Option<String> {
        hostname::get().ok().and_then(|h| h.into_string().ok()).map(|h| {
            // A host that calls itself `node-a.example.org` is still `node-a`;
            // the canonical name must not drift with DNS configuration.
            h.split('.').next().unwrap_or(&h).to_string()
        })
    }

    fn fqdn(&self) -> Option<String> {
        let full = hostname::get().ok().and_then(|h| h.into_string().ok())?;
        full.contains('.').then_some(full)
    }

    fn boot_id(&self) -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    }

    fn addresses(&self) -> Vec<String> {
        let Ok(interfaces) = if_addrs::get_if_addrs() else {
            return Vec::new();
        };
        let mut addresses: Vec<String> = interfaces
            .into_iter()
            // Loopback tells no one anything about reachability.
            .filter(|i| !i.is_loopback())
            .map(|i| i.addr.ip().to_string())
            .collect();
        addresses.sort();
        addresses.dedup();
        addresses
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    }

    fn mounts(&self) -> Vec<MountInfo> {
        // /proc/self/mounts is a read of kernel state: it does not touch the
        // filesystems it lists, so it is safe even when an NFS mount is hung.
        // Calling `df` or `stat` here would not be (SPEC.md §75).
        std::fs::read_to_string("/proc/self/mounts")
            .map(|text| parse_mounts(&text))
            .unwrap_or_default()
    }

    fn hardware(&self) -> HardwareSummary {
        HardwareSummary {
            cpus: Some(
                std::thread::available_parallelism()
                    .map(|n| n.get() as u32)
                    .unwrap_or(0),
            )
            .filter(|n| *n > 0),
            memory_mb: read_meminfo_total_mb(),
            gpus: None,
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .ok()
                .map(|k| k.trim().to_string()),
        }
    }
}

/// Parse `/proc/self/mounts`.
pub fn parse_mounts(text: &str) -> Vec<MountInfo> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let source = fields.next()?;
            let target = fields.next()?;
            let fstype = fields.next()?;
            let options = fields.next().unwrap_or("");
            Some(MountInfo {
                // The kernel escapes spaces and tabs in these fields.
                source: unescape_mount_field(source),
                target: unescape_mount_field(target),
                fstype: fstype.to_string(),
                options: options.to_string(),
            })
        })
        .collect()
}

/// Decode the kernel's octal escapes in a mount field.
fn unescape_mount_field(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_string();
    }
    let bytes: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == '\\' && index + 3 < bytes.len() {
            let octal: String = bytes[index + 1..index + 4].iter().collect();
            if let Ok(byte) = u8::from_str_radix(&octal, 8) {
                out.push(byte as char);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn read_meminfo_total_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// An inspector that reports whatever a test says, for exercising discovery
/// against machines the test machine is not.
#[derive(Debug, Clone, Default)]
pub struct FakeInspector {
    /// Host name to report.
    pub hostname: Option<String>,
    /// FQDN to report.
    pub fqdn: Option<String>,
    /// Boot id to report.
    pub boot_id: Option<String>,
    /// Addresses to report.
    pub addresses: Vec<String>,
    /// Paths that exist.
    pub paths: BTreeMap<String, bool>,
    /// Executables on `PATH`.
    pub programs: BTreeMap<String, PathBuf>,
    /// Mounted filesystems.
    pub mounts: Vec<MountInfo>,
    /// Hardware summary.
    pub hardware: HardwareSummary,
}

impl FakeInspector {
    /// An inspector reporting a bare host with nothing installed.
    pub fn bare() -> Self {
        Self {
            hostname: Some("test-host".into()),
            boot_id: Some("boot-1".into()),
            addresses: vec!["192.0.2.1".into()],
            hardware: HardwareSummary {
                cpus: Some(8),
                memory_mb: Some(16384),
                gpus: None,
                kernel: None,
            },
            ..Self::default()
        }
    }

    /// Builder: pretend a program is installed.
    pub fn with_program(mut self, program: &str) -> Self {
        self.programs
            .insert(program.to_string(), PathBuf::from(format!("/usr/bin/{program}")));
        self
    }

    /// Builder: pretend a path exists.
    pub fn with_path(mut self, path: &str) -> Self {
        self.paths.insert(path.to_string(), true);
        self
    }

    /// Builder: add a mount.
    pub fn with_mount(mut self, source: &str, target: &str, fstype: &str) -> Self {
        self.mounts.push(MountInfo {
            source: source.into(),
            target: target.into(),
            fstype: fstype.into(),
            options: "rw".into(),
        });
        self
    }

    /// Builder: set the host name.
    pub fn with_hostname(mut self, hostname: &str) -> Self {
        self.hostname = Some(hostname.to_string());
        self
    }

    /// Builder: set the boot id.
    pub fn with_boot_id(mut self, boot_id: &str) -> Self {
        self.boot_id = Some(boot_id.to_string());
        self
    }
}

impl SystemInspector for FakeInspector {
    fn hostname(&self) -> Option<String> {
        self.hostname.clone()
    }

    fn fqdn(&self) -> Option<String> {
        self.fqdn.clone()
    }

    fn boot_id(&self) -> Option<String> {
        self.boot_id.clone()
    }

    fn addresses(&self) -> Vec<String> {
        self.addresses.clone()
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.paths.get(&path.display().to_string()).copied().unwrap_or(false)
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.programs.get(program).cloned()
    }

    fn mounts(&self) -> Vec<MountInfo> {
        self.mounts.clone()
    }

    fn hardware(&self) -> HardwareSummary {
        self.hardware.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_mounts_parses_including_escaped_spaces() {
        let text = "\
/dev/sda1 / ext4 rw,relatime 0 0
fileserver:/export/home /home nfs4 rw,relatime,vers=4.2 0 0
/dev/sdb1 /mnt/with\\040space ext4 ro 0 0
";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[0].fstype, "ext4");
        assert_eq!(mounts[1].target, "/home");
        assert_eq!(
            mounts[2].target, "/mnt/with space",
            "the kernel escapes spaces as \\040"
        );
    }

    #[test]
    fn a_malformed_mount_line_is_skipped_not_fatal() {
        let mounts = parse_mounts("garbage\n/dev/sda1 / ext4 rw 0 0\n\n");
        assert_eq!(mounts.len(), 1);
    }

    #[test]
    fn nfs_mounts_are_recognised_across_versions() {
        for fstype in ["nfs", "nfs4"] {
            let mount = MountInfo {
                source: "fileserver:/export".into(),
                target: "/home".into(),
                fstype: fstype.into(),
                options: "rw".into(),
            };
            assert!(mount.is_nfs());
            assert_eq!(mount.nfs_server(), Some("fileserver"));
        }
    }

    #[test]
    fn a_local_filesystem_is_not_an_nfs_mount() {
        let mount = MountInfo {
            source: "/dev/sda1".into(),
            target: "/".into(),
            fstype: "ext4".into(),
            options: "rw".into(),
        };
        assert!(!mount.is_nfs());
        assert_eq!(mount.nfs_server(), None);
    }

    #[test]
    fn an_ipv6_nfs_server_is_extracted_without_its_brackets() {
        let mount = MountInfo {
            source: "[2001:db8::1]:/export".into(),
            target: "/home".into(),
            fstype: "nfs4".into(),
            options: "rw".into(),
        };
        assert_eq!(mount.nfs_server(), Some("2001:db8::1"));
    }

    #[test]
    fn a_read_only_mount_is_recognised_without_matching_a_substring() {
        let read_only = MountInfo {
            source: "s:/e".into(),
            target: "/h".into(),
            fstype: "nfs4".into(),
            options: "ro,relatime".into(),
        };
        assert!(read_only.is_read_only());

        let read_write = MountInfo {
            options: "rw,relatime".into(),
            ..read_only.clone()
        };
        assert!(!read_write.is_read_only());

        // `root` and `rootcontext` contain "ro" but do not mean read-only.
        let tricky = MountInfo {
            options: "rw,rootcontext=x".into(),
            ..read_only
        };
        assert!(!tricky.is_read_only());
    }

    #[test]
    fn the_real_inspector_reports_something_plausible_about_this_machine() {
        // Deliberately weak: this runs on developer laptops and CI containers
        // alike, so it asserts only what must be true anywhere.
        let inspector = LinuxInspector::new();
        assert!(inspector.hostname().is_some_and(|h| !h.is_empty()));
        assert!(
            !inspector.hostname().unwrap().contains('.'),
            "the short name must be short"
        );
        assert!(inspector.hardware().cpus.unwrap_or(0) > 0);
    }

    #[test]
    fn the_fake_inspector_reports_exactly_what_it_was_told() {
        let inspector = FakeInspector::bare()
            .with_hostname("fileserver-a")
            .with_program("nvidia-smi")
            .with_path("/etc/exports")
            .with_mount("fileserver:/export", "/home", "nfs4");

        assert_eq!(inspector.hostname().as_deref(), Some("fileserver-a"));
        assert!(inspector.which("nvidia-smi").is_some());
        assert!(inspector.which("zpool").is_none());
        assert!(inspector.path_exists(Path::new("/etc/exports")));
        assert!(!inspector.path_exists(Path::new("/etc/elsewhere")));
        assert_eq!(inspector.mounts().len(), 1);
    }
}
