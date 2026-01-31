use argos::devices::{device_selection_options, format_device_table};
use argos::types::{BlockDevice, DeviceType};

#[test]
fn test_format_device_table() {
    let devices = vec![
        BlockDevice {
            name: "sda".to_string(),
            device_type: DeviceType::Hdd,
            size: 1_000_000_000_000,
            path: "/dev/sda".to_string(),
        },
        BlockDevice {
            name: "nvme0n1".to_string(),
            device_type: DeviceType::NVMe,
            size: 500_000_000_000,
            path: "/dev/nvme0n1".to_string(),
        },
    ];

    let table = format_device_table(&devices);
    assert!(table.contains("sda"));
    assert!(table.contains("nvme0n1"));
    assert!(table.contains("HDD"));
    assert!(table.contains("NVMe"));
}

#[test]
fn test_device_selection_options() {
    let devices = vec![BlockDevice {
        name: "sda".to_string(),
        device_type: DeviceType::Hdd,
        size: 1_000_000_000_000,
        path: "/dev/sda".to_string(),
    }];

    let options = device_selection_options(&devices);
    assert_eq!(options.len(), 1);
    assert!(options[0].contains("/dev/sda"));
    assert!(options[0].contains("HDD"));
}
