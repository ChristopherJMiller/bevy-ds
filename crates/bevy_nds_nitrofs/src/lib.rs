//! ROM filesystem (NitroFS) mounting & file I/O for `bevy_nds`.
//!
//! The DS has no asset server; at runtime an asset is just *bytes at an address*,
//! read from the read-only filesystem packed into the ROM (the `nitro:/` drive).
//! [`NitroFsPlugin`] mounts that filesystem in [`PreStartup`] and inserts a
//! [`NitroFs`] resource recording whether the mount succeeded. Other subsystems
//! (3D model loading, audio soundbank, future sprite assets) all depend on this
//! crate so the mount happens exactly once.
//!
//! Use [`read_file`] to slurp a whole file into a `Vec<u8>`; if the bytes are
//! going to DMA, call [`flush_dcache`] on them first.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use core::ffi::{c_char, c_int, c_long, c_void};

use alloc::vec::Vec;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

#[allow(non_snake_case)]
unsafe extern "C" {
    /// Mount the ROM filesystem (NitroFS) so files can be read from `nitro:/`.
    /// Pass null to use the current ROM. Returns non-zero on success. Safe to
    /// call again if another component already mounted it. See `<filesystem.h>`.
    fn nitroFSInit(basepath: *const c_char) -> bool;

    /// Clean and flush a range of the ARM9 data cache to main memory, so a
    /// subsequent DMA read sees the CPU's writes. This is libnds'
    /// `DC_FlushRange` (a `static inline` wrapper, so we bind the underlying
    /// symbol). See `<nds/arm9/cache.h>`.
    fn CP15_CleanAndFlushDCacheRange(base: *const c_void, size: u32);

    // Minimal newlib `stdio.h` surface for reading NitroFS files at runtime.
    // The DS has no asset server, so runtime-loaded assets are read with plain
    // C file I/O from the `nitro:/` drive (resolved by NitroFS). See `<stdio.h>`.
    fn fopen(path: *const u8, mode: *const u8) -> *mut c_void;
    fn fclose(stream: *mut c_void) -> c_int;
    fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
    fn fseek(stream: *mut c_void, offset: c_long, whence: c_int) -> c_int;
    fn ftell(stream: *mut c_void) -> c_long;
    fn rewind(stream: *mut c_void);
}

/// `SEEK_END` from `<stdio.h>`.
const SEEK_END: c_int = 2;

/// Records whether the ROM filesystem mounted successfully. When `ready` is
/// `false`, [`read_file`] always returns `None` — the loader probably didn't
/// supply `argv[0]`/DLDI (emulators always work).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct NitroFs {
    pub ready: bool,
}

/// Mounts the ROM filesystem (NitroFS) in [`PreStartup`] so assets can be
/// loaded at runtime from `nitro:/`. Add it before any [`Startup`] system that
/// reads files. The [`NitroFs`] resource records whether the mount succeeded.
pub struct NitroFsPlugin;

impl Plugin for NitroFsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NitroFs>()
            .add_systems(PreStartup, init_nitrofs);
    }
}

/// PreStartup system that mounts the ROM filesystem. Exposed so other crates
/// can order their own PreStartup work after it (`.after(init_nitrofs)`).
pub fn init_nitrofs(mut nitrofs: ResMut<NitroFs>) {
    nitrofs.ready = unsafe { nitroFSInit(core::ptr::null()) };
}

/// Read an entire file from the filesystem into a byte buffer.
///
/// `path` is a NUL-terminated C string (e.g. `b"nitro:/teapot.dl\0"`).
/// Returns `None` if the file can't be opened or read. Uses CPU copies
/// (`fread`), so no cache flush is needed for the returned `Vec` itself;
/// callers that hand the data to DMA must flush it first (see [`flush_dcache`]).
pub fn read_file(path: &[u8]) -> Option<Vec<u8>> {
    debug_assert!(path.last() == Some(&0), "path must be NUL-terminated");
    unsafe {
        let f = fopen(path.as_ptr(), b"rb\0".as_ptr());
        if f.is_null() {
            return None;
        }
        let ok = (|| {
            if fseek(f, 0, SEEK_END) != 0 {
                return None;
            }
            let size = ftell(f);
            if size <= 0 {
                return None;
            }
            rewind(f);
            let size = size as usize;
            let mut buf = Vec::<u8>::with_capacity(size);
            let read = fread(buf.as_mut_ptr() as *mut c_void, 1, size, f);
            if read != size {
                return None;
            }
            buf.set_len(size);
            Some(buf)
        })();
        fclose(f);
        ok
    }
}

/// Clean+flush a buffer from the data cache to main RAM so a following DMA
/// read (e.g. `glCallList` on a heap display list) sees current contents.
pub fn flush_dcache(bytes: &[u8]) {
    unsafe {
        CP15_CleanAndFlushDCacheRange(bytes.as_ptr() as *const c_void, bytes.len() as u32);
    }
}
