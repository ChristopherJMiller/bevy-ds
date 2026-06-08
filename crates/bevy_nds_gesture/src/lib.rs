//! Lightweight touch gestures, derived from Bevy's [`Touches`] stream.
//!
//! Gestures on the DS are not a hardware feature — they are a small amount of
//! bookkeeping over the per-frame touch position that `bevy_nds_input` already
//! produces. This crate turns that single moving point into the gestures most
//! games actually need — **tap**, **long-press**, four-direction **swipe** and
//! **drag** — and surfaces them two ways, mirroring how Bevy exposes other
//! input:
//!
//! - a [`GestureEvent`] stream for one-shot reactions, and
//! - a [`Gestures`] resource for polling (`just_tapped`, `is_dragging`, ...),
//!   cleared each frame like [`ButtonInput`](bevy_input::ButtonInput).
//!
//! All recognition lives in the pure [`GestureRecognizer`] state machine, which
//! is unit-tested on the host; the system that calls it is a thin wrapper that
//! feeds it the current touch and time.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_input::touch::{Touches, touch_screen_input_system};
use bevy_math::Vec2;
use bevy_time::Time;

/// One of the four cardinal swipe directions. Y follows the touch panel, which
/// grows *downward*, so [`SwipeDir::Down`] is a swipe toward the bottom edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwipeDir {
    Up,
    Down,
    Left,
    Right,
}

/// A recognised touch gesture. Positions are in screen pixels (x `0..=255`,
/// y `0..=191`), matching the [`Touches`] coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// A quick press and release in roughly one spot.
    Tap(Vec2),
    /// The pen was held still past the long-press threshold (fires once, while
    /// still held).
    LongPress(Vec2),
    /// A fast directional flick: emitted on release of a quick, far-enough drag,
    /// in addition to the drag events.
    Swipe {
        direction: SwipeDir,
        /// Where the swipe began.
        start: Vec2,
        /// Where the pen lifted.
        end: Vec2,
    },
    /// The pen moved far enough to start dragging (fires once).
    DragStart(Vec2),
    /// The pen moved while dragging.
    Drag {
        /// Current pen position.
        position: Vec2,
        /// Movement since the previous frame.
        delta: Vec2,
    },
    /// The pen lifted after a drag.
    DragEnd(Vec2),
}

/// A [`Gesture`] delivered as a Bevy event.
#[derive(Event, Debug, Clone, Copy, PartialEq)]
pub struct GestureEvent(pub Gesture);

// Recognition thresholds. Tuned for the 256x192 touch panel.
/// Movement (px) under which a press is still a tap rather than a drag.
const TAP_SLOP: f32 = 8.0;
/// Longest press that still counts as a tap.
const TAP_MAX_TIME: Duration = Duration::from_millis(300);
/// How long the pen must rest in place to register a long-press.
const LONG_PRESS_TIME: Duration = Duration::from_millis(500);
/// Minimum travel (px) for a release to count as a swipe.
const SWIPE_MIN_DIST: f32 = 24.0;
/// A swipe must complete within this time (otherwise it is a plain drag).
const SWIPE_MAX_TIME: Duration = Duration::from_millis(400);

/// In-progress press state tracked between frames.
#[derive(Debug, Clone, Copy)]
struct Press {
    start: Vec2,
    start_time: Duration,
    last: Vec2,
    /// Has the pen moved beyond [`TAP_SLOP`]?
    moved: bool,
    /// Are we emitting drag events?
    dragging: bool,
    /// Has the long-press already fired for this press?
    long_fired: bool,
}

/// The pure gesture state machine. Fed one sample (`now`, optional touch
/// position) per frame; returns the gestures recognised on that frame.
#[derive(Debug, Default)]
pub struct GestureRecognizer {
    press: Option<Press>,
}

