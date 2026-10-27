//! Return freed raster scratch to the OS after a render, including cancellation.
//!
//! macOS may retain large, already freed surfaces in its malloc zones. The
//! returned image and retained layout/font/runtime data still own their memory;
//! this best-effort request only releases unused allocator pages.

pub(super) struct ReleaseScratch;

impl Drop for ReleaseScratch {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: Apple's public malloc API accepts NULL for all zones and 0
        // for maximal pressure relief. It manages its own zone synchronization
        // and never releases live allocations. The guard is declared before
        // raster owners, so their scratch has been dropped before this call.
        // https://github.com/apple-oss-distributions/libmalloc/blob/main/include/malloc/malloc.h
        unsafe {
            malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        }
    }
}
