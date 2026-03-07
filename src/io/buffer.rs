use std::alloc::{alloc_zeroed, dealloc, Layout};

use super::{BUFFER_SIZE, SECTOR_SIZE};

pub struct AlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
    size: usize,
}

impl AlignedBuffer {
    pub fn new() -> Self {
        Self::with_size(BUFFER_SIZE)
    }

    pub fn with_size(size: usize) -> Self {
        let aligned_size = (size + SECTOR_SIZE - 1) & !(SECTOR_SIZE - 1);
        let layout = Layout::from_size_align(aligned_size, SECTOR_SIZE)
            .expect("Invalid layout for AlignedBuffer");

        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        Self {
            ptr,
            layout,
            size: aligned_size,
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Default for AlignedBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr, self.layout);
        }
    }
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}
