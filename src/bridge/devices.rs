use serde::{Deserialize, Serialize};

use crate::error::ArgosError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceClassDto {
    Hdd,
    Ssd,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub class: DeviceClassDto,
    pub removable: bool,
    pub model: Option<String>,
}

pub fn list() -> Result<Vec<DeviceInfo>, ArgosError> {
    #[cfg(target_os = "linux")]
    {
        list_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Vec::new())
    }
}

#[cfg(target_os = "linux")]
fn list_linux() -> Result<Vec<DeviceInfo>, ArgosError> {
    let entries = std::fs::read_dir("/sys/block")?;
    let mut devices: Vec<DeviceInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| device_from_sysfs(&entry))
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

#[cfg(target_os = "linux")]
fn device_from_sysfs(entry: &std::fs::DirEntry) -> Option<DeviceInfo> {
    let name = entry.file_name().to_string_lossy().into_owned();
    if is_virtual_block(&name) {
        return None;
    }
    let base = entry.path();
    let size_sectors: u64 = read_trim(base.join("size"))?.parse().ok()?;
    let size_bytes = size_sectors.checked_mul(512)?;
    if size_bytes == 0 {
        return None;
    }
    let class = match read_trim(base.join("queue/rotational")).as_deref() {
        Some("1") => DeviceClassDto::Hdd,
        Some("0") => DeviceClassDto::Ssd,
        _ => DeviceClassDto::Unknown,
    };
    let removable = read_trim(base.join("removable"))
        .map(|s| s == "1")
        .unwrap_or(false);
    let model = read_trim(base.join("device/model")).filter(|s| !s.is_empty());
    Some(DeviceInfo {
        path: format!("/dev/{name}"),
        name,
        size_bytes,
        class,
        removable,
        model,
    })
}

#[cfg(target_os = "linux")]
fn is_virtual_block(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("dm-")
        || name.starts_with("zram")
        || name.starts_with("md")
}

#[cfg(target_os = "linux")]
fn read_trim<P: AsRef<std::path::Path>>(path: P) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}
