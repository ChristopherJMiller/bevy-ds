//! Compile-time model loaders for [`bevy_nds_3d`](../bevy_nds_3d/index.html).
//!
//! The Nintendo DS has no runtime filesystem or asset server, so model data can
//! only ever be *bytes at a ROM address*. This crate bridges the gap: it parses
//! model files **on the host at build time** and emits a `&'static` mesh that the
//! game crate bakes straight into the ROM. The ergonomics stay Bevy-flavoured —
//! you reference a model by path — but there is no runtime loading.
//!
//! Currently it supports Wavefront OBJ via [`include_obj!`].

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use proc_macro::{TokenStream, TokenTree};

/// Bake a Wavefront `.obj` model into the ROM as a `bevy_nds_3d::DsMesh`.
///
/// The path is resolved relative to the calling crate's `Cargo.toml`
/// (`CARGO_MANIFEST_DIR`). The OBJ is parsed at compile time: positions (`v`),
/// optional normals (`vn`) and faces (`f`) are read, faces with more than three
/// vertices are **fan-triangulated**, and any face lacking explicit normals gets
/// a computed flat normal. The result is a `&'static [[Vertex; 3]]` embedded in
/// the binary, wrapped in a hardware-**lit** [`DsMesh`].
///
/// # Origin / offset settings
///
/// Models are often authored around an off-centre origin (the Utah teapot sits
/// on the XY plane, so its pivot is at the *base*, not the middle), which makes
/// rotation look like it is tumbling around the wrong point. Two optional,
/// comma-separated settings adjust the model-space origin at build time (so they
/// cost nothing at runtime):
///
/// - `center` — recentre the geometry on the midpoint of its bounding box, so
///   the entity's [`Transform3d`] rotates it about its visual centre.
/// - `offset = [x, y, z]` — translate every vertex by this amount (applied
///   *after* `center` if both are given).
///
/// ```ignore
/// use bevy_nds_3d::prelude::*;
/// // As authored:
/// commands.spawn((include_obj!("assets/teapot.obj"), Transform3d::default()));
/// // Recentred so it spins about its middle:
/// commands.spawn((include_obj!("assets/teapot.obj", center), Transform3d::default()));
/// // Recentred, then nudged down a touch:
/// commands.spawn(include_obj!("assets/teapot.obj", center, offset = [0.0, -0.2, 0.0]));
/// ```
#[proc_macro]
pub fn include_obj(input: TokenStream) -> TokenStream {
    let args = match parse_args(input) {
        Ok(a) => a,
        Err(e) => return compile_error(&e),
    };
    let path = &args.path;

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| ".".into());
    let full = PathBuf::from(&manifest_dir).join(path);

    let source = match fs::read_to_string(&full) {
        Ok(s) => s,
        Err(e) => {
            return compile_error(&format!(
                "include_obj!: could not read {}: {e}",
                full.display()
            ));
        }
    };

    let mut tris = match parse_obj(&source) {
        Ok(t) => t,
        Err(e) => return compile_error(&format!("include_obj!({path:?}): {e}")),
    };
    if tris.is_empty() {
        return compile_error(&format!("include_obj!({path:?}): no triangles found"));
    }

    apply_origin(&mut tris, &args);

    let code = emit(&tris, &full);
    TokenStream::from_str(&code).expect("include_obj! produced invalid tokens")
}

/// Parsed `include_obj!` arguments: the model path plus the origin settings.
struct Args {
    path: String,
    /// Recentre on the bounding-box midpoint before emitting.
    center: bool,
    /// Constant translation applied to every vertex (after centring).
    offset: [f32; 3],
}

/// Shift the baked geometry's origin per the `center` / `offset` settings. Done
/// at build time so the runtime pays nothing.
fn apply_origin(tris: &mut [Tri], args: &Args) {
    let mut shift = [0.0f32; 3];

    if args.center {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for tri in tris.iter() {
            for (pos, _) in &tri.verts {
                for k in 0..3 {
                    min[k] = min[k].min(pos[k]);
                    max[k] = max[k].max(pos[k]);
                }
            }
        }
        for k in 0..3 {
            shift[k] = -0.5 * (min[k] + max[k]);
        }
    }
    for k in 0..3 {
        shift[k] += args.offset[k];
    }

    if shift == [0.0, 0.0, 0.0] {
        return;
    }
    for tri in tris.iter_mut() {
        for (pos, _) in &mut tri.verts {
            for k in 0..3 {
                pos[k] += shift[k];
            }
        }
    }
}

