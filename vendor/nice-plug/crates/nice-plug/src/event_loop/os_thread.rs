//! The calling OS thread's identity, read without allocating, locking or touching thread-local
//! storage, so `is_main_thread()` is safe to call from `process()`. Vendored fork, enth patch 10.
//!
//! nice-plug compared `std::thread::current().id()` with the id saved at construction, under a
//! `permit_alloc` and a FIXME. That allocates the first time it runs on a thread, and a plugin's
//! audio thread is the host's, so the first GUI task scheduled from `process()` paid for it:
//! `std::thread::current()` reads a thread local, and the first access to any thread local of a
//! `dlopen`ed library makes glibc allocate that library's TLS block for the calling thread
//! (`__tls_get_addr` -> `tls_get_addr_tail`, which takes `dl_load_tls_lock` and calls `malloc`;
//! glibc `elf/dl-tls.c`). That is what RealtimeSanitizer caught. macOS creates a dylib's
//! thread-local variables lazily as well, so a `thread_local!` of our own would not help either.
//!
//! The OS keeps the identity in the thread's control block, which these read directly:
//!
//! - Unix (Linux, macOS, the BSDs): `pthread_self()`, compared with `pthread_equal()`. glibc returns
//!   `THREAD_SELF`, a load relative to the thread pointer (`%fs` on x86-64, `tpidr_el0` on AArch64;
//!   `nptl/pthread_self.c`, `sysdeps/{x86_64,aarch64}/nptl/tls.h`), and `pthread_equal()` is `==`
//!   (`nptl/pthread_equal.c`). macOS's libpthread reads TSD slot 0 and checks the pointer's
//!   signature (`src/pthread_tsd.c`, `_pthread_self_direct()`).
//! - Windows: `GetCurrentThreadId()`, the `ClientId.UniqueThread` of the thread environment block.
//!
//! The one difference from `std::thread::ThreadId`: an OS thread identity can be handed to a new
//! thread after its thread exits. The identity saved here is the host's main thread (the thread that
//! created the wrapper or event loop), which outlives the plugin instance in a working host.

/// The identity of an OS thread, for comparing with the calling thread's.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OsThreadId(imp::Raw);

impl OsThreadId {
    /// The calling thread's identity. Never allocates, locks or touches thread-local storage.
    #[inline]
    pub fn current() -> Self {
        Self(imp::current())
    }

    /// Whether the calling thread is this thread. Never allocates, locks or touches thread-local
    /// storage.
    #[inline]
    pub fn is_current(&self) -> bool {
        imp::same(self.0, imp::current())
    }
}

/// Whether the calling thread is the process's main thread (`+[NSThread isMainThread]`, without
/// the Objective-C message: a method-cache miss in `objc_msgSend` takes the runtime lock and may
/// allocate). Never allocates, locks or touches thread-local storage.
#[cfg(target_os = "macos")]
#[inline]
pub(crate) fn is_process_main_thread() -> bool {
    // SAFETY: `pthread_main_np()` takes no arguments and only compares `pthread_self()` with the
    // main thread's `pthread_t` (libpthread `src/pthread.c`).
    unsafe { imp::pthread_main_np() != 0 }
}

#[cfg(all(target_family = "unix", not(target_os = "macos")))]
mod imp {
    #[derive(Clone, Copy, Debug)]
    pub struct Raw(libc::pthread_t);

    // SAFETY: the value is only ever compared, never dereferenced. On some Unixes `pthread_t` is a
    // pointer, which is what makes this impl necessary there.
    unsafe impl Send for Raw {}
    unsafe impl Sync for Raw {}

    #[inline]
    pub fn current() -> Raw {
        // SAFETY: `pthread_self()` always succeeds and has no preconditions.
        Raw(unsafe { libc::pthread_self() })
    }

    #[inline]
    pub fn same(a: Raw, b: Raw) -> bool {
        // SAFETY: `pthread_equal()` has no preconditions; both values came from `pthread_self()`.
        unsafe { libc::pthread_equal(a.0, b.0) != 0 }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_int, c_void};

    // libSystem, which every macOS binary links. nice-plug has no `libc` dependency on macOS.
    unsafe extern "C" {
        fn pthread_self() -> *mut c_void;
        fn pthread_equal(t1: *mut c_void, t2: *mut c_void) -> c_int;
        pub fn pthread_main_np() -> c_int;
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Raw(*mut c_void);

    // SAFETY: the value is only ever compared, never dereferenced.
    unsafe impl Send for Raw {}
    unsafe impl Sync for Raw {}

    #[inline]
    pub fn current() -> Raw {
        // SAFETY: `pthread_self()` always succeeds and has no preconditions.
        Raw(unsafe { pthread_self() })
    }

    #[inline]
    pub fn same(a: Raw, b: Raw) -> bool {
        // SAFETY: `pthread_equal()` has no preconditions; both values came from `pthread_self()`.
        unsafe { pthread_equal(a.0, b.0) != 0 }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    // kernel32, which every Windows binary links (the `windows` crate's `GetCurrentThreadId` needs a
    // feature nice-plug does not enable).
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Raw(u32);

    #[inline]
    pub fn current() -> Raw {
        // SAFETY: `GetCurrentThreadId()` always succeeds and has no preconditions.
        Raw(unsafe { GetCurrentThreadId() })
    }

    #[inline]
    pub fn same(a: Raw, b: Raw) -> bool {
        a.0 == b.0
    }
}

#[cfg(test)]
mod tests {
    use super::OsThreadId;

    #[test]
    fn the_calling_thread_is_current_and_another_is_not() {
        let here = OsThreadId::current();
        assert!(here.is_current());
        let elsewhere = std::thread::spawn(move || (here.is_current(), OsThreadId::current()))
            .join()
            .unwrap();
        assert!(!elsewhere.0);
        assert!(!elsewhere.1.is_current());
    }
}
