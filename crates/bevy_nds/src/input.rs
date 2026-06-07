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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_is_mapped_exactly_once() {
        // All 12 variants appear, with no duplicate buttons.
        assert_eq!(DsButton::ALL.len(), 12);
        for i in 0..DsButton::ALL.len() {
            for j in (i + 1)..DsButton::ALL.len() {
                assert_ne!(DsButton::ALL[i].0, DsButton::ALL[j].0);
            }
        }
    }

    #[test]
    fn key_masks_are_single_distinct_bits() {
        let mut seen = 0u32;
        for (_, mask) in DsButton::ALL {
            assert!(mask != 0, "mask must be non-zero");
            assert_eq!(mask & (mask - 1), 0, "mask must be a single bit");
            assert_eq!(seen & mask, 0, "masks must be disjoint");
            seen |= mask;
        }
    }

    #[test]
    fn directional_masks_match_libnds() {
        let mask = |b: DsButton| DsButton::ALL.iter().find(|(x, _)| *x == b).unwrap().1;
        assert_eq!(mask(DsButton::Left), ffi::KEY_LEFT);
        assert_eq!(mask(DsButton::Right), ffi::KEY_RIGHT);
        assert_eq!(mask(DsButton::Up), ffi::KEY_UP);
        assert_eq!(mask(DsButton::Down), ffi::KEY_DOWN);
    }
}
