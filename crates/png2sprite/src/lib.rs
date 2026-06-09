//! Host-side PNG → `.sprite` baker, wrapping BlocksDS's `grit`.
//!
//! Mirrors the `obj2dl` / `wav2bank` shape: a `build.rs` calls [`build_dir`]
//! over `assets/sprites/*.png` and the results land under `build/nitrofs/` so
//! `just rom` can pack them into the ROM filesystem. `bevy_nds_sprite` reads
//! the resulting `.sprite` blob at runtime.
//!
//! Each PNG is fed through `grit` with these flags (see grit's `--help`):
//!
//! - `-gt`     tile output (the OAM layout)
//! - `-gB4`    4 bits per pixel (16-colour palette per sprite)
//! - `-gT 0`   transparent colour is palette index 0
//! - `-p`      include palette
//! - `-ftb`    binary (`.bin`) output
//! - `-fh!`    no C header
//!
//! grit produces `<name>.img.bin` (gfx) and `<name>.pal.bin` (palette). We
//! repack them with a small header into a single `<name>.sprite` file so the
//! runtime only has one path to load.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// ASCII `"BSP1"` — magic prefix of a baked `.sprite` file.
pub const ASSET_MAGIC: u32 = u32::from_le_bytes(*b"BSP1");

/// Bake options for a single PNG.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Sprite width in pixels (must match the PNG width).
    pub width: u16,
    /// Sprite height in pixels (must match the PNG height).
    pub height: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 16,
            height: 16,
        }
    }
}

/// A baked sprite: header + palette + gfx, ready to write to disk.
#[derive(Clone, Debug)]
pub struct Sprite {
    pub width: u16,
    pub height: u16,
    /// 16-entry RGB15 palette (the first entry is transparent).
    pub palette: Vec<u16>,
    /// 4bpp tile gfx in 1D-32 tile order, as grit emits it.
    pub gfx: Vec<u8>,
}

