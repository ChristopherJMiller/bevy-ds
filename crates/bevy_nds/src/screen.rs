//! The two physical DS screens, modelled for the ECS.
//!
//! Each LCD is driven by a separate 2D engine (the "main" engine outputs to the
//! top screen, the "sub" engine to the bottom). We bring up a libnds text
//! console on each and expose them as a [`Consoles`] resource. Renderable
//! entities carry a [`DsScreen`] component selecting which screen they live on.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

use crate::ffi::{self, PrintConsole};

/// Which physical screen an entity is rendered on.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum DsScreen {
    /// The top LCD, driven by the main 2D engine.
    Top,
    /// The bottom LCD, driven by the sub 2D engine.
    Bottom,
}

/// A pointer to a libnds console. Wrapped so it can live in a `Resource`; the
/// DS is single-core and we only touch consoles from systems, so sharing the
/// raw pointer across the (single) thread is sound.
#[derive(Clone, Copy)]
pub struct ConsoleHandle(pub(crate) *mut PrintConsole);

// SAFETY: the DS runs systems on a single core; there is no real concurrency.
unsafe impl Send for ConsoleHandle {}
unsafe impl Sync for ConsoleHandle {}

/// Handles to the initialised top/bottom consoles.
#[derive(Resource, Clone, Copy)]
pub struct Consoles {
    top: ConsoleHandle,
    bottom: ConsoleHandle,
}

impl Consoles {
    /// The console backing the given screen.
    pub(crate) fn handle(&self, screen: DsScreen) -> *mut PrintConsole {
        match screen {
            DsScreen::Top => self.top.0,
            DsScreen::Bottom => self.bottom.0,
        }
    }
}

/// Backing storage for the top console. libnds initialises it in place during
/// [`init_screens`]; it must outlive the program, hence `static`.
static mut TOP_CONSOLE: PrintConsole = PrintConsole::zeroed();

/// Brings up a text console on both screens and inserts the [`Consoles`]
/// resource. Runs once, before the first frame is rendered.
fn init_screens(mut commands: Commands) {
    let (top, bottom) = unsafe {
        // The sub engine + bottom console: libnds has a one-call helper that
        // also configures the sub video mode and VRAM bank C.
        let bottom = ffi::consoleDemoInit();

        // The main engine + top console: configure video mode 0 and map VRAM
        // bank A to main-engine background memory, then init the console on it.
        core::ptr::write_volatile(ffi::REG_DISPCNT, ffi::MODE_0_2D);
        core::ptr::write_volatile(ffi::VRAM_A_CR, ffi::VRAM_ENABLE | ffi::VRAM_A_MAIN_BG);
        let top = ffi::consoleInit(
            &raw mut TOP_CONSOLE,
            0,                      // background layer 0
            ffi::BG_TYPE_TEXT_4BPP, // 4bpp tiled text
            ffi::BG_SIZE_T_256X256, // 256x256 text background
            22,                     // map base (matches the demo console)
            3,                      // tile base
            true,                   // main_display -> top screen
            true,                   // load the default font
        );
        (top, bottom)
    };

    commands.insert_resource(Consoles {
        top: ConsoleHandle(top),
        bottom: ConsoleHandle(bottom),
    });
}

/// Initialises the DS video hardware and exposes the screens to the ECS.
pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, init_screens);
    }
}
