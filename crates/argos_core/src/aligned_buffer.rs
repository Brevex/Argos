use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ops::{Deref, DerefMut};

pub const DEFAULT_ALIGNMENT: usize = 4096;

pub struct AlignedBuffer {
    ptr: *mut u8,
    size: usize,
    capacity: usize,
    layout: Layout,
}

impl AlignedBuffer {
    pub fn new(size: usize, alignment: usize) -> Self {
        assert!(size > 0, "Buffer size must be greater than 0");
        assert!(
            alignment.is_power_of_two(),
            "Alignment must be a power of 2"
        );

        let aligned_size = (size + alignment - 1) & !(alignment - 1);

        let layout = Layout::from_size_align(aligned_size, alignment)
            .expect("Invalid layout for aligned allocation");

        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            panic!(
                "Failed to allocate {} bytes with {} alignment",
                aligned_size, alignment
            );
        }

        Self {
            ptr,
            size,
            capacity: aligned_size,
            layout,
        }
    }

    #[inline]
    pub fn new_default(size: usize) -> Self {
        Self::new(size, DEFAULT_ALIGNMENT)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    pub fn alignment(&self) -> usize {
        self.layout.align()
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
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
    pub fn clear(&mut self) {
        unsafe {
            std::ptr::write_bytes(self.ptr, 0, self.size);
        }
    }

    #[inline]
    pub fn is_aligned(&self) -> bool {
        (self.ptr as usize).is_multiple_of(self.layout.align())
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                dealloc(self.ptr, self.layout);
            }
        }
    }
}

impl Deref for AlignedBuffer {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for AlignedBuffer {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

pub struct AlignedBufferPool {
    buffer: AlignedBuffer,
    high_water_mark: usize,
}

impl AlignedBufferPool {
    pub fn new(initial_size: usize) -> Self {
        Self {
            buffer: AlignedBuffer::new_default(initial_size),
            high_water_mark: 0,
        }
    }

    pub fn get(&mut self, size: usize) -> &mut [u8] {
        if size > self.buffer.capacity() {
            self.buffer = AlignedBuffer::new_default(size);
        }

        self.high_water_mark = self.high_water_mark.max(size);

        let slice = &mut self.buffer.as_mut_slice()[..size];
        slice.fill(0);
        slice
    }

    #[inline]
    pub fn high_water_mark(&self) -> usize {
        self.high_water_mark
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aligned_buffer_creation() {
        let buffer = AlignedBuffer::new(1024, 4096);
        assert_eq!(buffer.len(), 1024);
        assert!(buffer.capacity() >= 1024);
        assert!(buffer.is_aligned());
        assert_eq!(buffer.as_ptr() as usize % 4096, 0);
    }

    #[test]
    fn test_aligned_buffer_contents() {
        let mut buffer = AlignedBuffer::new(1024, 4096);

        assert!(buffer.iter().all(|&b| b == 0));

        buffer[0] = 0xFF;
        buffer[1] = 0xD8;
        assert_eq!(buffer[0], 0xFF);
        assert_eq!(buffer[1], 0xD8);
    }

    #[test]
    fn test_aligned_buffer_clear() {
        let mut buffer = AlignedBuffer::new(1024, 4096);
        buffer[0] = 0xFF;
        buffer[100] = 0xAB;

        buffer.clear();

        assert!(buffer.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_aligned_buffer_default() {
        let buffer = AlignedBuffer::new_default(65536);
        assert_eq!(buffer.len(), 65536);
        assert_eq!(buffer.alignment(), 4096);
    }

    #[test]
    fn test_buffer_pool() {
        let mut pool = AlignedBufferPool::new(4096);

        let slice = pool.get(1024);
        assert_eq!(slice.len(), 1024);

        let slice = pool.get(8192);
        assert_eq!(slice.len(), 8192);

        assert_eq!(pool.high_water_mark(), 8192);
    }

    #[test]
    #[should_panic(expected = "Buffer size must be greater than 0")]
    fn test_zero_size_panics() {
        let _ = AlignedBuffer::new(0, 4096);
    }

    #[test]
    #[should_panic(expected = "Alignment must be a power of 2")]
    fn test_non_power_of_two_alignment_panics() {
        let _ = AlignedBuffer::new(1024, 1000);
    }
}
