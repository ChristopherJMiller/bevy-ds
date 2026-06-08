//! Tiny CLI around the `png2sprite` library. Not used by the demo's `build.rs`
//! (which calls into the library directly) — handy for `cargo run -p
//! png2sprite -- assets/sprites/cursor.png` style spot-baking.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next().map(PathBuf::from) else {
        eprintln!("usage: png2sprite <input.png> [output.sprite]");
        return ExitCode::from(2);
    };
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&input).with_extension("sprite"));

    let grit = match png2sprite::find_grit() {
        Some(g) => g,
        None => {
            eprintln!(
                "png2sprite: grit not found. Set $GRIT, run inside `nix develop` \
                 (which sets $BLOCKSDS), or add grit to PATH."
            );
            return ExitCode::from(2);
        }
    };
    let work = env::temp_dir().join("png2sprite-cli");

    let sprite = match png2sprite::bake(&grit, &input, &work, &png2sprite::Options::default()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("png2sprite: {e}");
            return ExitCode::from(1);
        }
    };
    let bytes = png2sprite::encode(&sprite);
    if let Err(e) = std::fs::write(&output, &bytes) {
        eprintln!("png2sprite: write {}: {e}", output.display());
        return ExitCode::from(1);
    }
    println!(
        "wrote {} ({}x{}, {} palette entries, {} gfx bytes)",
        output.display(),
        sprite.width,
        sprite.height,
        sprite.palette.len(),
        sprite.gfx.len(),
    );
    ExitCode::SUCCESS
}
