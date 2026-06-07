//! Nintendo DS buttons, surfaced through Bevy's standard input abstraction.
//!
//! Rather than inventing a bespoke input resource, we reuse [`ButtonInput`]
//! (the same type Bevy uses for keyboards, mice and gamepads). Game code reads
//! `Res<ButtonInput<DsButton>>` and gets `pressed` / `just_pressed` /
//! `just_released` for free.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_input::ButtonInput;

use crate::ffi;

/// A button on the Nintendo DS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DsButton {
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
}

impl DsButton {
    /// Every button paired with its libnds key mask.
    const ALL: [(DsButton, u32); 12] = [
        (DsButton::A, ffi::KEY_A),
        (DsButton::B, ffi::KEY_B),
        (DsButton::X, ffi::KEY_X),
        (DsButton::Y, ffi::KEY_Y),
        (DsButton::L, ffi::KEY_L),
        (DsButton::R, ffi::KEY_R),
        (DsButton::Start, ffi::KEY_START),
        (DsButton::Select, ffi::KEY_SELECT),
        (DsButton::Up, ffi::KEY_UP),
        (DsButton::Down, ffi::KEY_DOWN),
        (DsButton::Left, ffi::KEY_LEFT),
        (DsButton::Right, ffi::KEY_RIGHT),
    ];
}

/// Latches the hardware key state into the [`ButtonInput`] resource each frame,
/// driving its pressed / just-pressed / just-released bookkeeping.
fn read_keys(mut buttons: ResMut<ButtonInput<DsButton>>) {
    // Clear last frame's "just" transitions, then re-derive press state.
    buttons.clear();

    let held = unsafe {
        ffi::scanKeys();
        ffi::keysHeld()
    };

    for (button, mask) in DsButton::ALL {
        if held & mask != 0 {
            buttons.press(button);
        } else {
            buttons.release(button);
        }
    }
}

/// Exposes the DS buttons as a `ButtonInput<DsButton>` resource.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ButtonInput<DsButton>>()
            .add_systems(PreUpdate, read_keys);
    }
}
