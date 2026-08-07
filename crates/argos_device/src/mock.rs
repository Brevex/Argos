//! Mocked device core (`test-util` only).
//!
//! [`Ctrl`] is returned by [`Device::new_mocked`](crate::Device::new_mocked) and
//! scripts the mocked device while counting the reads it serves; the device and
//! its controller share one state by construction.

use std::sync::{Arc, Mutex};

use argos_core::fixture::MemDisk;
use argos_core::geometry::Lba;
use argos_core::source::{Geometry, ReadError};

/// Controller for a mocked [`Device`](crate::Device).
#[derive(Clone, Debug)]
pub struct Ctrl {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    disk: MemDisk,
    reads: u64,
}

impl Ctrl {
    pub(crate) fn new(disk: MemDisk) -> Self {
        Self {
            state: Arc::new(Mutex::new(State { disk, reads: 0 })),
        }
    }

    /// Replaces the backing disk.
    pub fn set_disk(&self, disk: MemDisk) {
        self.lock().disk = disk;
    }

    /// Number of `read_at` calls the mocked device has served.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.lock().reads
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.lock().disk.geometry()
    }

    pub(crate) fn read_at(&self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        let mut state = self.lock();
        state.reads += 1;
        state.disk.read_at(lba, buf)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect(
            "mock state lock poisoned: a previous mock operation panicked while \
                 holding it (e.g. the buffer-contract assert in MemDisk::read_at)",
        )
    }
}
