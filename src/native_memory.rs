//! Bound temporary Objective-C objects to the native work that creates them.
pub(crate) struct AutoreleasePool {
    #[cfg(target_os = "macos")]
    token: *mut std::ffi::c_void,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(target_os = "macos")]
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut std::ffi::c_void;
    fn objc_autoreleasePoolPop(token: *mut std::ffi::c_void);
}

impl AutoreleasePool {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            // The thread-bound guard pops exactly this pool on every exit path.
            token: unsafe { objc_autoreleasePoolPush() },
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        // Skia objects returned from the scope own their native references.
        // Only autoreleased temporaries are drained here, on the creating thread.
        unsafe {
            objc_autoreleasePoolPop(self.token);
        }
    }
}
