use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    pub device_type: &'static str,
    pub size_bytes: u64,
}

impl DiskInfo {
    pub fn human_size(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;
        const TB: u64 = GB * 1024;

        if self.size_bytes >= TB {
            format!("{:.2} TB", self.size_bytes as f64 / TB as f64)
        } else if self.size_bytes >= GB {
            format!("{:.2} GB", self.size_bytes as f64 / GB as f64)
        } else if self.size_bytes >= MB {
            format!("{:.2} MB", self.size_bytes as f64 / MB as f64)
        } else if self.size_bytes >= KB {
            format!("{:.2} KB", self.size_bytes as f64 / KB as f64)
        } else {
            format!("{} B", self.size_bytes)
        }
    }

    pub fn display(&self) -> String {
        format!(
            "{} ({}) - {}",
            self.path,
            self.device_type,
            self.human_size()
        )
    }
}

pub fn discover_disks() -> Result<Vec<DiskInfo>> {
    let sys_block = Path::new("/sys/block");

    if !sys_block.exists() {
        anyhow::bail!("/sys/block not found - are you running on Linux?");
    }

    let mut disks = Vec::new();

    let entries = fs::read_dir(sys_block).context("Failed to read /sys/block")?;

    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let name = entry.file_name().to_string_lossy().to_string();

        if is_virtual_device(&name) {
            continue;
        }

        let device_path = format!("/dev/{}", name);
        let sys_device_path = entry.path();
        let size_bytes = read_device_size(&sys_device_path).unwrap_or(0);

        if size_bytes == 0 {
            continue;
        }

        let device_type = detect_device_type(&name, &sys_device_path);

        disks.push(DiskInfo {
            name,
            path: device_path,
            device_type,
            size_bytes,
        });
    }

    disks.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(disks)
}

fn is_virtual_device(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("dm-")
        || name.starts_with("zram")
        || name.starts_with("nbd")
}

fn read_device_size(sys_path: &Path) -> Result<u64> {
    let size_path = sys_path.join("size");
    let size_str = fs::read_to_string(&size_path)
        .with_context(|| format!("Failed to read {}", size_path.display()))?;

    let sectors: u64 = size_str
        .trim()
        .parse()
        .with_context(|| format!("Failed to parse size from {}", size_path.display()))?;

    Ok(sectors * 512)
}

fn detect_device_type(name: &str, sys_path: &Path) -> &'static str {
    if name.starts_with("nvme") {
        return "NVMe";
    }

    if name.starts_with("mmcblk") {
        return "SD/MMC";
    }

    let rotational_path = sys_path.join("queue/rotational");

    if let Ok(content) = fs::read_to_string(&rotational_path) {
        if content.trim() == "0" {
            return "SSD";
        } else if content.trim() == "1" {
            return "HDD";
        }
    }

    let removable_path = sys_path.join("removable");

    if let Ok(content) = fs::read_to_string(&removable_path) {
        if content.trim() == "1" {
            return "USB/Removable";
        }
    }

    "Unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_size() {
        let disk = DiskInfo {
            name: "test".to_string(),
            path: "/dev/test".to_string(),
            device_type: "Test",
            size_bytes: 1024 * 1024 * 1024,
        };
        assert_eq!(disk.human_size(), "1.00 GB");
    }

    #[test]
    fn test_is_virtual_device() {
        assert!(is_virtual_device("loop0"));
        assert!(is_virtual_device("ram0"));
        assert!(is_virtual_device("dm-0"));
        assert!(!is_virtual_device("sda"));
        assert!(!is_virtual_device("nvme0n1"));
    }
}
