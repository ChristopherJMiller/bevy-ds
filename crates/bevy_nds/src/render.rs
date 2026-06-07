//! Tile-console rendering, expressed as an ECS extraction step.
//!
//! Full Bevy renders entities to the GPU via wgpu; that stack cannot run on the
//! DS. We keep the *shape* of that model — entities describe what to draw, a
//! system extracts them to the display each frame — but the "GPU" is the DS
//! text console (a tiled background) and the draw call is libnds `printf`.
//!
//! Drawables carry a [`TilePos`] (grid coordinate) and a [`DsScreen`], plus one
//! of:
//! - [`Glyph`] — a single character ("text sprite"), or
//! - [`DsText`] — a run of text.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_uint};

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

use crate::ffi;
use crate::screen::{Consoles, DsScreen};

/// A position on the 32x24 tile grid (0-based, origin top-left).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct TilePos {
    pub x: i16,
    pub y: i16,
}

impl TilePos {
    pub const fn new(x: i16, y: i16) -> Self {
        Self { x, y }
    }
}

/// A single character drawn at a [`TilePos`] — the DS analogue of a text sprite.
#[derive(Component, Clone, Copy)]
pub struct Glyph(pub u8);

/// A run of text drawn starting at a [`TilePos`].
#[derive(Component, Clone)]
pub struct DsText(pub String);

impl DsText {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

/// Move the console cursor to a 1-based (row, col) via an ANSI escape.
///
/// # Safety
/// A console must currently be selected.
unsafe fn move_cursor(pos: TilePos) {
    unsafe {
        ffi::printf(
            c"\x1b[%u;%uH".as_ptr(),
            (pos.y + 1) as c_uint,
            (pos.x + 1) as c_uint,
        );
    }
}

/// Clears both consoles and redraws every drawable entity onto its screen.
///
/// Runs in `Last`, after game systems have updated component state.
fn render(
    consoles: Res<Consoles>,
    glyphs: Query<(&DsScreen, &TilePos, &Glyph)>,
    texts: Query<(&DsScreen, &TilePos, &DsText)>,
) {
    unsafe {
        // Start each frame from a clean slate on both screens.
        ffi::consoleSelect(consoles.handle(DsScreen::Top));
        ffi::consoleClear();
        ffi::consoleSelect(consoles.handle(DsScreen::Bottom));
        ffi::consoleClear();

        for (screen, pos, glyph) in &glyphs {
            ffi::consoleSelect(consoles.handle(*screen));
            move_cursor(*pos);
            ffi::printf(c"%c".as_ptr(), glyph.0 as c_int);
        }

        for (screen, pos, text) in &texts {
            ffi::consoleSelect(consoles.handle(*screen));
            move_cursor(*pos);
            // printf needs a NUL-terminated string.
            let mut buf: Vec<u8> = Vec::with_capacity(text.0.len() + 1);
            buf.extend_from_slice(text.0.as_bytes());
            buf.push(0);
            ffi::printf(c"%s".as_ptr(), buf.as_ptr() as *const c_char);
        }
    }
}

/// Draws [`Glyph`] / [`DsText`] entities to the DS text consoles each frame.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, render);
    }
}