/// One triangle's worth of baked vertex data: position + normal per corner.
struct Tri {
    verts: [([f32; 3], [f32; 3]); 3],
}

/// Parse the subset of Wavefront OBJ we need: `v`, `vn`, `f`. Faces are
/// fan-triangulated; missing per-vertex normals are filled with the triangle's
/// flat (geometric) normal.
fn parse_obj(source: &str) -> Result<Vec<Tri>, String> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut tris: Vec<Tri> = Vec::new();

    for (lineno, line) in source.lines().enumerate() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v = parse_vec3(&mut it)
                    .ok_or_else(|| format!("line {}: malformed vertex", lineno + 1))?;
                positions.push(v);
            }
            Some("vn") => {
                let n = parse_vec3(&mut it)
                    .ok_or_else(|| format!("line {}: malformed normal", lineno + 1))?;
                normals.push(n);
            }
            Some("f") => {
                // Collect the face's (position, optional-normal) corner indices.
                let mut corners: Vec<([f32; 3], Option<[f32; 3]>)> = Vec::new();
                for tok in it {
                    let (vi, ni) = parse_face_vertex(tok)
                        .ok_or_else(|| format!("line {}: malformed face vertex {tok:?}", lineno + 1))?;
                    let pos = *resolve(&positions, vi)
                        .ok_or_else(|| format!("line {}: vertex index out of range", lineno + 1))?;
                    let nor = match ni {
                        Some(ni) => Some(
                            *resolve(&normals, ni).ok_or_else(|| {
                                format!("line {}: normal index out of range", lineno + 1)
                            })?,
                        ),
                        None => None,
                    };
                    corners.push((pos, nor));
                }
                if corners.len() < 3 {
                    return Err(format!("line {}: face has < 3 vertices", lineno + 1));
                }
                // Fan-triangulate: (0, i, i+1) for i in 1..n-1.
                for i in 1..corners.len() - 1 {
                    let a = corners[0];
                    let b = corners[i];
                    let c = corners[i + 1];
                    let flat = flat_normal(a.0, b.0, c.0);
                    tris.push(Tri {
                        verts: [
                            (a.0, a.1.unwrap_or(flat)),
                            (b.0, b.1.unwrap_or(flat)),
                            (c.0, c.1.unwrap_or(flat)),
                        ],
                    });
                }
            }
            _ => {} // comments, o/g/s/usemtl/mtllib, blanks, unsupported records
        }
    }

    Ok(tris)
}

/// Resolve a 1-based OBJ index (negative = relative to the end) into a slice.
fn resolve<T>(items: &[T], idx: i32) -> Option<&T> {
    if idx > 0 {
        items.get((idx - 1) as usize)
    } else if idx < 0 {
        let from_end = items.len() as i32 + idx;
        usize::try_from(from_end).ok().and_then(|i| items.get(i))
    } else {
        None
    }
}

fn parse_vec3<'a>(it: &mut impl Iterator<Item = &'a str>) -> Option<[f32; 3]> {
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    Some([x, y, z])
}

/// Parse one face vertex token (`v`, `v/t`, `v//n`, or `v/t/n`) into a vertex
/// index and an optional normal index.
fn parse_face_vertex(tok: &str) -> Option<(i32, Option<i32>)> {
    let mut parts = tok.split('/');
    let v: i32 = parts.next()?.parse().ok()?;
    let _t = parts.next(); // texture coord index, ignored
    let n = match parts.next() {
        Some(s) if !s.is_empty() => Some(s.parse().ok()?),
        _ => None,
    };
    Some((v, n))
}