impl GestureRecognizer {
    /// Advance the recogniser by one frame and return any gestures produced.
    ///
    /// `pos` is the current touch position, or `None` when the pen is up.
    pub fn update(&mut self, now: Duration, pos: Option<Vec2>) -> Vec<Gesture> {
        let mut out = Vec::new();
        match pos {
            Some(p) => match &mut self.press {
                None => {
                    self.press = Some(Press {
                        start: p,
                        start_time: now,
                        last: p,
                        moved: false,
                        dragging: false,
                        long_fired: false,
                    });
                }
                Some(press) => {
                    let prev = press.last;
                    press.last = p;
                    if press.start.distance(p) > TAP_SLOP {
                        press.moved = true;
                    }
                    if press.moved && !press.dragging {
                        press.dragging = true;
                        out.push(Gesture::DragStart(press.start));
                        out.push(Gesture::Drag {
                            position: p,
                            delta: p - prev,
                        });
                    } else if press.dragging {
                        out.push(Gesture::Drag {
                            position: p,
                            delta: p - prev,
                        });
                    } else if !press.long_fired
                        && now.saturating_sub(press.start_time) >= LONG_PRESS_TIME
                    {
                        press.long_fired = true;
                        out.push(Gesture::LongPress(press.start));
                    }
                }
            },
            None => {
                if let Some(press) = self.press.take() {
                    let held = now.saturating_sub(press.start_time);
                    let dist = press.start.distance(press.last);
                    if press.dragging {
                        if held <= SWIPE_MAX_TIME && dist >= SWIPE_MIN_DIST {
                            out.push(Gesture::Swipe {
                                direction: swipe_dir(press.start, press.last),
                                start: press.start,
                                end: press.last,
                            });
                        }
                        out.push(Gesture::DragEnd(press.last));
                    } else if !press.long_fired && held <= TAP_MAX_TIME && dist <= TAP_SLOP {
                        out.push(Gesture::Tap(press.last));
                    }
                }
            }
        }
        out
    }
}

/// Classify a swipe by its dominant axis. Ties go to the horizontal axis.
fn swipe_dir(start: Vec2, end: Vec2) -> SwipeDir {
    let d = end - start;
    let a = d.abs();
    if a.x >= a.y {
        if d.x >= 0.0 {
            SwipeDir::Right
        } else {
            SwipeDir::Left
        }
    } else if d.y >= 0.0 {
        SwipeDir::Down
    } else {
        SwipeDir::Up
    }
}

/// Per-frame gesture state for polling, cleared at the start of each frame like
/// [`ButtonInput`](bevy_input::ButtonInput). One-shot gestures (tap, long-press,
/// swipe) are readable only on the frame they occur; drag state persists for the
/// duration of the drag.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Gestures {
    tapped: Option<Vec2>,
    long_pressed: Option<Vec2>,
    swiped: Option<(SwipeDir, Vec2, Vec2)>,
    dragging: bool,
    drag_position: Option<Vec2>,
    drag_delta: Vec2,
}

impl Gestures {
    /// The position of a tap that happened this frame, if any.
    pub fn just_tapped(&self) -> Option<Vec2> {
        self.tapped
    }

    /// The position of a long-press that triggered this frame, if any.
    pub fn just_long_pressed(&self) -> Option<Vec2> {
        self.long_pressed
    }

    /// The swipe that completed this frame, if any: `(direction, start, end)`.
    pub fn just_swiped(&self) -> Option<(SwipeDir, Vec2, Vec2)> {
        self.swiped
    }

    /// Is the pen currently dragging?
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// The current drag position, while [`is_dragging`](Self::is_dragging).
    pub fn drag_position(&self) -> Option<Vec2> {
        self.drag_position
    }

    /// Pen movement since last frame (zero when not dragging).
    pub fn drag_delta(&self) -> Vec2 {
        self.drag_delta
    }

    /// Reset the per-frame fields. Called once at the top of each frame.
    fn clear_frame(&mut self) {
        self.tapped = None;
        self.long_pressed = None;
        self.swiped = None;
        self.drag_delta = Vec2::ZERO;
    }

    /// Fold one recognised gesture into the polled state.
    fn apply(&mut self, gesture: &Gesture) {
        match *gesture {
            Gesture::Tap(p) => self.tapped = Some(p),
            Gesture::LongPress(p) => self.long_pressed = Some(p),
            Gesture::Swipe {
                direction,
                start,
                end,
            } => self.swiped = Some((direction, start, end)),
            Gesture::DragStart(p) => {
                self.dragging = true;
                self.drag_position = Some(p);
            }
            Gesture::Drag { position, delta } => {
                self.drag_position = Some(position);
                self.drag_delta = delta;
            }
            Gesture::DragEnd(_) => {
                self.dragging = false;
                self.drag_position = None;
            }
        }
    }
}

