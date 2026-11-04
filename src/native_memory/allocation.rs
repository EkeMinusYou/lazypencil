//! Release large Rust buffers directly instead of retaining freed malloc pages.
use std::alloc::{GlobalAlloc, Layout, System};

const MAPPED_THRESHOLD: usize = 16 * 1024 * 1024;

/// Application-only allocator; embedding the library keeps the host allocator.
pub(super) struct LargeBufferAllocator;

fn mapped(layout: Layout) -> bool {
    if layout.size() < MAPPED_THRESHOLD {
        return false;
    }
    // The page size is constant for the lifetime of every allocation.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    page > 0 && layout.align() <= page as usize
}

// Each allocation's immutable Layout selects exactly one matching deallocator.
unsafe impl GlobalAlloc for LargeBufferAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !mapped(layout) {
            return unsafe { System.alloc(layout) };
        }
        // Anonymous mappings supply zeroed pages without temporary-file I/O.
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                layout.size(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return std::ptr::null_mut();
        }
        if address.is_null() {
            unsafe { libc::munmap(address, layout.size()) };
        }
        address.cast()
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            unsafe { self.alloc(layout) }
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if mapped(layout) {
            unsafe { libc::munmap(pointer.cast(), layout.size()) };
        } else {
            unsafe { System.dealloc(pointer, layout) };
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let Ok(next_layout) = Layout::from_size_align(size, layout.align()) else {
            return std::ptr::null_mut();
        };
        if !mapped(layout) && !mapped(next_layout) {
            return unsafe { System.realloc(pointer, layout, size) };
        }
        let next = unsafe { self.alloc(next_layout) };
        if !next.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(pointer, next, layout.size().min(size));
                self.dealloc(pointer, layout);
            }
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroed_buffers_preserve_alignment_and_bytes_across_allocator_boundaries() {
        for align in [1, 64, 4096, 16384, 32768] {
            let mut layout = Layout::from_size_align(MAPPED_THRESHOLD + 33, align).unwrap();
            let mut pointer = unsafe { LargeBufferAllocator.alloc_zeroed(layout) };
            assert!(!pointer.is_null());
            assert_eq!(pointer as usize % align, 0);
            let bytes = unsafe { std::slice::from_raw_parts_mut(pointer, layout.size()) };
            assert!(bytes.iter().all(|byte| *byte == 0));
            bytes.fill(173);
            for size in [
                MAPPED_THRESHOLD * 2 + 7,
                MAPPED_THRESHOLD - 13,
                MAPPED_THRESHOLD + 17,
            ] {
                let next = unsafe { LargeBufferAllocator.realloc(pointer, layout, size) };
                assert!(!next.is_null());
                assert_eq!(next as usize % align, 0);
                let bytes = unsafe { std::slice::from_raw_parts_mut(next, size) };
                assert!(
                    bytes[..layout.size().min(size)]
                        .iter()
                        .all(|byte| *byte == 173)
                );
                bytes.fill(173);
                pointer = next;
                layout = Layout::from_size_align(size, align).unwrap();
            }
            unsafe { LargeBufferAllocator.dealloc(pointer, layout) };
        }
    }

    #[test]
    fn large_buffers_can_be_read_resized_and_freed_on_another_thread() {
        let layout = Layout::from_size_align(MAPPED_THRESHOLD + 97, 64).unwrap();
        let pointer = unsafe { LargeBufferAllocator.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe { std::slice::from_raw_parts_mut(pointer, layout.size()) }.fill(91);
        // Transfer exclusive ownership; the allocating thread never reads it again.
        let address = pointer as usize;
        std::thread::spawn(move || unsafe {
            let pointer = address as *mut u8;
            assert!(
                std::slice::from_raw_parts(pointer, layout.size())
                    .iter()
                    .all(|byte| *byte == 91)
            );
            let size = MAPPED_THRESHOLD - 1;
            let next = LargeBufferAllocator.realloc(pointer, layout, size);
            assert!(!next.is_null());
            assert!(
                std::slice::from_raw_parts(next, size)
                    .iter()
                    .all(|byte| *byte == 91)
            );
            LargeBufferAllocator.dealloc(next, Layout::from_size_align(size, 64).unwrap());
        })
        .join()
        .unwrap();
    }
}