/// Geometric (flat) normal of a triangle, normalised; zero if degenerate.
fn flat_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    normalize(n)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Emit the `DsMesh` expression. The geometry is **pre-packed at build time**
/// into a libnds *display list* (see [`display_list`]): a self-contained block
/// of packed Geometry Engine commands that the DS draws in one DMA burst via
/// `glCallList`, with no per-frame fixed-point or normal maths on the 33 MHz
/// ARM9. A leading `include_bytes!` ties the build to the source file so edits
/// to the model trigger a recompile (proc-macros don't track file reads on their
/// own).
fn emit(tris: &[Tri], full: &std::path::Path) -> String {
    let (words, [min, max]) = display_list(tris);

    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(
        out,
        "    const _: &[u8] = include_bytes!({:?});",
        full.display().to_string()
    );
    out.push_str("    const WORDS: &[u32] = &[");
    for (i, w) in words.iter().enumerate() {
        if i % 12 == 0 {
            out.push_str("\n        ");
        }
        let _ = write!(out, "0x{w:08X},");
    }
    out.push_str("\n    ];\n");
    let _ = writeln!(
        out,
        "    ::bevy_nds_3d::DsMesh::from_baked(WORDS, [{}f32,{}f32,{}f32], [{}f32,{}f32,{}f32])",
        fl(min[0]), fl(min[1]), fl(min[2]),
        fl(max[0]), fl(max[1]), fl(max[2]),
    );
    out.push_str("}\n");
    out
}

// Packed Geometry-Engine FIFO command IDs, i.e. `REG2ID(reg) = (addr - 0x04000400) >> 2`
// from `<nds/arm9/videoGL.h>`. These index a register so four of them pack into
// one 32-bit word via [`fifo_pack`]; the arguments each command consumes then
// follow, in order, in the words after the packed-command word.
const FIFO_NOP: u8 = 0x00; // GFX_FIFO 0x04000400 — padding, no arguments
const FIFO_NORMAL: u8 = 0x21; // GFX_NORMAL 0x04000484 — 1 argument
const FIFO_VERTEX16: u8 = 0x23; // GFX_VERTEX16 0x0400048C — 2 arguments
const FIFO_BEGIN: u8 = 0x40; // GFX_BEGIN 0x04000500 — 1 argument (primitive type)
const FIFO_END: u8 = 0x41; // GFX_END 0x04000504 — no arguments
/// `GL_TRIANGLES` primitive selector for `GFX_BEGIN`.
const GL_TRIANGLES: u32 = 0;

/// Build a libnds display list for a hardware-lit triangle mesh, plus its
/// local-space axis-aligned bounding box (`[min, max]`).
///
/// A display list is a self-contained command block the GPU consumes in one DMA
/// burst (`glCallList`). Its layout (see the BlocksDS `display_list_creation`
/// example) is: a leading word giving the body length in `u32`s, then the body —
/// words that pack four command IDs each ([`fifo_pack`]), with the arguments for
/// those commands following in order. We emit one `GFX_BEGIN(GL_TRIANGLES)`, then
/// per vertex a `GFX_NORMAL` (1 word) and a `GFX_VERTEX16` (2 words), then
/// `GFX_END`. Lighting/material/poly-format are set by the renderer *outside* the
/// list, so the same baked geometry honours the live [`DsMaterial`]/`DsLights`.
fn display_list(tris: &[Tri]) -> (Vec<u32>, [[f32; 3]; 2]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    // (command id, its argument words), in submission order.
    let mut ops: Vec<(u8, Vec<u32>)> = Vec::with_capacity(tris.len() * 6 + 2);
    ops.push((FIFO_BEGIN, vec![GL_TRIANGLES]));
    for tri in tris {
        for (pos, nor) in &tri.verts {
            for k in 0..3 {
                min[k] = min[k].min(pos[k]);
                max[k] = max[k].max(pos[k]);
            }
            let n = normalize(*nor);
            ops.push((FIFO_NORMAL, vec![normal_pack(n[0], n[1], n[2])]));
            let (xy, z) = vertex16(pos[0], pos[1], pos[2]);
            ops.push((FIFO_VERTEX16, vec![xy, z]));
        }
    }
    ops.push((FIFO_END, vec![]));

    (pack_display_list(&ops), [min, max])
}

/// Pack four FIFO command IDs into one little-endian word (`c0` in the low byte),
/// matching libnds' `FIFO_COMMAND_PACK`.
fn fifo_pack(cmds: [u8; 4]) -> u32 {
    (cmds[0] as u32)
        | ((cmds[1] as u32) << 8)
        | ((cmds[2] as u32) << 16)
        | ((cmds[3] as u32) << 24)
}

