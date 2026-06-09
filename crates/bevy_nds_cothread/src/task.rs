//! [`Task`] handle and the [`Tasks`] resource marker.
//!
//! The state shared between the main thread and the cothread is a single
//! heap-allocated `Inner` holding the closure (before it runs), the output
//! slot (after), and a `done` flag the cothread sets when it has finished
//! writing the output. The cothread receives a raw pointer to `Inner` as its
//! `void *arg`; the main thread polls `done` to know when the output is
//! ready.
//!
//! ## Why detached cothreads, not joinable
//!
//! Joinable threads must be cleaned up with `cothread_delete` from somewhere
//! after the thread joins. In a one-task game that delete is called by main
//! — but main is itself a cothread, and the libnds scheduler captures
//! `next_ctx = main->next = spawned` at the top of main's resume iteration.
//! When main returns control to the scheduler, the scheduler assigns
//! `ctx = next_ctx` and reads `ctx->next` — from freed memory. `free()`
//! clobbers the first words of the freed `cothread_info_t` with allocator
//! metadata, so the scheduler ends up resuming garbage. The user-visible
//! symptom is a hard freeze on the frame after the task completes.
//!
//! Detached threads avoid this entirely: the scheduler deletes them itself,
//! at a point where `next_ctx` is already the deleted thread's *successor*.
//! Communication happens through our own `Inner` box, which we free
//! ourselves after observing `done`.
//!
//! Host (non-DS) builds short-circuit: spawning runs the closure synchronously
//! and `Task::poll` returns the result on the first call. That keeps the
//! type-level surface testable without needing a cothread implementation on
//! the host.

use alloc::boxed::Box;
use bevy_ecs::prelude::*;
use core::marker::PhantomData;

/// Marker resource for the cothread task pool. Mirrors Bevy's
/// `AsyncComputeTaskPool` shape so call sites can read like ordinary Bevy:
///
/// ```ignore
/// fn start_load(tasks: Res<Tasks>) {
///     let _t = tasks.spawn(|| heavy_io());
/// }
/// ```
///
/// The resource carries no state — cothread is a global libnds facility — so
/// dropping or re-creating it is harmless.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Tasks;

impl Tasks {
    /// Equivalent to the free [`crate::spawn`].
    pub fn spawn<F, T>(&self, closure: F) -> Task<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        Task::spawn(closure)
    }
}

/// Handle to a value being produced on a cothread.
///
/// Poll with [`Task::poll`] each frame; when it returns `Some(value)` the
/// cothread has finished. Dropping an unfinished task blocks (yielding the
/// frame loop) until it finishes, so the boxed state can be reclaimed
/// safely.
pub struct Task<T: Send + 'static> {
    #[cfg(target_vendor = "nintendo")]
    state: ds::TaskState<T>,
    #[cfg(not(target_vendor = "nintendo"))]
    state: host::TaskState<T>,
    _phantom: PhantomData<T>,
}

impl<T: Send + 'static> Task<T> {
    pub(crate) fn spawn<F>(closure: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        Self {
            #[cfg(target_vendor = "nintendo")]
            state: ds::TaskState::spawn(closure),
            #[cfg(not(target_vendor = "nintendo"))]
            state: host::TaskState::spawn(closure),
            _phantom: PhantomData,
        }
    }

    /// Return `Some(value)` once the cothread has joined; otherwise `None`.
    /// Cheap to call every frame.
    pub fn poll(&mut self) -> Option<T> {
        self.state.poll()
    }

    /// True if the cothread has joined (its closure returned). After this
    /// flips to true, the *next* [`Task::poll`] yields the value.
    pub fn is_finished(&self) -> bool {
        self.state.is_finished()
    }

    /// Yield the frame loop until the cothread completes, then return the
    /// value. Use sparingly — the calling system stalls until done.
    pub fn block_on(mut self) -> T {
        loop {
            if let Some(v) = self.poll() {
                return v;
            }
            crate::yield_until_vblank();
        }
    }
}

impl<T: Send + 'static> Drop for Task<T> {
    fn drop(&mut self) {
        self.state.drop_in_place();
    }
}

// SAFETY: the DS is single-core and cothread scheduling is cooperative —
// a `Task<T>` can only be touched from one execution context at a time, so
// the usual `Send`/`Sync` data-race concerns don't apply. The bound on `T`
// keeps us honest if someone tries to send a `!Send` value out of a closure.
unsafe impl<T: Send + 'static> Send for Task<T> {}
unsafe impl<T: Send + 'static> Sync for Task<T> {}

// ---------------------------------------------------------------------------
// DS implementation: real cothread.
// ---------------------------------------------------------------------------

#[cfg(target_vendor = "nintendo")]
mod ds {
    use super::*;
    use crate::ffi;
    use core::ffi::{c_int, c_void};
    use core::ptr::NonNull;
    use portable_atomic::{AtomicBool, Ordering};

    /// Heap-allocated state shared between the main thread and the (detached)
    /// cothread. The cothread takes `closure` out, runs it, writes `output`,
    /// and **last** sets `done` — the main thread sees `done == true` only
    /// after the output write is visible.
    pub(super) struct Inner<T> {
        pub(super) closure: Option<Box<dyn FnOnce() -> T + Send + 'static>>,
        pub(super) output: Option<T>,
        pub(super) done: AtomicBool,
    }

