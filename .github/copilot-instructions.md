# Copilot instructions for `kts-nds`

**Kill the Serpent** — a Nintendo DS game built on **`bevy_nds`**, a companion
library that runs Bevy's `no_std` ECS on the DS and is developed in this same
repo. The game (`kts`, the root crate) drives what platform crates `bevy_nds`
grows. Read `README.md` for the full narrative; this file captures the
conventions that matter when editing the code.

## Environment & commands

All builds need the BlocksDS toolchain, which is provided by the Nix dev shell.
`build.rs` reads `$BLOCKSDS` (set by the shell) to inject link flags, so **every
command must run inside `nix develop`** — outside it, linking fails with a
`BLOCKSDS is not set` warning.

The `Justfile` is the entry point (run `just --list`):

- `just build` / `just build-release` — compile the ARM9 ELF.
- `just check` — `cargo check` (fast feedback loop; no ROM).
- `just test [filter]` — run the `bevy_nds` host-side unit tests.
- `just fmt` — `cargo fmt`.
- `just rom [profile]` — package the ELF into `kts.nds` with `ndstool`.
- `just run [profile]` — build + package + launch melonDS.
- `just preview [profile]` — build + package + headless desmume screenshot to
  `preview.png` (CI-friendly; override with `OUT=`, `WAIT=`, `DISP=`).
- `profile` defaults to `debug`; pass `release` for the small/fast ROM.

### Testing

Hardware-independent logic in `bevy_nds` is unit-tested **on the host**, since
the DS target is `no_std` with no test harness. Run `just test` (or
`just test <filter>` for a single test). Key mechanics, which you must preserve
when adding tests:

- The crate is `#![cfg_attr(not(test), no_std)]`; under `cfg(test)` it links
  host `std`. The bare-metal `runtime` module (allocator / panic handler /
  critical-section) is `#[cfg(not(test))]` so it doesn't clash with `std`.
- `just test` builds for the host triple and overrides `build-std` (full `std`
  from source, to avoid a duplicate-`core` lang-item clash) and `panic` — it
  does **not** use the default DS target. Plain `cargo test` will *not* work.
- Tests cover pure logic only — never call FFI. When adding hardware code, split
  the testable computation out of the FFI call (see `Grid::diff_runs` vs
  `Grid::flush` in `render.rs`) and test the pure half.

`clippy` is installed but not wired into a task; run `cargo clippy` manually.

## Architecture

Two crates, with a deliberate separation of concerns:

- **`crates/bevy_nds`** — the reusable engine layer. It owns *all* DS-specific
  wiring: FFI, the global allocator, the panic handler, the `critical-section`
  impl, video/input/time/rendering plugins, and the vblank frame loop.
- **`kts`** (root crate, `src/main.rs`) — *Kill the Serpent*, the game. A
  *pure-Bevy consumer* of `bevy_nds`: only components and systems, **no FFI /
  allocator / panic handler**. New game logic belongs here; new hardware
  capability gets its own `bevy_nds_<capability>` crate.

The mapping from DS hardware to ECS concepts is the core idea — keep it intact:

| DS hardware            | `bevy_nds` exposes                          | Plugin / file        |
| ---------------------- | ------------------------------------------- | -------------------- |
| Top / bottom LCDs      | `DsScreen` component + `Consoles` resource  | `VideoPlugin` / `screen.rs` |
| Buttons                | `ButtonInput<DsButton>` resource            | `InputPlugin` / `input.rs` |
| Vertical-blank @ 60 Hz | `run` loop + `Time` resource                | `TimePlugin` / `time.rs` |
| Smoothed FPS           | `Fps` resource                              | `DiagnosticsPlugin` / `diagnostics.rs` |
| Tiled text background   | `Glyph` / `DsText` + `TilePos`              | `RenderPlugin` / `render.rs` |

`DsPlugins` (in `runner.rs`) bundles them all; `bevy_nds::run(app)` installs the
runner via `App::set_runner` and loops forever (`swiWaitForVBlank` → `app.update()`).

### Rendering model

`render.rs` mirrors desktop Bevy's "extract entities to the GPU" shape, but the
"GPU" is the DS text console and the draw call is libnds `printf`. It is
**double-buffered and diffed at the grid level**: a static `front` buffer
mirrors the live tilemap, a `back` buffer is composed fresh each frame, and only
*differing* cells are written to hardware. Never call `consoleClear()` per frame
— that reintroduces flicker. The grid is fixed at 32×24 tiles (libnds default
font). `bevy_text` is intentionally *not* used (too heavy for the DS).

## Conventions

- **`no_std` everywhere.** Both crates are `#![no_std]`; `src/main.rs` is also
  `#![no_main]` with a `#[unsafe(no_mangle)] extern "C" fn main`. Use `core` /
  `alloc` (`extern crate alloc;`), never `std`.
- **FFI is hand-written and centralised** in `crates/bevy_nds/src/ffi.rs` — no
  bindgen. Add only the minimal libnds surface you need there, with a comment
  citing the libnds header (e.g. `<nds/input.h>`), and resolve symbols at link
  time via `build.rs`.
- **Raw pointers in resources** (e.g. `ConsoleHandle`) get manual
  `unsafe impl Send + Sync`, justified by "the DS is single-core". Follow that
  pattern and keep the SAFETY comment.
- **Plugins, not free functions.** Each capability is a Bevy `Plugin`; the game
  groups its own systems in a `GamePlugin`. Re-export public plugins/types from
  `lib.rs` and add game-facing items to `bevy_nds::prelude`.
- **Schedule usage:** hardware init runs in `PreStartup` (`init_screens`), game
  setup in `Startup`, per-frame logic in `Update`.
- **Avoid per-frame heap churn.** Reuse buffers/`String` capacity (see
  `update_hud` calling `text.0.clear()` then `write!`) rather than allocating
  each frame — RAM is ~4 MB and the ARM9 is 33 MHz.

## Build internals (rarely touched, but load-bearing)

- Custom Tier-3 target `armv5te-nintendo-ds.json`; `.cargo/config.toml` enables
  `-Z build-std` (core/alloc from source) and sets
  `--cfg portable_atomic_no_outline_atomics`.
- The DS has no atomic CAS, so `portable-atomic` (pulled in by Bevy via the
  `critical-section` feature on every `bevy_*` dependency) relies on the
  interrupt-toggling `critical-section` impl in `runtime.rs`. Keep the
  `critical-section` feature on Bevy crates.
- `panic = "abort"` in both profiles; the dev profile optimizes *dependencies*
  only (`[profile.dev.package."*"] opt-level = 3`) to keep the debug ROM at
  60 fps while own-crate rebuilds stay fast.
