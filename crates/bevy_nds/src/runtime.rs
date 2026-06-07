//! Bare-metal runtime glue required to run Rust on the DS ARM9 core: a global
//! allocator backed by newlib's heap, a panic handler, and the
//! `critical-section` implementation that Bevy's atomics build upon.
//!
//! These items must exist exactly once in the final binary. Because this crate
//! is statically linked into the (no_std) game, defining them here keeps the
//! game itself free of bare-metal boilerplate.

use core::ptr;

use crate::ffi;

/// Global allocator backed by newlib's heap (set up by the BlocksDS crt0).
struct NewlibAlloc;

unsafe impl core::alloc::GlobalAlloc for NewlibAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        // newlib guarantees 8-byte alignment; honour larger requests too.
        unsafe { ffi::memalign(layout.align().max(8), layout.size()) as *mut u8 }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        unsafe { ffi::free(ptr as *mut core::ffi::c_void) }
    }
}

#[global_allocator]
static ALLOCATOR: NewlibAlloc = NewlibAlloc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Best-effort: show that we panicked, then spin.
    unsafe {
        ffi::consoleClear();
        ffi::printf(c"PANIC".as_ptr());
    }
    let _ = info;
    loop {
        unsafe { ffi::swiWaitForVBlank() }
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
