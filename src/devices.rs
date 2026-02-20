use crate::types::{BlockDevice, DeviceType};
use std::fs;
use std::path::Path;

pub fn discover_block_devices() -> Vec<BlockDevice> {
    #[cfg(target_os = "linux")]
    return discover_linux_devices();

    #[cfg(target_os = "windows")]
    return discover_windows_devices();

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return Vec::new();
}

#[cfg(target_os = "linux")]
fn discover_linux_devices() -> Vec<BlockDevice> {
    let mut devices = Vec::new();

    let sys_block = Path::new("/sys/block");
    if !sys_block.exists() {
        return devices;
    }

    if let Ok(entries) = fs::read_dir(sys_block) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("dm-") {
                continue;
            }

            if let Some(device) = parse_linux_device(&name) {
                devices.push(device);
            }
        }
    }
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

#[cfg(target_os = "linux")]
fn parse_linux_device(name: &str) -> Option<BlockDevice> {
    let sys_path = format!("/sys/block/{}", name);
    let dev_path = format!("/dev/{}", name);
    let size_path = format!("{}/size", sys_path);

    let size = fs::read_to_string(&size_path)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        * 512;

    if size == 0 {
        return None;
    }

    let device_type = detect_linux_device_type(name, &sys_path);

    Some(BlockDevice {
        name: name.to_string(),
        device_type,
        size,
        path: dev_path,
    })
}

#[cfg(target_os = "linux")]
fn detect_linux_device_type(name: &str, sys_path: &str) -> DeviceType {
    if name.starts_with("nvme") {
        return DeviceType::NVMe;
    }

    let removable_path = format!("{}/removable", sys_path);

    if let Ok(removable) = fs::read_to_string(&removable_path) {
        if removable.trim() == "1" {
            return DeviceType::Usb;
        }
    }

    let rotational_path = format!("{}/queue/rotational", sys_path);
    if let Ok(rotational) = fs::read_to_string(&rotational_path) {
        match rotational.trim() {
            "1" => return DeviceType::Hdd,
            "0" => return DeviceType::Ssd,
            _ => {}
        }
    }

    DeviceType::Unknown
}

#[cfg(target_os = "windows")]
fn discover_windows_devices() -> Vec<BlockDevice> {
    let mut devices = Vec::new();

    for i in 0..16 {
        let path = format!("\\\\.\\PhysicalDrive{}", i);

        if let Some(device) = parse_windows_device(i, &path) {
            devices.push(device);
        }
    }

    devices
}

#[cfg(target_os = "windows")]
fn parse_windows_device(index: usize, path: &str) -> Option<BlockDevice> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(0x80000000)
        .open(path)
        .ok()?;

    let metadata = file.metadata().ok()?;
    let size = metadata.len();

    if size == 0 {
        return None;
    }

    Some(BlockDevice {
        name: format!("PhysicalDrive{}", index),
        device_type: DeviceType::Unknown,
        size,
        path: path.to_string(),
    })
}

pub fn format_device_table(devices: &[BlockDevice]) -> String {
    let mut output = String::new();

    output.push_str("NAME       TYPE       SIZE PATH\n");
    output.push_str("---------------------------------------------\n");

    for device in devices {
        output.push_str(&format!(
            "{:<10} {:<8} {:>10} {}\n",
            device.name,
            format!("{}", device.device_type),
            device.size_human(),
            device.path
        ));
    }

    output
}

pub fn device_selection_options(devices: &[BlockDevice]) -> Vec<String> {
    devices
        .iter()
        .map(|d| format!("{} ({}) - {}", d.path, d.device_type, d.size_human()))
        .collect()
}