/// Encode `(command, args)` ops into the display-list `u32` stream: a leading
/// body-length word, then groups of one packed-command word (four IDs, padded
/// with [`FIFO_NOP`]) followed by those commands' argument words in order.
fn pack_display_list(ops: &[(u8, Vec<u32>)]) -> Vec<u32> {
    let mut body: Vec<u32> = Vec::new();
    for chunk in ops.chunks(4) {
        let mut ids = [FIFO_NOP; 4];
        for (i, (cmd, _)) in chunk.iter().enumerate() {
            ids[i] = *cmd;
        }
        body.push(fifo_pack(ids));
        for (_, args) in chunk {
            body.extend_from_slice(args);
        }
    }

    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(body.len() as u32); // glCallList: first word is the body length in words
    out.extend_from_slice(&body);
    out
}

/// Pack a position into the DS `GFX_VERTEX16` command pair, matching
/// `bevy_nds_3d::ffi::gl::vertex_v16`: each component is 4.12 fixed (`* 4096`),
/// `(xy, z)` as two command words.
fn vertex16(x: f32, y: f32, z: f32) -> (u32, u32) {
    let xi = (x * 4096.0) as i16 as u16 as u32;
    let yi = (y * 4096.0) as i16 as u16 as u32;
    let zi = (z * 4096.0) as i16 as u16 as u32;
    ((yi << 16) | xi, zi)
}

/// Pack a unit normal into the DS `GFX_NORMAL` command word, matching
/// `bevy_nds_3d::ffi::normal_pack` (10-bit signed per component).
fn normal_pack(x: f32, y: f32, z: f32) -> u32 {
    float_to_v10(x) | (float_to_v10(y) << 10) | (float_to_v10(z) << 20)
}

/// Float → 10-bit signed `v10` (1.0 → 0x1FF), matching `ffi::float_to_v10`.
fn float_to_v10(v: f32) -> u32 {
    let x = if v >= 1.0 {
        0x1FF
    } else if v < -1.0 {
        0x200
    } else {
        ((v * 512.0) as i32) & 0x3FF
    };
    x as u32
}

/// Format an `f32` so it always round-trips as a float literal (`0` -> `0.0`).
fn fl(v: f32) -> String {
    let mut s = format!("{v:?}");
    if !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("NaN") {
        s.push_str(".0");
    }
    s
}

/// Parse the macro input: a string-literal path, optionally followed by
/// comma-separated `center` and/or `offset = [x, y, z]` settings.
fn parse_args(input: TokenStream) -> Result<Args, String> {
    let mut trees = input.into_iter().peekable();

    let path = match trees.next() {
        Some(TokenTree::Literal(lit)) => unquote(&lit.to_string())
            .ok_or_else(|| "include_obj!: first argument must be a string-literal path".to_string())?,
        _ => {
            return Err("include_obj! expects a path, e.g. include_obj!(\"assets/model.obj\")".into());
        }
    };

    let mut args = Args {
        path,
        center: false,
        offset: [0.0, 0.0, 0.0],
    };

    while let Some(tt) = trees.next() {
        // Settings are comma-separated.
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => continue,
            _ => {}
        }
        let TokenTree::Ident(ident) = &tt else {
            return Err(format!("include_obj!: unexpected token {tt}"));
        };
        match ident.to_string().as_str() {
            "center" => args.center = true,
            "offset" => {
                // Expect `= [x, y, z]`.
                match trees.next() {
                    Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                    other => {
                        return Err(format!(
                            "include_obj!: expected `=` after `offset`, found {}",
                            describe(other.as_ref())
                        ));
                    }
                }
                match trees.next() {
                    Some(TokenTree::Group(g)) => {
                        args.offset = parse_f32_triple(&g.stream())?;
                    }
                    other => {
                        return Err(format!(
                            "include_obj!: expected `[x, y, z]` after `offset =`, found {}",
                            describe(other.as_ref())
                        ));
                    }
                }
            }
            other => return Err(format!("include_obj!: unknown setting `{other}`")),
        }
    }

    Ok(args)
}