    pub(super) struct TaskState<T: Send + 'static> {
        inner: NonNull<Inner<T>>,
    }

    impl<T: Send + 'static> TaskState<T> {
        pub(super) fn spawn<F>(closure: F) -> Self
        where
            F: FnOnce() -> T + Send + 'static,
        {
            let boxed: Box<dyn FnOnce() -> T + Send + 'static> = Box::new(closure);
            let inner = Box::into_raw(Box::new(Inner {
                closure: Some(boxed),
                output: None,
                done: AtomicBool::new(false),
            }));
            // Detached: when the closure returns, the libnds scheduler
            // deletes the cothread automatically. We never call
            // `cothread_delete` from outside the scheduler — see the
            // module-level "Why detached cothreads" note. The handle is
            // returned for completeness but we drop it on the floor.
            // SAFETY: `cothread_create` copies the function pointer + arg
            // into libnds bookkeeping. The Box stays live until our Drop.
            let thread = unsafe {
                ffi::cothread_create(
                    entrypoint::<T>,
                    inner as *mut c_void,
                    0,
                    ffi::COTHREAD_DETACHED,
                )
            };
            assert!(thread >= 0, "cothread_create failed (out of memory?)");
            Self {
                // SAFETY: `Box::into_raw` never returns null.
                inner: unsafe { NonNull::new_unchecked(inner) },
            }
        }

        pub(super) fn is_finished(&self) -> bool {
            // SAFETY: `inner` is live for as long as `self` exists.
            unsafe { (*self.inner.as_ptr()).done.load(Ordering::Acquire) }
        }

        pub(super) fn poll(&mut self) -> Option<T> {
            if !self.is_finished() {
                return None;
            }
            // SAFETY: `done == true` → cothread is past its last write of
            // `output` (and has either returned or is about to). We are now
            // the unique reader of `output`.
            unsafe { (*self.inner.as_ptr()).output.take() }
        }

        pub(super) fn drop_in_place(&mut self) {
            // The cothread may still be writing to `inner.output`. We must
            // wait until it has set `done` before freeing the box. Yielding
            // gives the scheduler — and the cothread — time to run.
            while !self.is_finished() {
                crate::yield_until_vblank();
            }
            // SAFETY: `done == true` → cothread no longer touches `inner`.
            // Its own stack/ctx are reaped by the libnds scheduler
            // (detached). We just free the box we allocated.
            unsafe {
                drop(Box::from_raw(self.inner.as_ptr()));
            }
        }
    }

    /// Trampoline that runs the boxed closure, writes its return into the
    /// shared `Inner`, and signals completion. One monomorphization per `T`.
    unsafe extern "C" fn entrypoint<T: Send + 'static>(arg: *mut c_void) -> c_int {
        // SAFETY: `arg` was set by `spawn` to a valid `*mut Inner<T>` and
        // stays live until we set `done == true` (after which the main
        // side may free it, but only after observing `done`, so the box is
        // valid for our entire run).
        let inner = arg as *mut Inner<T>;
        let closure = unsafe { (*inner).closure.take() }
            .expect("cothread entrypoint should run exactly once");
        let result = closure();
        unsafe {
            (*inner).output = Some(result);
            // Release: pairs with the Acquire load in `is_finished`. Ensures
            // the `output` write is visible to the main thread before it
            // observes `done == true`.
            (*inner).done.store(true, Ordering::Release);
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Host implementation: run the closure on spawn so unit tests can exercise
// the handle without a cothread runtime.
// ---------------------------------------------------------------------------

#[cfg(not(target_vendor = "nintendo"))]
mod host {
    use super::*;

    pub(super) struct TaskState<T: Send + 'static> {
        output: Option<T>,
    }

    impl<T: Send + 'static> TaskState<T> {
        pub(super) fn spawn<F>(closure: F) -> Self
        where
            F: FnOnce() -> T + Send + 'static,
        {
            // No cothread on the host — run synchronously. Tests of pure
            // handle logic still see the realistic "produces a value, then
            // polls return None" lifecycle.
            Self {
                output: Some(closure()),
            }
        }

        pub(super) fn is_finished(&self) -> bool {
            self.output.is_some()
        }

        pub(super) fn poll(&mut self) -> Option<T> {
            self.output.take()
        }

        pub(super) fn drop_in_place(&mut self) {
            // Nothing to clean up — the output (if any) drops with the
            // struct.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn task_yields_output_once() {
        let mut task = Task::<u32>::spawn(|| 7);
        assert!(task.is_finished());
        assert_eq!(task.poll(), Some(7));
        assert_eq!(
            task.poll(),
            None,
            "output should be moved out on first poll"
        );
    }

    #[test]
    fn task_holds_owned_value() {
        let mut task = Task::<Vec<u8>>::spawn(|| vec![1, 2, 3]);
        let out = task.poll().expect("ready immediately on host");
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn block_on_returns_value() {
        let task = Task::<&'static str>::spawn(|| "done");
        assert_eq!(task.block_on(), "done");
    }

    #[test]
    fn tasks_resource_spawn() {
        let tasks = Tasks;
        let mut t = tasks.spawn(|| 42i32);
        assert_eq!(t.poll(), Some(42));
    }
}