/// Locate the `grit` binary that ships with BlocksDS. Honours `$GRIT` first,
/// then falls back to `$BLOCKSDS/tools/grit/grit`, then `$PATH`.
pub fn find_grit() -> Option<PathBuf> {
    if let Ok(p) = env::var("GRIT") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(b) = env::var("BLOCKSDS") {
        let path = PathBuf::from(b).join("tools/grit/grit");
        if path.is_file() {
            return Some(path);
        }
    }
    which("grit")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Bake one PNG into a [`Sprite`] in memory using `grit`. `work` is a scratch
/// directory grit writes its intermediate `.img.bin` / `.pal.bin` files into.
pub fn bake(grit: &Path, png: &Path, work: &Path, opts: &Options) -> Result<Sprite, String> {
    fs::create_dir_all(work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;
    let stem = png
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("bad PNG name {}", png.display()))?;
    let out_base = work.join(stem);

    let status = Command::new(grit)
        .arg(png)
        .args([
            "-gt",   // tile output
            "-gB4",  // 4 bpp
            "-gT0",  // transparent palette index 0
            "-p",    // include palette
            "-pu16", // u16 palette entries
            "-ftb",  // binary output
            "-fh!",  // no C header
        ])
        .arg(format!("-o{}", out_base.display()))
        .status()
        .map_err(|e| format!("spawn grit: {e}"))?;
    if !status.success() {
        return Err(format!("grit failed on {}", png.display()));
    }

    let img_path = work.join(format!("{stem}.img.bin"));
    let pal_path = work.join(format!("{stem}.pal.bin"));
    let gfx = fs::read(&img_path).map_err(|e| format!("read {}: {e}", img_path.display()))?;
    let pal_bytes = fs::read(&pal_path).map_err(|e| format!("read {}: {e}", pal_path.display()))?;

    if pal_bytes.len() % 2 != 0 {
        return Err(format!(
            "palette length {} not a multiple of 2 (u16)",
            pal_bytes.len()
        ));
    }
    let palette: Vec<u16> = pal_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Sanity: 4bpp 16x16 should be 128 bytes of gfx + 16 palette entries. We
    // don't hard-fail on mismatches (grit may emit a longer palette for
    // colour-quantisation reasons), but we do warn.
    let expected_gfx = (opts.width as usize) * (opts.height as usize) / 2;
    if gfx.len() != expected_gfx {
        // Print on stderr so cargo:warning picks it up from build.rs.
        eprintln!(
            "png2sprite: warning: {} gfx is {} bytes, expected {} for {}x{} 4bpp",
            png.display(),
            gfx.len(),
            expected_gfx,
            opts.width,
            opts.height,
        );
    }

    Ok(Sprite {
        width: opts.width,
        height: opts.height,
        palette,
        gfx,
    })
}

/// Serialise a [`Sprite`] to the on-disk `.sprite` format. Little-endian
/// throughout:
///
/// | offset | type      | field                              |
/// |--------|-----------|------------------------------------|
/// | 0      | `u32`     | magic [`ASSET_MAGIC`] (`"BSP1"`)   |
/// | 4      | `u16`     | width (pixels)                     |
/// | 6      | `u16`     | height (pixels)                    |
/// | 8      | `u32`     | palette entry count                |
/// | 12     | `u32`     | gfx byte count                     |
/// | 16     | `u16` × P | palette (RGB15)                    |
/// | 16+2P  | `u8` × G  | gfx (4bpp tiles, 1D-32 order)      |
pub fn encode(sprite: &Sprite) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + sprite.palette.len() * 2 + sprite.gfx.len());
    out.extend_from_slice(&ASSET_MAGIC.to_le_bytes());
    out.extend_from_slice(&sprite.width.to_le_bytes());
    out.extend_from_slice(&sprite.height.to_le_bytes());
    out.extend_from_slice(&(sprite.palette.len() as u32).to_le_bytes());
    out.extend_from_slice(&(sprite.gfx.len() as u32).to_le_bytes());
    for &p in &sprite.palette {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out.extend_from_slice(&sprite.gfx);
    out
}

/// What [`build_dir`] returns: the inputs it touched, for cargo rerun tracking.
#[derive(Default, Debug)]
pub struct Built {
    pub inputs: Vec<PathBuf>,
}

/// Bake every `*.png` directly under `src` into `dst/*.sprite`. Returns the
/// list of inputs so callers can emit `cargo:rerun-if-changed` for each.
pub fn build_dir(
    src: &Path,
    dst: &Path,
    grit: &Path,
    work: &Path,
    opts: &Options,
) -> Result<Built, String> {
    fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    let mut built = Built::default();

    let entries = fs::read_dir(src).map_err(|e| format!("read_dir {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("png") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("bad PNG name {}", path.display()))?;
        let sprite = bake(grit, &path, work, opts)?;
        let bytes = encode(&sprite);
        let out = dst.join(format!("{stem}.sprite"));
        fs::write(&out, &bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
        built.inputs.push(path);
    }
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header encoding matches the on-disk layout exactly (so the runtime
    /// loader's offsets stay in sync with this writer).
    #[test]
    fn header_layout_is_stable() {
        let sprite = Sprite {
            width: 16,
            height: 16,
            palette: vec![0x0001, 0x0002, 0x0003],
            gfx: vec![0xAA, 0xBB, 0xCC, 0xDD],
        };
        let bytes = encode(&sprite);
        // magic
        assert_eq!(&bytes[0..4], b"BSP1");
        // width / height
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 16);
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 16);
        // palette count, gfx count
        assert_eq!(
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            3
        );
        assert_eq!(
            u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            4
        );
        // palette entries
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 0x0001);
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 0x0002);
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 0x0003);
        // gfx bytes
        assert_eq!(&bytes[22..26], &[0xAA, 0xBB, 0xCC, 0xDD]);
        // total length
        assert_eq!(bytes.len(), 16 + 6 + 4);
    }
}
