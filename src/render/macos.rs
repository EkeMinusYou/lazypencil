//! Reclaim freed native scratch after the complete PNG or preview conversion.
//!
//! macOS 15 diagnostics showed resident, mostly clean medium malloc regions
//! after all rendering finished. Release only unused allocator pages, retaining
//! live font/layout/runtime data and the returned PNG or preview pixels.
pub(super) struct ReleaseScratch;

impl Drop for ReleaseScratch {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: Apple's public API accepts NULL for all zones and zero for
        // maximal relief, synchronizes zone access and preserves live memory.
        // The caller declares this guard before the raster and copy/encoding
        // temporaries, so those have been dropped before the request runs.
        // https://github.com/apple-oss-distributions/libmalloc/blob/main/include/malloc/malloc.h
        unsafe {
            malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        }
    }
}
