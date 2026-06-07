# bevy-ds

[Bevy](https://bevyengine.org/)'s ECS running on a **Nintendo DS**, built into a
real `.nds` ROM you can boot in an emulator (or on hardware). The whole thing
comes out of a Nix dev shell, so there's no toolchain to install by hand.

It started as a "can this even work?" experiment and grew into a small library
plus a tech demo. There are three crates:

- **`bevy_nds`** (`crates/bevy_nds`) — the library. It glues Bevy's `no_std`
  ECS/App core to the DS hardware through [libnds](https://github.com/blocksds/libnds)
  and hands it back as normal Bevy plugins, components and resources.
- **`bevy_nds_3d`** (`crates/bevy_nds_3d`) — an add-on that drives the DS's
  hardware 3D engine, with `Transform3d`, `DsMesh` and a `Camera3d` resource.
  Built on top of `bevy_nds`.
- **`bevy-ds`** (the root crate) — the demo itself. It's plain Bevy: components
  and systems, no FFI or allocator or panic handler in sight.

<p align="center">
  <img src="docs/cube-demo.png" alt="The hardware-rendered 3D cube on the top screen with the live HUD below" width="320">
</p>

The demo is a spinning cube on one screen with a live HUD on the other. The
D-pad pushes the cube around, and ABXY tumble it so you can see the hardware
shading each face. Run it off the edge of the screen and it crosses to the other
one.

That screen-crossing is a fun side effect of how the DS is wired: the 3D core is
bolted to the *main* 2D engine, and a single hardware bit (`POWER_SWAP_LCDS`)
decides which physical LCD that engine drives. The sub engine always gets the
other screen. So the cube and the text HUD are always on opposite screens, and a
`Display3d` resource flips which is which. Crossing the edge just toggles that
bit, so the cube pops out on the other screen and the HUD slides over to where
the cube used to be.

## How it works

The full `bevy` crate pulls in `wgpu` and `winit`, so it's a non-starter on the
DS. But Bevy's core has been `no_std`-friendly since 0.16, so `bevy_nds`
cherry-picks the ECS/App pieces and supplies the platform layer itself. The
trick is to express DS hardware as ordinary Bevy concepts, so game code never
touches the metal directly:

| DS hardware              | `bevy_nds` exposes                                                    | Plugin              |
| ------------------------ | -------------------------------------------------------------------- | ------------------- |
| Top / bottom LCDs        | `DsScreen::{Top,Bottom}` component + `Consoles` resource             | `VideoPlugin`       |
| Buttons (`REG_KEYINPUT`) | the standard `ButtonInput<DsButton>` resource                        | `InputPlugin`       |
| Vertical-blank @ ~60 Hz  | a `set_runner` frame loop + a real `Time` resource (hardware timer)  | `TimePlugin`        |
| —                        | a smoothed `Fps` resource for diagnostics                            | `DiagnosticsPlugin` |
| Tiled text background    | `Glyph` / `DsText` + `TilePos`, drawn by an extraction system        | `RenderPlugin`      |
| 3D geometry engine       | `Transform3d` + `DsMesh` + a `Camera3d` resource (in `bevy_nds_3d`)  | `Ds3dPlugin`        |

`DsPlugins` bundles all of it, and `bevy_nds::run(app)` installs the runner that
owns the frame loop (`swiWaitForVBlank` → `app.update()`).

### Rendering model

Desktop Bevy extracts entities to the GPU every frame. `bevy_nds` keeps that
shape but the "GPU" is the DS text console (a tiled background) and the draw call
is a libnds `printf`. A drawable is any entity with a `TilePos` and a `DsScreen`,
plus either a `Glyph` (one character, the DS version of a text sprite) or a
`DsText` (a string).

The renderer is double-buffered at the grid level to keep things from flickering.
Each screen keeps a `front` buffer that mirrors what's actually on the tilemap
and a `back` buffer that gets composed from scratch each frame. The render system
stamps every drawable into `back`, then writes only the cells that changed to the
hardware and copies them into `front`. The screen is never blanked, so no
flicker, and most frames only touch a handful of tiles. That sidesteps both the
visible flash of a full `consoleClear()` and any per-frame allocation.

`bevy_text` (cosmic-text font rasterisation) is way too heavy for the DS, so it's
dropped entirely and replaced with this small `no_std` text layer on the tile
engine.

This is roughly the same playbook
[`bevy_mod_gba`](https://github.com/bushrat011899/bevy_mod_gba) uses for the
Game Boy Advance.

### Bare-metal runtime

`bevy_nds` also carries the bits a bare-metal Rust program needs so the game
doesn't have to (`crates/bevy_nds/src/runtime.rs`):

- a `#[global_allocator]` on top of newlib's heap (set up by the BlocksDS crt0),
- a `#[panic_handler]`, and
- a `critical-section` impl that toggles the DS interrupt-enable register, which
  is what Bevy's atomics (`portable-atomic`) sit on.

## Prerequisites

- [Nix](https://nixos.org/) with flakes enabled.

That's the whole list. The dev shell brings the Rust nightly toolchain, the
BlocksDS SDK, `ndstool`, the melonDS and desmume emulators, and the preview
tooling.

BlocksDS comes in as a proper Nix derivation (no `buildFHSEnv`) via
[`pgattic/blocksds-nix`](https://github.com/pgattic/blocksds-nix), which patches
the official toolchain into the Nix store and exports `$BLOCKSDS` /
`$WONDERFUL_TOOLCHAIN`.

## Quick start

```sh
nix develop          # enter the dev shell (first run builds/fetches the toolchain)

just build           # compile the ARM9 ELF (debug)
just rom             # package it into bevy-ds.nds with ndstool
just run             # build + package + launch melonDS
just preview         # build + package + headless desmume screenshot -> preview.png
```

Want the small, fast build? Tack `release` onto the end, e.g. `just run release`.

### Tasks

| Command                  | Description                                                |
| ------------------------ | ---------------------------------------------------------- |
| `just build`             | Compile the ARM9 ELF (debug).                              |
| `just build-release`     | Compile the ARM9 ELF (release).                            |
| `just rom [profile]`     | Package an ELF into `bevy-ds.nds` (`ndstool`).             |
| `just run [profile]`     | Build, package, and run in **melonDS** (interactive).      |
| `just preview [profile]` | Build, package, boot in **desmume** headlessly and save `preview.png`. Override with `OUT=`, `WAIT=`, `DISP=`. |
| `just check`             | `cargo check`.                                             |
| `just test [filter]`     | Run the `bevy_nds` host-side unit tests (builds for the host triple). |
| `just fmt`               | `cargo fmt`.                                               |
| `just clean`             | Remove build artifacts and the ROM.                        |

### Testing

The hardware-independent logic in `bevy_nds` has unit tests: the render diffing,
the timer-tick→nanoseconds math, the FPS smoothing, the button-mask mapping.
They run on your host machine, not the DS:

```sh
just test          # run all bevy_nds unit tests
just test render   # run only tests whose name matches "render"
```

The crate is only `no_std` when it's not under `cfg(test)`, so the test build
gets the host `std` and the normal test harness. `just test` compiles for the
host triple and overrides the project's `build-std`/panic settings just for that
run (the `Justfile` explains why). The first run builds `std` and is slow; after
that it's quick. Anything that calls into the hardware is kept out of the tested
functions, so you never need a DS or an emulator to run them.

## Project layout

```
flake.nix                       dev shell: Rust nightly + BlocksDS + emulators + preview tools
rust-toolchain.toml             pins nightly + rust-src (for build-std)
armv5te-nintendo-ds.json        custom Tier-3 target spec (ARM946E-S, no_std)
.cargo/config.toml              build-std + target selection
build.rs                        injects libnds/specs/libgcc link args from $BLOCKSDS
Cargo.toml                      workspace root + the `bevy-ds` game binary
src/main.rs                     the game: pure Bevy components + systems (no FFI)
Justfile                        build / rom / run / preview tasks
crates/bevy_nds/                the reusable Bevy <-> Nintendo DS library
  src/lib.rs                      crate root, plugin/component re-exports, run()
  src/ffi.rs                      hand-written FFI to the libnds functions we use
  src/runtime.rs                  allocator, panic handler, critical-section impl
  src/screen.rs                   DsScreen, Consoles, VideoPlugin (both screens)
  src/input.rs                    DsButton + ButtonInput<DsButton> (InputPlugin)
  src/time.rs                     real-time Time from the hardware timer (TimePlugin)
  src/diagnostics.rs              smoothed Fps resource (DiagnosticsPlugin)
  src/render.rs                   Glyph/DsText/TilePos + diffed render system (RenderPlugin)
  src/runner.rs                   the vblank App runner + DsPlugins group
```

## Writing a game

A game is a `no_std` binary that adds `DsPlugins`, registers its systems, and
calls `bevy_nds::run`:

```rust
#![no_std]
#![no_main]

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_nds::prelude::*;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> core::ffi::c_int {
    let mut app = App::new();
    app.add_plugins(DsPlugins);
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((DsScreen::Top, TilePos::new(4, 2), DsText::new("Hello, DS!")));
    });
    bevy_nds::run(app)
}
```

`src/main.rs` is the full example: the 3D cube, D-pad movement across both
screens (the `Display3d` LCD swap), ABXY rotation, and the live HUD.

## Build details

A few things that are easy to trip over if you go poking at the build:

- **Target.** `armv5te-nintendo-ds.json` describes the ARM946E-S core (no std,
  `panic = "abort"`, soft-float). It's Tier 3, so `core`/`alloc` get built from
  source with `-Z build-std` (set up in `.cargo/config.toml`).
- **Linking.** `build.rs` reads `$BLOCKSDS` (the dev shell sets it) and passes
  the ARM9 crt0/linker script via `-specs=…/ds_arm9.specs`, plus
  `-lnds9 -lc -lgcc`. `libgcc` is there because the BlocksDS specs alias
  `__sync_synchronize` to a helper that lives in it.
- **Atomics.** The DS has no atomic compare-and-swap, so `portable-atomic`
  (which Bevy drags in) is backed by the `critical-section` impl in
  `crates/bevy_nds/src/runtime.rs`, which just disables interrupts around the
  section.
- **Packaging.** `ndstool` stitches our ARM9 ELF together with a stock BlocksDS
  ARM7 core (`arm7_minimal.elf`) into the final `.nds`.
- **Performance.** Following Bevy's own advice, the dev profile leaves our crates
  unoptimized for fast rebuilds but cranks every dependency to
  `opt-level = 3` (`[profile.dev.package."*"]`), so even the debug ROM holds a
  steady 60 fps on the 33 MHz ARM9. Build `release` for the smallest, fastest ROM.

## Limitations / next steps

- Text rendering goes through the libnds **text console**. Real sprite/tile
  graphics would use libnds OAM/backgrounds (and `grit` for asset conversion,
  already in the shell) behind the same `RenderPlugin` extraction model.
- No audio (maxmod) or Wi-Fi (dswifi) yet. Swap in the matching ARM7 core and
  link `-lmm9` / `-ldswifi9` to turn them on.
- Keep entity counts modest. The DS only has ~4 MB of RAM.

## References

- BlocksDS SDK — https://github.com/blocksds/sdk
- blocksds-nix (Nix packaging) — https://github.com/pgattic/blocksds-nix
- nds-rs (libnds Rust bindings / target spec reference) — https://github.com/BlueTheDuck/nds-rs
- Bevy `no_std` docs — https://github.com/bevyengine/bevy/blob/main/docs/cargo_features.md
