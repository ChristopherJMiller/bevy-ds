//! Cooperative threads (libnds **cothread**) wrapped as a Bevy-friendly task
//! handle.
//!
//! The DS frame loop in [`bevy_nds_runtime`] is strictly synchronous —
//! any blocking work (NitroFS read, FAT write, WiFi request) inside a system
//! stalls vblank and drops the frame rate. libnds ships a cooperative
//! scheduler (`<nds/cothread.h>`) that lets that work move off the critical
//! path:
//!
//! - The runtime's per-frame wait is `cothread_yield_irq(IRQ_VBLANK)` rather
//!   than `swiWaitForVBlank()`. The semantics are identical when no tasks are
//!   spawned (block until the next vblank IRQ), but when tasks exist they get
//!   scheduling time during that wait.
//! - Spawned tasks run until they call [`yield_now`] or [`yield_until_vblank`]
//!   (or any libnds call that yields internally, like `cothread_yield_irq`).
//!   CPU-bound work that never yields still blocks — this is **cooperative**,
//!   not preemptive.
//!
//! ## Spawning a task
//!
//! ```ignore
//! use bevy_nds_cothread::spawn;
//!
//! let mut task = spawn(|| {
//!     // Blocking work (e.g. NitroFS read). Yields are inserted by libnds
//!     // syscalls; long CPU loops should sprinkle yield_now() calls.
//!     bevy_nds_nitrofs::read_file("nitro:/big.bin")
//! });
//!
//! // Each frame, poll for completion:
//! if let Some(bytes) = task.poll() {
//!     // …use bytes…
//! }
//! ```
//!
//! Dropping an unfinished [`Task`] blocks (yielding the frame loop) until the
//! cothread has joined, so the boxed task state can be freed safely. Detach
//! with [`Task::detach`] for fire-and-forget work that intentionally outlives
//! its handle.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use bevy_app::prelude::*;

mod task;

pub use task::{Task, Tasks};

/// libnds VBLANK IRQ mask (`<nds/interrupts.h>: IRQ_VBLANK = BIT(0)`).
pub const IRQ_VBLANK: u32 = 1 << 0;

/// Spawn `closure` on a new cothread. Returns a [`Task`] handle that systems
/// poll each frame for completion.
///
/// The cothread starts suspended; it runs the first time the main loop (or
/// some other task) yields.
pub fn spawn<F, T>(closure: F) -> Task<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Task::spawn(closure)
}

/// Tell the cothread scheduler to switch to a different ready thread, if any.
/// Returns immediately when called from the main thread with no other ready
/// tasks. Long-running CPU loops on a spawned task should call this
/// periodically so they don't starve the frame loop.
pub fn yield_now() {
    #[cfg(target_vendor = "nintendo")]
    unsafe {
        ffi::cothread_yield()
    };
}

/// Yield until the next vertical blank interrupt. This is the call the runtime
/// uses to pace the frame loop; tasks can use it to align work to vblank
/// boundaries (e.g. a streaming loader that reads one chunk per frame).
pub fn yield_until_vblank() {
    #[cfg(target_vendor = "nintendo")]
    unsafe {
        ffi::cothread_yield_irq(IRQ_VBLANK)
    };
}

/// Cothread-backed async work as a Bevy plugin. Currently a no-op (the
/// scheduler is initialized by libnds at boot and the vblank yield wiring
/// lives in [`bevy_nds_runtime`]) — but having a plugin makes the capability
/// discoverable through [`bevy_nds::DsPlugins`] and gives us a hook for any
/// future task-bookkeeping systems.
///
/// [`bevy_nds::DsPlugins`]: https://docs.rs/bevy_nds
pub struct CothreadPlugin;

impl Plugin for CothreadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tasks>();
    }
}

#[cfg(target_vendor = "nintendo")]
pub(crate) mod ffi {
    use core::ffi::{c_int, c_void};

    /// `cothread_entrypoint_t` — `int (*)(void *)` (`<nds/cothread.h>`).
    pub(crate) type CothreadEntrypoint = unsafe extern "C" fn(*mut c_void) -> c_int;

    /// `typedef int cothread_t;` — libnds thread ID.
    pub(crate) type Cothread = c_int;

    /// `COTHREAD_DETACHED` (`<nds/cothread_asm.h>`). A detached thread is
    /// deleted by the scheduler immediately on completion: the scheduler
    /// already has a valid `next_ctx` at that point, so it doesn't matter
    /// that the deleted thread is no longer in the list. Joinable threads
    /// would have to be deleted by the user via `cothread_delete`, which
    /// is a use-after-free against the scheduler's saved `next_ctx`
    /// whenever the deleting thread is the joinable one's predecessor in
    /// the list — exactly the situation we hit in a one-task Bevy game.
    pub(crate) const COTHREAD_DETACHED: u32 = 1 << 0;

    unsafe extern "C" {
        /// Create a thread; libnds owns the stack and frees it on
        /// `cothread_delete` (or auto-deletes if [`COTHREAD_DETACHED`] is
        /// set). `stack_size = 0` selects libnds's default (~1 KiB).
        /// (`<nds/cothread.h>`)
        pub(crate) fn cothread_create(
            entrypoint: CothreadEntrypoint,
            arg: *mut c_void,
            stack_size: usize,
            flags: u32,
        ) -> Cothread;

        /// Yield to another ready thread. (`<nds/cothread.h>`)
        pub(crate) fn cothread_yield();

        /// Yield until the specified IRQ fires. Equivalent to
        /// `swiIntrWait`+yield. (`<nds/cothread.h>`)
        pub(crate) fn cothread_yield_irq(flag: u32);
    }
}