/// Recognise gestures from this frame's touch position and publish them to the
/// [`Gestures`] resource and the [`GestureEvent`] stream. Runs after Bevy's
/// touch system so [`Touches`] is up to date.
fn detect_gestures(
    time: Res<Time>,
    touches: Res<Touches>,
    mut recognizer: Local<GestureRecognizer>,
    mut gestures: ResMut<Gestures>,
    mut events: EventWriter<GestureEvent>,
) {
    let pos = touches.iter().next().map(|t| t.position());
    gestures.clear_frame();
    for gesture in recognizer.update(time.elapsed(), pos) {
        gestures.apply(&gesture);
        events.write(GestureEvent(gesture));
    }
}

/// Adds touch-gesture recognition: the [`Gestures`] resource and
/// [`GestureEvent`] stream, derived from [`Touches`]. Requires the input plugin
/// (for the touch data) and a `Time` resource (for timing).
pub struct GesturePlugin;

impl Plugin for GesturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Gestures>()
            .add_event::<GestureEvent>()
            .add_systems(PreUpdate, detect_gestures.after(touch_screen_input_system));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Drain a whole gesture sequence, collecting everything produced.
    fn run(samples: &[(u64, Option<Vec2>)]) -> Vec<Gesture> {
        let mut rec = GestureRecognizer::default();
        let mut all = Vec::new();
        for &(t, pos) in samples {
            all.extend(rec.update(ms(t), pos));
        }
        all
    }

    #[test]
    fn quick_press_release_is_a_tap() {
        let a = Vec2::new(40.0, 90.0);
        let out = run(&[(0, Some(a)), (100, Some(a)), (120, None)]);
        assert_eq!(out, [Gesture::Tap(a)]);
    }

    #[test]
    fn slow_press_is_not_a_tap_but_a_long_press() {
        let a = Vec2::new(40.0, 90.0);
        // Held in place past the long-press threshold, then released.
        let out = run(&[(0, Some(a)), (550, Some(a)), (600, None)]);
        assert_eq!(out, [Gesture::LongPress(a)]);
    }

    #[test]
    fn long_press_fires_once_and_suppresses_tap() {
        let a = Vec2::new(10.0, 10.0);
        let out = run(&[(0, Some(a)), (520, Some(a)), (700, Some(a)), (900, None)]);
        // Exactly one LongPress, and no Tap on release.
        assert_eq!(out, [Gesture::LongPress(a)]);
    }

    #[test]
    fn fast_far_drag_release_is_a_swipe_right() {
        let start = Vec2::new(20.0, 50.0);
        let mid = Vec2::new(50.0, 52.0);
        let end = Vec2::new(90.0, 53.0);
        let out = run(&[
            (0, Some(start)),
            (50, Some(mid)),
            (100, Some(end)),
            (120, None),
        ]);
        // Drag events, then a swipe + drag-end on release.
        assert!(matches!(out.first(), Some(Gesture::DragStart(_))));
        let swipe = out.iter().find(|g| matches!(g, Gesture::Swipe { .. }));
        assert_eq!(
            swipe,
            Some(&Gesture::Swipe {
                direction: SwipeDir::Right,
                start,
                end,
            })
        );
        assert_eq!(out.last(), Some(&Gesture::DragEnd(end)));
    }

    #[test]
    fn upward_swipe_is_classified_up() {
        // Y decreases toward the top of the panel.
        let start = Vec2::new(60.0, 150.0);
        let end = Vec2::new(62.0, 100.0);
        let out = run(&[(0, Some(start)), (80, Some(end)), (100, None)]);
        let swipe = out.iter().find(|g| matches!(g, Gesture::Swipe { .. }));
        assert_eq!(
            swipe,
            Some(&Gesture::Swipe {
                direction: SwipeDir::Up,
                start,
                end,
            })
        );
    }

    #[test]
    fn slow_long_drag_is_drag_without_swipe() {
        let start = Vec2::new(20.0, 50.0);
        let end = Vec2::new(90.0, 53.0);
        // Far enough for a swipe, but too slow (held > SWIPE_MAX_TIME).
        let out = run(&[(0, Some(start)), (300, Some(end)), (600, None)]);
        assert!(out.iter().any(|g| matches!(g, Gesture::DragStart(_))));
        assert!(!out.iter().any(|g| matches!(g, Gesture::Swipe { .. })));
        assert_eq!(out.last(), Some(&Gesture::DragEnd(end)));
    }

    #[test]
    fn idle_produces_nothing() {
        let out = run(&[(0, None), (100, None)]);
        assert!(out.is_empty());
    }
}
