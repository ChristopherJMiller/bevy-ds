//! A frame-driven clock that powers Bevy's standard [`Time`] resource.
//!
//! On desktop, `bevy_time` reads a wall clock. The DS has no `std` clock, but
//! the display refresh is a precise 59.83 Hz heartbeat, so we advance virtual
//! time by one frame's worth each update. Game code can then use the ordinary
//! `Res<Time>` API (`elapsed_secs`, `delta_secs`, ...).

use core::time::Duration;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_time::Time;

/// One DS frame: the LCD refreshes at ~59.8261 Hz.
const FRAME: Duration = Duration::from_nanos(16_715_000);

fn advance_time(mut time: ResMut<Time>) {
    time.advance_by(FRAME);
}

/// Inserts a [`Time`] resource and advances it one frame per update.
pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Time>()
            .add_systems(First, advance_time);
    }
}
