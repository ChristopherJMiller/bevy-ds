//! Lightweight runtime diagnostics, surfaced as ECS resources.
//!
//! Right now this is just a smoothed frames-per-second counter derived from the
//! real per-frame delta provided by [`bevy_nds_time`](https://docs.rs/bevy_nds_time).
//! Games read `Res<Fps>` and display it however they like.

#![cfg_attr(not(test), no_std)]

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_time::Time;

/// Smoothed frames-per-second estimate. `0.0` until the first delta arrives.
#[derive(Resource, Default, Clone, Copy)]
pub struct Fps(pub f32);

/// Exponential-smoothing factor (weight given to the newest sample).
const SMOOTHING: f32 = 0.1;

/// Fold a new per-frame delta into the smoothed FPS estimate. The first sample
/// (when `prev == 0.0`) seeds the average; non-positive deltas leave it
/// unchanged. Pure helper so the smoothing maths is unit-testable off-target.
fn smooth_fps(prev: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return prev;
    }
    let instant = 1.0 / dt;
    if prev == 0.0 {
        instant
    } else {
        prev * (1.0 - SMOOTHING) + instant * SMOOTHING
    }
}

fn update_fps(time: Res<Time>, mut fps: ResMut<Fps>) {
    fps.0 = smooth_fps(fps.0, time.delta_secs());
}

/// Maintains the [`Fps`] resource each frame.
pub struct DiagnosticsPlugin;

impl Plugin for DiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Fps>()
            .add_systems(PreUpdate, update_fps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_seeds_with_instantaneous_rate() {
        // 1/60 s frame -> 60 fps on the very first delta.
        assert!((smooth_fps(0.0, 1.0 / 60.0) - 60.0).abs() < 0.01);
    }

    #[test]
    fn non_positive_delta_leaves_estimate_unchanged() {
        assert_eq!(smooth_fps(42.0, 0.0), 42.0);
        assert_eq!(smooth_fps(42.0, -1.0), 42.0);
    }

    #[test]
    fn blends_towards_the_new_sample() {
        // From a steady 60, a slower (30 fps) frame nudges the estimate down,
        // but only by the smoothing weight, so it stays well above 30.
        let next = smooth_fps(60.0, 1.0 / 30.0);
        let expected = 60.0 * (1.0 - SMOOTHING) + 30.0 * SMOOTHING;
        assert!((next - expected).abs() < 0.001);
        assert!(next < 60.0 && next > 30.0);
    }

    #[test]
    fn steady_rate_converges_to_that_rate() {
        let mut fps = 0.0;
        for _ in 0..200 {
            fps = smooth_fps(fps, 1.0 / 60.0);
        }
        assert!((fps - 60.0).abs() < 0.01);
    }
}