/// Parse three comma-separated float literals (the body of an `[x, y, z]`).
fn parse_f32_triple(stream: &TokenStream) -> Result<[f32; 3], String> {
    let mut out = [0.0f32; 3];
    let mut i = 0;
    let mut pending_neg = false;
    for tt in stream.clone() {
        match tt {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                pending_neg = false;
            }
            TokenTree::Punct(p) if p.as_char() == '-' => {
                pending_neg = true;
            }
            TokenTree::Literal(lit) => {
                if i >= 3 {
                    return Err("include_obj!: offset takes exactly 3 numbers".into());
                }
                let v: f32 = lit
                    .to_string()
                    .parse()
                    .map_err(|_| format!("include_obj!: `{lit}` is not a number"))?;
                out[i] = if pending_neg { -v } else { v };
                pending_neg = false;
                i += 1;
            }
            other => return Err(format!("include_obj!: unexpected token `{other}` in offset")),
        }
    }
    if i != 3 {
        return Err("include_obj!: offset takes exactly 3 numbers, e.g. [0.0, -0.2, 0.0]".into());
    }
    Ok(out)
}

/// Strip surrounding double quotes from a string-literal token's text.
fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

/// A short human description of an optional token, for error messages.
fn describe(tt: Option<&TokenTree>) -> String {
    match tt {
        Some(tt) => format!("`{tt}`"),
        None => "end of input".to_string(),
    }
}

/// Produce a `compile_error!` token stream with the given message.
fn compile_error(msg: &str) -> TokenStream {
    TokenStream::from_str(&format!("compile_error!({msg:?})")).expect("valid compile_error tokens")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_pack_is_little_endian() {
        assert_eq!(fifo_pack([0x40, 0x21, 0x23, 0x41]), 0x4123_2140);
        // NOP padding lands in the high bytes.
        assert_eq!(fifo_pack([FIFO_END, FIFO_NOP, FIFO_NOP, FIFO_NOP]), 0x0000_0041);
    }

    /// One triangle → BEGIN + 3×(NORMAL, VERTEX16) + END = 8 ops, packed four to
    /// a command word with their args interleaved, and a correct length header.
    #[test]
    fn single_triangle_display_list_layout() {
        let tris = parse_obj(
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//1 2//1 3//1\n",
        )
        .unwrap();
        assert_eq!(tris.len(), 1);

        let (words, _aabb) = display_list(&tris);

        // 8 ops → 2 command words. Args: BEGIN 1, NORMAL 1 (×3), VERTEX16 2 (×3),
        // END 0 = 1 + 3 + 6 = 10 arg words. Body = 2 + 10 = 12; +1 length word.
        assert_eq!(words[0], 12, "length header counts body words only");
        assert_eq!(words.len(), 13);

        // First command word packs BEGIN, NORMAL, VERTEX16, NORMAL.
        assert_eq!(words[1], fifo_pack([FIFO_BEGIN, FIFO_NORMAL, FIFO_VERTEX16, FIFO_NORMAL]));
        // BEGIN's argument is the GL_TRIANGLES selector.
        assert_eq!(words[2], GL_TRIANGLES);

        // chunk0 args (BEGIN1 + NORMAL1 + VERTEX16:2 + NORMAL1 = 5) put the second
        // command word at index 7, packing the run's tail: VERTEX16, NORMAL,
        // VERTEX16, END (exactly four ops, so no NOP padding needed here).
        assert_eq!(words[7], fifo_pack([FIFO_VERTEX16, FIFO_NORMAL, FIFO_VERTEX16, FIFO_END]));
    }

    /// The packing math must stay identical to `bevy_nds_3d::ffi` so baked words
    /// mean the same thing as the runtime path (4.12 fixed; v10 signed normals).
    #[test]
    fn packing_matches_hardware_format() {
        // 1.0 in 4.12 fixed is 0x1000; packed (x,y) low/high halves.
        assert_eq!(vertex16(1.0, 0.0, 0.0), (0x0000_1000, 0x0000_0000));
        // -1.0 → 0xF000 as i16, zero-extended into the half word.
        assert_eq!(vertex16(-1.0, 0.0, 0.0).0 & 0xFFFF, 0xF000);

        // Unit +Z normal: only the z field (bits 20..30) is set to +0.998 (0x1FF).
        assert_eq!(normal_pack(0.0, 0.0, 1.0), 0x1FF << 20);
        // float_to_v10 clamps to the representable signed range.
        assert_eq!(float_to_v10(2.0), 0x1FF);
        assert_eq!(float_to_v10(-2.0), 0x200);
    }

    /// Quad faces are fan-triangulated into two triangles.
    #[test]
    fn quads_are_fan_triangulated() {
        let tris = parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").unwrap();
        assert_eq!(tris.len(), 2);
    }
}
