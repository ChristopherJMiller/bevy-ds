//! A real-time clock that powers Bevy's standard [`Time`] resource.
//!
//! On desktop, `bevy_time` reads a wall clock. The DS has no `std` clock, but it
//! does have a free-running hardware timer at the bus clock (~33.51 MHz). We
//! start it once and, each frame, advance virtual time by the real number of
//! ticks elapsed since the previous frame. Game code can then use the ordinary
//! `Res<Time>` API (`elapsed_secs`, `delta_secs`, ...) and it reflects true
//! wall-clock time — including any frames that ran long.

use core::time::Duration;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::ffi;

/// DS bus clock in Hz (see `BUS_CLOCK` in libnds `timers.h`).
const BUS_CLOCK: u64 = 33_513_982;

/// Convert a span of hardware-timer ticks into nanoseconds. Pure helper so the
/// (overflow-sensitive) arithmetic can be unit-tested off-target.
fn ticks_to_nanos(delta_ticks: u32) -> u64 {
    delta_ticks as u64 * 1_000_000_000 / BUS_CLOCK
}

/// Last hardware-timer reading, used to compute per-frame deltas. The timer is
/// 32 bits and wraps about every 128 s, which `wrapping_sub` handles.
#[derive(Resource)]
struct HardwareClock {
    last_ticks: u32,
}

fn start_clock(mut commands: Commands) {
    let last_ticks = unsafe {
        ffi::cpuStartTiming(0);
        ffi::cpuGetTiming()
    };
    commands.insert_resource(HardwareClock { last_ticks });
}

fn advance_time(mut time: ResMut<Time>, mut clock: ResMut<HardwareClock>) {
    let now = unsafe { ffi::cpuGetTiming() };
    let delta_ticks = now.wrapping_sub(clock.last_ticks);
    clock.last_ticks = now;

    time.advance_by(Duration::from_nanos(ticks_to_nanos(delta_ticks)));
}

/// Inserts a [`Time`] resource and advances it by real elapsed time each frame.
pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Time>()
            .add_systems(PreStartup, start_clock)
            .add_systems(First, advance_time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_ticks_is_zero_nanos() {
        assert_eq!(ticks_to_nanos(0), 0);
    }

    #[test]
    fn one_second_of_ticks_is_one_billion_nanos() {
        // BUS_CLOCK ticks span ~1 s; allow a tiny truncation error.
        let nanos = ticks_to_nanos(BUS_CLOCK as u32);
        assert!((nanos as i64 - 1_000_000_000).abs() < 100);
    }

    #[test]
    fn does_not_overflow_near_a_full_wrap() {
        // The intermediate product (u64) must not overflow for a near-u32::MAX
        // delta — the reason the cast to u64 happens before the multiply.
        let nanos = ticks_to_nanos(u32::MAX);
        // u32::MAX ticks is ~128 s.
        assert!(nanos > 120_000_000_000 && nanos < 130_000_000_000);
    }

    #[test]
    fn wrapping_delta_matches_elapsed_ticks() {
        // A timer reading that wrapped past u32::MAX still yields the true span.
        let before: u32 = u32::MAX - 10;
        let after: u32 = 5; // 16 ticks really elapsed across the wrap
        assert_eq!(after.wrapping_sub(before), 16);
        assert_eq!(
            ticks_to_nanos(after.wrapping_sub(before)),
            ticks_to_nanos(16)
        );
    }
}
