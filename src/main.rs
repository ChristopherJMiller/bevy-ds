//! The game: a Bevy app that runs on the Nintendo DS.
//!
//! Everything here is ordinary Bevy — components, systems, resources. The DS
//! itself is handled entirely by the [`bevy_nds`] library via [`DsPlugins`]
//! (the platform layer) and [`bevy_nds_3d`] via [`Ds3dPlugin`] (the hardware 3D
//! backend): this file contains no FFI, no allocator and no panic handler.
//!
//! The top screen shows a hardware-rendered 3D cube that auto-spins and that you
//! move around in space with the D-pad; the bottom screen shows a title and a
//! live HUD driven by the `Time`, `Fps` and input resources.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_nds::prelude::*;
use bevy_nds_3d::prelude::*;

/// Program entry point, called by the BlocksDS crt0.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> core::ffi::c_int {
    let mut app = App::new();
    app.add_plugins(DsPlugins)
        .add_plugins(Ds3dPlugin)
        .add_plugins(GamePlugin);
    bevy_nds::run(app)
}

/// The actual game, as a self-contained Bevy plugin.
struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (spin_cube, move_cube, update_hud));
    }
}

/// The D-pad-controlled, auto-spinning cube.
#[derive(Component)]
struct Cube;

/// The live status line on the bottom screen.
#[derive(Component)]
struct Hud;

fn setup(mut commands: Commands) {
    // Top screen: a hardware-rendered 3D cube (drawn by the DS 3D engine).
    commands.spawn((Cube, DsMesh::cube(0.6), Transform3d::default()));

    // Bottom screen (sub-engine text console): a title, a HUD that updates every
    // frame, and a control hint.
    commands.spawn((
        DsScreen::Bottom,
        TilePos::new(4, 2),
        DsText::new("Bevy 3D on Nintendo DS"),
    ));
    commands.spawn((DsScreen::Bottom, TilePos::new(4, 4), Hud, DsText::new("")));
    commands.spawn((
        DsScreen::Bottom,
        TilePos::new(6, 22),
        DsText::new("D-pad: move the cube"),
    ));
}

/// Continuously rotate the cube so it tumbles in place.
fn spin_cube(mut query: Query<&mut Transform3d, With<Cube>>) {
    for mut transform in &mut query {
        transform.rotation.x += 0.02;
        transform.rotation.y += 0.03;
    }
}

/// Translate the cube around in space with the held D-pad direction.
fn move_cube(input: Res<ButtonInput<DsButton>>, mut query: Query<&mut Transform3d, With<Cube>>) {
    const SPEED: f32 = 0.03;
    for mut transform in &mut query {
        if input.pressed(DsButton::Left) {
            transform.translation.x -= SPEED;
        }
        if input.pressed(DsButton::Right) {
            transform.translation.x += SPEED;
        }
        if input.pressed(DsButton::Up) {
            transform.translation.y += SPEED;
        }
        if input.pressed(DsButton::Down) {
            transform.translation.y -= SPEED;
        }
        transform.translation.x = transform.translation.x.clamp(-1.5, 1.5);
        transform.translation.y = transform.translation.y.clamp(-1.5, 1.5);
    }
}

/// Refresh the bottom-screen HUD from the `Time`, `Fps` and input resources.
fn update_hud(
    time: Res<Time>,
    fps: Res<Fps>,
    input: Res<ButtonInput<DsButton>>,
    mut query: Query<&mut DsText, With<Hud>>,
) {
    let secs = time.elapsed_secs() as u32;
    let fps = fps.0;
    let held = input.get_pressed().count();
    for mut text in &mut query {
        // Reuse the existing String's capacity instead of allocating anew.
        text.0.clear();
        let _ = write!(text.0, "t={secs:>4}s  fps={fps:>2.0}  held={held}");
    }
}
