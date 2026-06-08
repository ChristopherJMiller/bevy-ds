//! Bare-metal runtime glue + frame loop for running Bevy on the DS ARM9.
//!
//! Three responsibilities, one crate:
//!
//! 1. **Bare-metal items** — `#[global_allocator]` (newlib heap), `#[panic_handler]`,
//!    and a `critical-section` impl (the single-core "disable interrupts" pattern
//!    Bevy's atomics build on). These must appear exactly once in the final
//!    binary, so a single crate provides them all.
//! 2. **The frame loop** — [`ds_runner`] is installed as the Bevy app runner via
//!    [`run`]; it blocks on the vertical-blank interrupt at ~60 Hz and calls
//!    `App::update`.
//! 3. **A bit of supporting FFI** — `swiWaitForVBlank`, `memalign`/`free`, and a
//!    minimal `consoleClear` + `printf` for the panic message.
//!
//! When the target vendor is not `nintendo` (i.e. building for the host to run
//! unit tests of a dependent crate), the bare-metal items are inert so they
//! don't clash with `std`.

#![cfg_attr(not(test), no_std)]

use bevy_app::{App, AppExit};

/// The DS frame loop. Never returns — there is nothing to exit *to*.
#[cfg(target_vendor = "nintendo")]
fn ds_runner(mut app: App) -> AppExit {
    // Finish plugin setup, then run forever, one `update` per display refresh.
    app.finish();
    app.cleanup();
    loop {
        unsafe { swiWaitForVBlank() };
        app.update();
    }
}

/// Stub used when building for the host (tests of dependent crates). Should
/// never actually be installed because tests don't run `App::run()`, but we
/// need *something* to type-check.
#[cfg(not(target_vendor = "nintendo"))]
fn ds_runner(_app: App) -> AppExit {
    AppExit::Success
}

/// Installs the DS runner and starts the frame loop. Does not return.
pub fn run(mut app: App) -> ! {
    app.set_runner(ds_runner);
    app.run();
    // The runner loops forever, so control never reaches here.
    #[allow(clippy::empty_loop)]
    loop {}
}

// --- DS-only bare-metal items -------------------------------------------------

#[cfg(target_vendor = "nintendo")]
mod bare_metal {
    use core::ffi::{c_char, c_int, c_void};
    use core::ptr;

    unsafe extern "C" {
        /// newlib aligned allocation, backing our global allocator.
        fn memalign(align: usize, size: usize) -> *mut c_void;
        /// newlib free.
        fn free(ptr: *mut c_void);

        // Minimal console FFI for the panic handler's best-effort message.
        fn consoleClear();
        fn printf(fmt: *const c_char, ...) -> c_int;

        /// Block until the next vertical blank (~60 Hz), pacing the game loop.
        pub(super) fn swiWaitForVBlank();
    }

    /// Global allocator backed by newlib's heap (set up by the BlocksDS crt0).
    struct NewlibAlloc;

    unsafe impl core::alloc::GlobalAlloc for NewlibAlloc {
        unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
            // newlib guarantees 8-byte alignment; honour larger requests too.
            unsafe { memalign(layout.align().max(8), layout.size()) as *mut u8 }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
            unsafe { free(ptr as *mut c_void) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: NewlibAlloc = NewlibAlloc;

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        // Best-effort: show that we panicked, then spin.
        unsafe {
            consoleClear();
            printf(c"PANIC".as_ptr());
        }
        let _ = info;
        loop {
            unsafe { swiWaitForVBlank() }
        }
    }

    /// DS interrupt master enable register (single 32-bit MMIO word).
    const REG_IME: *mut u32 = 0x0400_0208 as *mut u32;

    /// Single-core critical section: disable interrupts on acquire, restore on
    /// release. This is what Bevy's `critical-section` feature builds upon.
    struct DsCriticalSection;
    critical_section::set_impl!(DsCriticalSection);

    unsafe impl critical_section::Impl for DsCriticalSection {
        unsafe fn acquire() -> bool {
            unsafe {
                let was_enabled = ptr::read_volatile(REG_IME) & 1 != 0;
                ptr::write_volatile(REG_IME, 0);
                was_enabled
            }
        }

        unsafe fn release(was_enabled: bool) {
            if was_enabled {
                unsafe { ptr::write_volatile(REG_IME, 1) }
            }
        }
    }
}

#[cfg(target_vendor = "nintendo")]
use bare_metal::swiWaitForVBlank;
