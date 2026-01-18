use super::{is_restart_marker, restart_marker_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartMarkerInfo {
    pub offset: usize,
    pub rst_number: u8,
}

#[derive(Debug, Clone, Default)]
pub struct RestartMarkerScanner {
    expected_next: u8,
}

impl RestartMarkerScanner {
    pub fn new() -> Self {
        Self { expected_next: 0 }
    }

    pub fn reset(&mut self) {
        self.expected_next = 0;
    }

    pub fn scan(&self, buffer: &[u8]) -> Vec<RestartMarkerInfo> {
        let mut markers = Vec::new();
        let mut i = 0;

        while i < buffer.len().saturating_sub(1) {
            if buffer[i] == 0xFF {
                let marker = buffer[i + 1];

                if marker == 0x00 {
                    i += 2;
                    continue;
                }

                if is_restart_marker(marker) {
                    if let Some(num) = restart_marker_index(marker) {
                        markers.push(RestartMarkerInfo {
                            offset: i,
                            rst_number: num,
                        });
                    }
                    i += 2;
                    continue;
                }

                if marker == 0xD9 {
                    break;
                }
            }

            i += 1;
        }

        markers
    }

    pub fn validate_sequence(&self, markers: &[RestartMarkerInfo]) -> bool {
        if markers.is_empty() {
            return true;
        }

        let mut expected = markers[0].rst_number;

        for (i, marker) in markers.iter().enumerate() {
            if i == 0 {
                continue;
            }

            expected = (expected + 1) % 8;
            if marker.rst_number != expected {
                return false;
            }
        }

        true
    }

    pub fn would_maintain_sequence(
        &self,
        existing_markers: &[RestartMarkerInfo],
        new_marker: RestartMarkerInfo,
    ) -> bool {
        if existing_markers.is_empty() {
            return true;
        }

        let last = existing_markers.last().unwrap();
        let expected_next = (last.rst_number + 1) % 8;

        new_marker.rst_number == expected_next
    }

    pub fn junction_score(
        &self,
        head_markers: &[RestartMarkerInfo],
        tail_markers: &[RestartMarkerInfo],
    ) -> f32 {
        if head_markers.is_empty() || tail_markers.is_empty() {
            return 0.5;
        }

        let last_head = head_markers.last().unwrap();
        let first_tail = tail_markers.first().unwrap();

        let expected_next = (last_head.rst_number + 1) % 8;

        if first_tail.rst_number == expected_next {
            1.0
        } else {
            let diff = (first_tail.rst_number as i8 - expected_next as i8).abs() as u8;
            let min_diff = diff.min(8 - diff);

            1.0 - (min_diff as f32 / 4.0)
        }
    }

    pub fn count_markers(&self, buffer: &[u8]) -> usize {
        let mut count = 0;
        let mut i = 0;

        while i < buffer.len().saturating_sub(1) {
            if buffer[i] == 0xFF && is_restart_marker(buffer[i + 1]) {
                count += 1;
                i += 2;
            } else {
                i += 1;
            }
        }

        count
    }

    pub fn find_first(&self, buffer: &[u8]) -> Option<RestartMarkerInfo> {
        let mut i = 0;

        while i < buffer.len().saturating_sub(1) {
            if buffer[i] == 0xFF {
                let marker = buffer[i + 1];

                if marker == 0x00 {
                    i += 2;
                    continue;
                }

                if is_restart_marker(marker) {
                    return restart_marker_index(marker).map(|num| RestartMarkerInfo {
                        offset: i,
                        rst_number: num,
                    });
                }
            }

            i += 1;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_no_markers() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let markers = scanner.scan(&data);
        assert!(markers.is_empty());
    }

    #[test]
    fn test_scan_single_marker() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![0x00, 0x11, 0xFF, 0xD0, 0x22, 0x33];
        let markers = scanner.scan(&data);

        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].offset, 2);
        assert_eq!(markers[0].rst_number, 0);
    }

    #[test]
    fn test_scan_multiple_markers() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![
            0xFF, 0xD0, 0x00, 0x00, 0xFF, 0xD1, 0x00, 0x00, 0xFF, 0xD2, 0x00, 0x00,
        ];
        let markers = scanner.scan(&data);

        assert_eq!(markers.len(), 3);
        assert_eq!(markers[0].rst_number, 0);
        assert_eq!(markers[1].rst_number, 1);
        assert_eq!(markers[2].rst_number, 2);
    }

    #[test]
    fn test_scan_ignores_stuffed_zeros() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![0xFF, 0x00, 0xFF, 0xD0, 0x00];
        let markers = scanner.scan(&data);

        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].offset, 2);
    }

    #[test]
    fn test_scan_stops_at_eoi() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![0xFF, 0xD0, 0xFF, 0xD9, 0xFF, 0xD1];
        let markers = scanner.scan(&data);

        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].rst_number, 0);
    }

    #[test]
    fn test_validate_sequence_valid() {
        let scanner = RestartMarkerScanner::new();
        let markers = vec![
            RestartMarkerInfo {
                offset: 0,
                rst_number: 0,
            },
            RestartMarkerInfo {
                offset: 10,
                rst_number: 1,
            },
            RestartMarkerInfo {
                offset: 20,
                rst_number: 2,
            },
            RestartMarkerInfo {
                offset: 30,
                rst_number: 3,
            },
        ];

        assert!(scanner.validate_sequence(&markers));
    }

    #[test]
    fn test_validate_sequence_wraps() {
        let scanner = RestartMarkerScanner::new();
        let markers = vec![
            RestartMarkerInfo {
                offset: 0,
                rst_number: 6,
            },
            RestartMarkerInfo {
                offset: 10,
                rst_number: 7,
            },
            RestartMarkerInfo {
                offset: 20,
                rst_number: 0,
            },
            RestartMarkerInfo {
                offset: 30,
                rst_number: 1,
            },
        ];

        assert!(scanner.validate_sequence(&markers));
    }

    #[test]
    fn test_validate_sequence_invalid() {
        let scanner = RestartMarkerScanner::new();
        let markers = vec![
            RestartMarkerInfo {
                offset: 0,
                rst_number: 0,
            },
            RestartMarkerInfo {
                offset: 10,
                rst_number: 1,
            },
            RestartMarkerInfo {
                offset: 20,
                rst_number: 5,
            },
        ];

        assert!(!scanner.validate_sequence(&markers));
    }

    #[test]
    fn test_junction_score_perfect() {
        let scanner = RestartMarkerScanner::new();
        let head = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 3,
        }];
        let tail = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 4,
        }];

        let score = scanner.junction_score(&head, &tail);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_junction_score_wrapping() {
        let scanner = RestartMarkerScanner::new();
        let head = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 7,
        }];
        let tail = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 0,
        }];

        let score = scanner.junction_score(&head, &tail);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_junction_score_misaligned() {
        let scanner = RestartMarkerScanner::new();
        let head = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 3,
        }];
        let tail = vec![RestartMarkerInfo {
            offset: 0,
            rst_number: 6,
        }];

        let score = scanner.junction_score(&head, &tail);
        assert!(score < 1.0);
    }

    #[test]
    fn test_count_markers() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![0xFF, 0xD0, 0x00, 0xFF, 0xD1, 0x00, 0xFF, 0xD2, 0x00];

        assert_eq!(scanner.count_markers(&data), 3);
    }

    #[test]
    fn test_find_first() {
        let scanner = RestartMarkerScanner::new();

        let data = vec![0x00, 0x11, 0xFF, 0xD5, 0x22];
        let first = scanner.find_first(&data);

        assert!(first.is_some());
        let marker = first.unwrap();
        assert_eq!(marker.offset, 2);
        assert_eq!(marker.rst_number, 5);
    }
}
