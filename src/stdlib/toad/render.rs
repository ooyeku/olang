//! The renderer: a frame's parts drawn in order into an HDR target, lit,
//! bloomed, graded and tone mapped to an 8-bit frame.
//!
//! - **Swarms** are one instanced draw each, whose vertex shader reads the
//!   swarm's columns straight from its storage buffer: glowing points,
//!   added. They give light.
//! - **Shapes** are signed distance fields (circle, box, capsule, ring,
//!   triangle; a line is a capsule), any size, rotated, rounded or
//!   outlined, antialiased by their distance. A shape is a surface (its
//!   colour times the light where it is), may `emit` light of its own,
//!   and may `cast` (block light: a shadow).
//! - **Sprites** are pictures `gui` decodes, rotated and tinted, lit as
//!   surfaces.
//!
//! **Light** (when the look asks for it: `gi` above zero) is 2D global
//! illumination by radiance cascades, at a quarter of the frame's size:
//! 1. the light scene: what emits (emitting shapes, swarms) and what
//!    casts, drawn small;
//! 2. its distance field, by jump flooding, so rays step over empty space;
//! 3. the cascades, far to near: cascade n has probes every 2^n texels and
//!    4^(n+1) directions, each ray marching its own stretch of distance
//!    (4^n long) and, where it reaches the end unblocked, taking the
//!    light the next cascade found further on. Cascade 0 is the light
//!    arriving at every texel, soft shadows and colour bleeding included.
//!
//! **The finish**: bloom (a chain of halvings and a tent filter back up),
//! then exposure, contrast, saturation, a tint, vignette and grain, then
//! AgX tone mapping.

use std::collections::HashMap;
use std::sync::Arc;

/// The camera and how the pass draws, shared by every pass that draws the
/// scene: the target's size in pixels, the world point at its centre,
/// pixels per world unit, its turn, the mode (0 the frame, 1 the light
/// scene), the point radius scale, the ambient light, the light's gain,
/// whether surfaces are lit, and the clock (seconds: patterns that move).
const VIEW: &str = r#"
struct V { size: vec2<f32>, center: vec2<f32>, scale: f32, rot: f32, mode: f32, pxk: f32,
           ambient: vec4<f32>, gi: f32, lit: f32, time: f32, pad1: f32 };
fn to_clip(v: V, p: vec2<f32>) -> vec4<f32> {
    let c = cos(-v.rot); let s = sin(-v.rot);
    let q = p - v.center;
    let r = vec2(q.x * c - q.y * s, q.x * s + q.y * c) * v.scale;
    return vec4(r / (v.size * 0.5), 0.0, 1.0);
}
fn turn(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a); let s = sin(a);
    return vec2(p.x * c - p.y * s, p.x * s + p.y * c);
}
/// What a pass writes: linear light, or (the screen, mode 3, drawn over
/// the finished frame) display-encoded sRGB, premultiplied either way.
fn out_of(c: vec4<f32>) -> vec4<f32> {
    if (v.mode < 2.5) { return c; }
    let a = max(c.a, 1e-5);
    let l = clamp(c.rgb / a, vec3(0.0), vec3(1.0));
    let e = select(1.055 * pow(l, vec3(1.0 / 2.4)) - 0.055, l * 12.92, l <= vec3(0.0031308));
    return vec4(e * c.a, c.a);
}
fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
                                vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    return c[i];
}
"#;

/// The light arriving at a pixel of the frame (white when unlit).
const LIGHT: &str = r#"
@group(1) @binding(0) var light_tex: texture_2d<f32>;
@group(1) @binding(1) var light_smp: sampler;
fn light_at(v: V, at: vec4<f32>) -> vec3<f32> {
    return light_off(v, at, vec2(0.0));
}
/// The light `off` world units away from the pixel `at` (a blocker's face
/// takes the light just outside it: inside, none arrives).
fn light_off(v: V, at: vec4<f32>, off: vec2<f32>) -> vec3<f32> {
    if (v.lit < 0.5) { return vec3(1.0); }
    let o = turn(off, -v.rot) * v.scale;
    let l = textureSampleLevel(light_tex, light_smp, (at.xy + vec2(o.x, -o.y)) / v.size, 0.0).rgb;
    return v.ambient.rgb + l * v.gi;
}
"#;

const POINTS: &str = r#"
struct D { cap: u32, n: u32, pos: u32, glow: i32, dead: i32, pad: u32, radius: f32, light: f32, color: vec4<f32>, reach: f32, pad3: f32, pad4: f32, pad5: f32 };
@group(0) @binding(0) var<storage, read> s: array<f32>;
@group(0) @binding(1) var<uniform> v: V;
@group(0) @binding(2) var<uniform> d: D;
struct O { @builtin(position) at: vec4<f32>, @location(0) q: vec2<f32>, @location(1) k: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    var o: O;
    let c = corner(vi);
    var k = 1.0;
    if (d.glow >= 0) { k = s[u32(d.glow) * d.cap + i]; }
    if (d.dead >= 0 && s[u32(d.dead) * d.cap + i] > 0.5) { k = 0.0; }
    let p = vec2(s[d.pos * d.cap + i], s[(d.pos + 1u) * d.cap + i]);
    // the radius is in the frame's pixels, whatever the zoom; a pool of
    // light (mode 2) reaches `reach` world units
    var r = d.radius * v.pxk;
    if (v.mode > 1.5) { r = d.reach * v.scale; }
    o.at = to_clip(v, p) + vec4(c * max(r, 1.0) / (v.size * 0.5), 0.0, 0.0);
    // swarms are not in the light scene (mode 1): their light is pooled
    if (k <= 0.0 || (v.mode > 0.5 && v.mode < 1.5)) { o.at = vec4(2.0, 2.0, 2.0, 1.0); }
    o.q = c;
    o.k = k;
    return o;
}
@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    let r2 = dot(o.q, o.q);
    if (r2 > 1.0) { discard; }
    // a hot core and a soft halo
    let core = exp(-r2 * 18.0);
    let halo = exp(-r2 * 4.0) * 0.35;
    var e = (core + halo) * o.k * d.color.a;
    // a pool of its light on what is near: smooth to nothing at its reach
    if (v.mode > 1.5) { let f = 1.0 - r2; e = f * f * o.k * d.color.a * d.light; }
    return vec4(d.color.rgb * e, 0.0);
}
"#;

/// A shape: centre, half size, turn, kind, rounding, outline, colour
/// (straight, linear), emission, whether it casts, then its material:
/// relief, gloss, a second colour, a pattern (kind, scale, speed, seed,
/// angle), whether it is unlit, its bump, and room.
pub const SHAPE_FLOATS: usize = 32;

/// The patterns a material draws (`pattern`), by number.
pub const PATTERNS: &[&str] = &["none", "noise", "stripes", "dots", "water", "rings", "checker", "gradient", "radial", "cells"];

const SHAPES: &str = r#"
struct S { c: vec2<f32>, half: vec2<f32>, rot: f32, kind: f32, round: f32, line: f32,
           color: vec4<f32>, emit: f32, blocks: f32, relief: f32, gloss: f32,
           color2: vec4<f32>, pattern: f32, pscale: f32, pspeed: f32, seed: f32,
           pangle: f32, unlit: f32, bump: f32, pad0: f32, pad1: vec4<f32> };
@group(0) @binding(0) var<storage, read> sh: array<S>;
@group(0) @binding(1) var<uniform> v: V;
struct O { @builtin(position) at: vec4<f32>, @location(0) q: vec2<f32>, @location(1) @interpolate(flat) i: u32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    let s = sh[i];
    // the quad holds the shape, its outline and a pixel for the edge
    let pad = s.line * 0.5 + 1.5 / v.scale;
    let q = corner(vi) * (s.half + vec2(pad));
    var o: O;
    o.at = to_clip(v, s.c + turn(q, s.rot));
    // the light scene draws only what emits or casts
    if (v.mode > 0.5 && v.mode < 2.5 && s.emit <= 0.0 && s.blocks <= 0.0) { o.at = vec4(2.0, 2.0, 2.0, 1.0); }
    o.q = q;
    o.i = i;
    return o;
}
fn sd_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
fn sd_tri(p0: vec2<f32>, r: f32) -> f32 {
    // an equilateral triangle, point up, circumradius r
    let k = sqrt(3.0);
    var p = vec2(abs(p0.x) - r * 0.866, p0.y + r * 0.5);
    if (p.x + k * p.y > 0.0) { p = vec2(p.x - k * p.y, -k * p.x - p.y) / 2.0; }
    p.x -= clamp(p.x, -r * 1.732, 0.0);
    return -length(p) * sign(p.y);
}
fn sdf(s: S, p: vec2<f32>) -> f32 {
    var d = 0.0;
    let kind = u32(s.kind + 0.5);
    if (kind == 0u) { d = length(p) - s.half.x; }                       // circle
    else if (kind == 1u) { d = sd_box(p, s.half, min(s.round, min(s.half.x, s.half.y))); } // box
    else if (kind == 2u) {                                               // capsule along x
        let r = s.half.y;
        let x = clamp(p.x, -(s.half.x - r), s.half.x - r);
        d = length(p - vec2(x, 0.0)) - r;
    }
    else if (kind == 3u) { d = sd_tri(p, s.half.x); }                    // triangle
    if (s.line > 0.0) { d = abs(d) - s.line * 0.5; }
    return d;
}

// ── the material: procedural patterns ─────────────────────────────────
fn hash2(p: vec2<i32>, seed: u32) -> f32 {
    var h = u32(p.x) * 374761393u + u32(p.y) * 668265263u + seed * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967295.0;
}
fn vnoise(p: vec2<f32>, seed: u32) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i, seed);
    let b = hash2(i + vec2(1, 0), seed);
    let c = hash2(i + vec2(0, 1), seed);
    let d = hash2(i + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p0: vec2<f32>, seed: u32, octaves: u32) -> f32 {
    var p = p0; var a = 0.5; var t = 0.0; var n = 0.0;
    for (var i = 0u; i < octaves; i++) {
        t += vnoise(p, seed + i) * a;
        n += a;
        p = vec2(p.x * 1.6 - p.y * 1.2, p.x * 1.2 + p.y * 1.6);
        a *= 0.5;
    }
    return t / n;
}
// the nearest of a grid of jittered points: (distance, the next nearest)
fn cells(p: vec2<f32>, seed: u32) -> vec2<f32> {
    let i = vec2<i32>(floor(p));
    var d1 = 9.0; var d2 = 9.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = i + vec2(x, y);
            let o = vec2(hash2(c, seed), hash2(c, seed + 7u));
            let d = length(vec2<f32>(c) + o - p);
            if (d < d1) { d2 = d1; d1 = d; } else if (d < d2) { d2 = d; }
        }
    }
    return vec2(d1, d2);
}
// how far toward `color2` the pattern is at local point `p` (0 to 1)
fn pattern_at(s: S, p: vec2<f32>) -> f32 {
    let kind = u32(s.pattern + 0.5);
    if (kind == 0u) { return 0.0; }
    let seed = u32(s.seed);
    let t = v.time * s.pspeed;
    let ca = cos(s.pangle); let sa = sin(s.pangle);
    let r = vec2(p.x * ca + p.y * sa, -p.x * sa + p.y * ca);
    let q = r / max(s.pscale, 0.001);
    if (kind == 1u) { return fbm(q + vec2(t, t * 0.6), seed, 4u); }                       // noise
    if (kind == 2u) { return 0.5 + 0.5 * sin((q.x + t) * 6.2831853); }                // stripes
    if (kind == 3u) {                                                                  // dots
        let g = fract(q + vec2(t, 0.0)) - 0.5;
        return 1.0 - smoothstep(0.22, 0.3, length(g));
    }
    if (kind == 4u) {                                                                  // water
        let a = fbm(q * 0.8 + vec2(t * 0.35, t * 0.2), seed, 2u);
        let b = fbm(q * 1.7 - vec2(t * 0.25, -t * 0.4) + vec2(a * 1.5), seed + 9u, 3u);
        return clamp(b * 1.25 - 0.15, 0.0, 1.0);
    }
    if (kind == 5u) { return 0.5 + 0.5 * sin((length(q) - t) * 6.2831853); }          // rings
    if (kind == 6u) {                                                                  // checker
        let c = vec2<i32>(floor(q + vec2(t, 0.0)));
        return f32((c.x + c.y) & 1);
    }
    if (kind == 7u) {                                                                  // gradient
        let ext = max(abs(s.half.x * ca) + abs(s.half.y * sa), 0.001);
        return clamp(r.x / ext * 0.5 + 0.5, 0.0, 1.0);
    }
    if (kind == 8u) { return clamp(length(p) / max(max(s.half.x, s.half.y), 0.001), 0.0, 1.0); } // radial
    if (kind == 9u) {                                                                  // cells
        let c = cells(q + vec2(t, 0.0), seed);
        return clamp(1.0 - (c.y - c.x) * 2.5, 0.0, 1.0);
    }
    return 0.0;
}


@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    let s = sh[o.i];
    let p = o.q;
    let d = sdf(s, p);
    let cov = clamp(0.5 - d * v.scale, 0.0, 1.0);
    if (cov <= 0.0) { discard; }
    // the pattern once: the colour, and (with a bump) the slope from it
    var pv = 0.0;
    if (s.pattern > 0.5) { pv = pattern_at(s, p); }
    let alb = mix(s.color.rgb, s.color2.rgb, pv);
    let e = alb * s.emit * cov;
    if (v.mode > 0.5 && v.mode < 2.5) {
        // the light scene: its light, and whether it blocks
        return vec4(e, cov * s.blocks);
    }
    let a = cov * s.color.a;
    let lit = v.lit > 0.5 && s.unlit < 0.5;
    // the surface's normal: a bevel `relief` deep at its edges, and the
    // pattern's bumps
    let k = 0.5;
    let g = vec2(sdf(s, p + vec2(k, 0.0)) - sdf(s, p - vec2(k, 0.0)),
                 sdf(s, p + vec2(0.0, k)) - sdf(s, p - vec2(0.0, k)));
    let out_n = normalize(g + vec2(1e-6));
    var nrm = vec3(0.0, 0.0, 1.0);
    if (s.relief > 0.0) {
        let edge = 1.0 - clamp(max(-d, 0.0) / s.relief, 0.0, 1.0);
        nrm = normalize(vec3(out_n * edge * edge * 1.8, 1.0));
    }
    if (s.bump > 0.0 && s.pattern > 0.5) {
        let h = max(s.pscale, 0.001) * 0.15;
        let gx = pattern_at(s, p + vec2(h, 0.0)) - pv;
        let gy = pattern_at(s, p + vec2(0.0, h)) - pv;
        nrm = normalize(nrm + vec3(-gx, -gy, 0.0) * s.bump * 4.0);
    }
    let n_w = vec3(turn(nrm.xy, s.rot), nrm.z);
    let shaded = s.relief > 0.0 || s.gloss > 0.0 || (s.bump > 0.0 && s.pattern > 0.5);
    let w3 = vec3(0.2126, 0.7152, 0.0722);
    if (!shaded || !lit) {
        // unlit: a fixed sun from the upper left shows the relief
        var l = vec3(1.0);
        if (lit) {
            // a blocker takes the light just outside its nearest edge
            if (s.blocks > 0.5) {
                l = light_off(v, o.at, turn(out_n, s.rot) * (max(-d, 0.0) + 6.0 / v.scale));
            } else {
                l = light_at(v, o.at);
            }
        }
        if (!shaded) { return out_of(vec4(alb * l * a + e, a)); }
        let ld = normalize(vec3(-0.45, 0.6, 0.66));
        let diffuse = clamp(dot(n_w, ld) / ld.z, 0.0, 2.5);
        let hv = normalize(ld + vec3(0.0, 0.0, 1.0));
        let spec = pow(max(dot(n_w, hv), 0.0), mix(8.0, 90.0, s.gloss)) * s.gloss * 1.2;
        return out_of(vec4((alb * diffuse + vec3(spec)) * l * a + e, a));
    }
    // lit and shaded: the light around it from four sides (outside it for
    // a blocker, where light arrives; around the pixel otherwise), each
    // side lighting what faces it
    let dirs = array<vec2<f32>, 4>(vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0));
    var side: array<vec3<f32>, 4>;
    var here = vec3(0.0);
    if (s.blocks > 0.5) {
        // from the pixel to just past the shape's far reach on each side
        let reach = max(s.half.x, s.half.y) + s.line + 6.0 / v.scale;
        let me = turn(p, s.rot);
        for (var k = 0u; k < 4u; k++) { side[k] = light_off(v, o.at, dirs[k] * reach - me); }
    } else {
        let r = 12.0 / v.scale;
        for (var k = 0u; k < 4u; k++) { side[k] = light_off(v, o.at, dirs[k] * r); }
        here = light_at(v, o.at);
    }
    var l = vec3(0.0);
    var wsum = 0.0;
    var pull = vec2(0.0);
    var lsum = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let lk = dot(side[k], w3);
        pull += dirs[k] * lk;
        lsum += lk;
        let w = max(dot(n_w, normalize(vec3(dirs[k], 0.6))), 0.0);
        l += side[k] * w;
        wsum += 0.5145;
    }
    l = l / wsum;
    // how one-sided the light is: 0 all round, 1 from one side
    let dirk = clamp(length(pull) / max(lsum, 1e-4) * 3.0, 0.0, 1.0);
    if (s.blocks < 0.5) {
        // a surface away from blockers: its own light, shaded by its facing
        let flat_l = here;
        let facing = l / max(dot((side[0] + side[1] + side[2] + side[3]) * 0.25, w3), 1e-4) * dot(here, w3);
        l = mix(flat_l * nrm.z, facing, dirk);
    }
    let ld = normalize(vec3(pull / max(length(pull), 1e-6), 0.6));
    let hv = normalize(ld + vec3(0.0, 0.0, 1.0));
    let spec = pow(max(dot(n_w, hv), 0.0), mix(8.0, 90.0, s.gloss)) * s.gloss * 1.6 * dirk;
    return out_of(vec4((alb + vec3(spec)) * l * a + e, a));
}
"#;

/// A sprite: centre, half size, turn, whether it is unlit, its light
/// (`emit`), whether it casts, tint (rgba: past 1 glows).
pub const SPRITE_FLOATS: usize = 12;

const SPRITES: &str = r#"
struct P { c: vec2<f32>, half: vec2<f32>, rot: f32, unlit: f32, emit: f32, blocks: f32, tint: vec4<f32> };
@group(0) @binding(0) var<storage, read> sp: array<P>;
@group(0) @binding(1) var<uniform> v: V;
@group(0) @binding(2) var pic: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
struct O { @builtin(position) at: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) @interpolate(flat) i: u32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> O {
    let s = sp[i];
    let c = corner(vi);
    var o: O;
    o.at = to_clip(v, s.c + turn(c * s.half, s.rot));
    // the light scene draws only the sprites that give or block light
    if (v.mode > 0.5 && v.mode < 2.5 && s.emit <= 0.0 && s.blocks <= 0.0) { o.at = vec4(2.0, 2.0, 2.0, 1.0); }
    o.uv = vec2(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    o.i = i;
    return o;
}
@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    // pictures are premultiplied; the texture is sRGB, so this is linear
    let s = sp[o.i];
    let t = s.tint;
    let c = textureSample(pic, smp, o.uv);
    if (v.mode > 0.5 && v.mode < 2.5) {
        // the light scene: its colours as light, its shape as a blocker
        return vec4(c.rgb * t.rgb * s.emit * t.a, select(0.0, c.a, s.blocks > 0.5 && c.a > 0.5));
    }
    var l = vec3(1.0);
    if (s.unlit < 0.5 && v.lit > 0.5) {
        if (s.blocks > 0.5) {
            // a blocker: no light inside it, so the light just outside each
            // side, blended across it
            let local = vec2((o.uv.x - 0.5) * 2.0 * s.half.x, (0.5 - o.uv.y) * 2.0 * s.half.y);
            let m = 6.0 / v.scale;
            let lr = light_off(v, o.at, turn(vec2(s.half.x + m, 0.0) - local, s.rot));
            let ll = light_off(v, o.at, turn(vec2(-s.half.x - m, 0.0) - local, s.rot));
            let lt = light_off(v, o.at, turn(vec2(0.0, s.half.y + m) - local, s.rot));
            let lb = light_off(v, o.at, turn(vec2(0.0, -s.half.y - m) - local, s.rot));
            l = (mix(ll, lr, o.uv.x) + mix(lt, lb, o.uv.y)) * 0.5;
        } else {
            l = light_at(v, o.at);
        }
    }
    return out_of(vec4(c.rgb * t.rgb * l + c.rgb * t.rgb * s.emit, c.a) * t.a);
}
"#;

/// A pass over the whole target.
const FULL: &str = r#"
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0))[vi];
    return vec4(p, 0.0, 1.0);
}
"#;

/// Jump flooding: each texel's nearest light or blocker. The seed pass
/// marks them; each step looks `step` texels away in eight directions.
const JFA: &str = r#"
struct J { step: f32, seed: f32, pad0: f32, pad1: f32 };
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> j: J;
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(at.xy);
    let dim = vec2<i32>(textureDimensions(src));
    if (j.seed > 0.5) {
        let c = textureLoad(src, p, 0);
        if (c.a > 0.1 || max(c.r, max(c.g, c.b)) > 0.001) { return vec4(at.xy, 0.0, 1.0); }
        return vec4(-1.0, -1.0, 0.0, 0.0);
    }
    var best = vec4(-1.0, -1.0, 0.0, 0.0);
    var bd = 1e9;
    let st = i32(j.step);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = p + vec2(x, y) * st;
            if (q.x < 0 || q.y < 0 || q.x >= dim.x || q.y >= dim.y) { continue; }
            let s = textureLoad(src, q, 0);
            if (s.x < 0.0) { continue; }
            let d = distance(at.xy, s.xy);
            if (d < bd) { bd = d; best = s; }
        }
    }
    return best;
}
"#;

/// The distance to the nearest light or blocker, in texels.
const DIST: &str = r#"
@group(0) @binding(0) var seeds: texture_2d<f32>;
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let s = textureLoad(seeds, vec2<i32>(at.xy), 0);
    if (s.x < 0.0) { return vec4(1e4); }
    return vec4(distance(at.xy, s.xy));
}
"#;

/// One cascade. Texel (block, probe): `probe` the probe's place in its
/// grid, `block` which four of the cascade's directions it holds; the
/// texel answers those four rays' mean, each merged with the cascade
/// above where it ran out unblocked.
const CASCADE: &str = r#"
struct C { n: f32, count: f32, interval: f32, pad: f32 };
@group(0) @binding(0) var dist: texture_2d<f32>;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var upper: texture_2d<f32>;
@group(0) @binding(3) var<uniform> c: C;
const TAU: f32 = 6.28318530718;

fn march(o: vec2<f32>, dir: vec2<f32>, start: f32, len: f32, dim: vec2<f32>) -> vec4<f32> {
    var t = start;
    let end = start + len;
    var seen = vec3(0.0);
    for (var i = 0; i < 32; i++) {
        let p = o + dir * t;
        if (p.x < 0.0 || p.y < 0.0 || p.x >= dim.x || p.y >= dim.y) { return vec4(seen, 1.0); }
        let d = textureLoad(dist, vec2<i32>(p), 0).r;
        if (d < 0.75) {
            let s = textureLoad(scene, vec2<i32>(p), 0);
            // a blocker ends the ray, seeing its own light (often none)
            if (s.a > 0.5) { return vec4(seen + s.rgb, 0.0); }
            // a light that blocks nothing (a firefly: no shadow of it) is
            // seen once and passed: the ray steps over it
            seen += s.rgb;
            t += 4.0;
        } else {
            t += max(d - 0.5, 0.75);
        }
        if (t >= end) { break; }
    }
    return vec4(seen, 1.0);
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(scene));
    let dimu = vec2<u32>(textureDimensions(scene));
    let n = u32(c.n);
    let spacing = 1u << n;
    let size = max(dimu / spacing, vec2(1u));
    let coord = vec2<u32>(at.xy);
    let block = coord / size;
    if (block.x >= spacing || block.y >= spacing) { return vec4(0.0); }
    let probe = coord % size;
    let rays = f32(spacing * spacing * 4u);
    let base = 4u * (block.x + spacing * block.y);
    let center = (vec2<f32>(probe) + 0.5) * f32(spacing);
    let pw = pow(4.0, f32(n));
    let start = c.interval * (pw - 1.0) / 3.0;
    let len = c.interval * pw;
    let up_spacing = spacing * 2u;
    let up_size = max(dimu / up_spacing, vec2(1u));
    var total = vec4(0.0);
    for (var i = 0u; i < 4u; i++) {
        let index = base + i;
        let a = (f32(index) + 0.5) * TAU / rays;
        let dir = vec2(cos(a), sin(a));
        if (f32(n) + 1.0 >= c.count) {
            total += march(center, dir, start, len, dim);
            continue;
        }
        // the bilinear fix: a ray to each of the four probes above, from
        // where this interval starts to where theirs does, merged with
        // what that probe saw further on, the four weighted by nearness
        let ub = vec2(index % up_spacing, index / up_spacing);
        let pos = center / f32(up_spacing) - 0.5;
        let p0 = floor(pos);
        let f = pos - p0;
        let a0 = center + dir * start;
        var acc = vec4(0.0);
        for (var k = 0u; k < 4u; k++) {
            let o = vec2(f32(k & 1u), f32(k >> 1u));
            let q = clamp(vec2<i32>(p0 + o), vec2<i32>(0), vec2<i32>(up_size) - vec2<i32>(1));
            let w = select(1.0 - f.x, f.x, o.x > 0.5) * select(1.0 - f.y, f.y, o.y > 0.5);
            let to = (vec2<f32>(q) + 0.5) * f32(up_spacing) + dir * (start + len);
            let seg = to - a0;
            let l = max(length(seg), 0.001);
            var r = march(a0, seg / l, 0.0, l, dim);
            if (r.a > 0.5) {
                let u = textureLoad(upper, vec2<i32>(ub * up_size) + q, 0);
                r = vec4(r.rgb + u.rgb, u.a);
            }
            acc += r * w;
        }
        total += acc;
    }
    return total * 0.25;
}
"#;

/// Bloom: a halving with a 13-tap filter, and a tent back up added in.
const BLOOM_DOWN: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(src));
    let uv = at.xy * 2.0 * px;
    var c = textureSampleLevel(src, smp, uv, 0.0).rgb * 0.125;
    c += (textureSampleLevel(src, smp, uv + px * vec2(-1.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(-1.0, 1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(1.0, 1.0), 0.0).rgb) * 0.125;
    c += (textureSampleLevel(src, smp, uv + px * vec2(-2.0, -2.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(2.0, -2.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(-2.0, 2.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(2.0, 2.0), 0.0).rgb) * 0.03125;
    c += (textureSampleLevel(src, smp, uv + px * vec2(-2.0, 0.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(2.0, 0.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(0.0, -2.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(0.0, 2.0), 0.0).rgb) * 0.0625;
    // keep it finite and bounded: one hot texel is not a sun
    return vec4(min(c, vec3(64.0)), 1.0);
}
"#;

const BLOOM_UP: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let dst = vec2<f32>(textureDimensions(src)) * 2.0;
    let uv = at.xy / dst;
    let px = 1.0 / vec2<f32>(textureDimensions(src));
    var c = textureSampleLevel(src, smp, uv, 0.0).rgb * 4.0;
    c += (textureSampleLevel(src, smp, uv + px * vec2(-1.0, 0.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(1.0, 0.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(0.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(0.0, 1.0), 0.0).rgb) * 2.0;
    c += textureSampleLevel(src, smp, uv + px * vec2(-1.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(-1.0, 1.0), 0.0).rgb
        + textureSampleLevel(src, smp, uv + px * vec2(1.0, 1.0), 0.0).rgb;
    // each wider level counts for less: a glow, not a fog
    return vec4(c / 16.0 * 0.6, 1.0);
}
"#;

const TONE: &str = r#"
struct T { bg: vec4<f32>, tint: vec4<f32>, exposure: f32, vignette: f32, bloom: f32, contrast: f32,
           saturation: f32, grain: f32, seed: f32, pad: f32 };
@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> t: T;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
// AgX-style: a log encoding and a fitted sigmoid, so bright colours
// desaturate toward white instead of clipping into flat hues.
fn agx(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(vec3(0.842, 0.0423, 0.0424), vec3(0.0784, 0.878, 0.0784), vec3(0.0792, 0.0792, 0.879));
    var x = m * max(c, vec3(1e-10));
    x = clamp((log2(x) + 12.47) / 16.5, vec3(0.0), vec3(1.0));
    let x2 = x * x; let x4 = x2 * x2;
    var y = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
    let mi = mat3x3<f32>(vec3(1.197, -0.0528, -0.0529), vec3(-0.0980, 1.152, -0.0980), vec3(-0.0990, -0.0990, 1.152));
    return clamp(mi * y, vec3(0.0), vec3(1.0));
}
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(hdr));
    var c = textureLoad(hdr, vec2<i32>(at.xy), 0).rgb + t.bg.rgb;
    c += textureSampleLevel(bloom, smp, at.xy / dim, 0.0).rgb * t.bloom;
    c *= t.exposure * t.tint.rgb;
    // contrast about middle grey, in log space; saturation about luminance
    let grey = 0.18;
    c = grey * pow(max(c, vec3(0.0)) / grey, vec3(t.contrast));
    let lum = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3(lum), c, t.saturation), vec3(0.0));
    let uv = at.xy / dim - 0.5;
    var o = agx(c) * (1.0 - t.vignette * dot(uv, uv) * 1.6);
    o += (hash(at.xy + t.seed) - 0.5) * t.grain;
    // AgX's output is display-encoded already
    return vec4(clamp(o, vec3(0.0), vec3(1.0)), 1.0);
}
"#;

/// A swarm's part of a frame.
pub struct Layer<'a> {
    pub buf: &'a wgpu::Buffer,
    pub cap: usize,
    pub n: usize,
    pub pos: usize,
    pub glow: Option<usize>,
    pub dead: Option<usize>,
    pub color: [f32; 4],
    pub radius: f32,
    /// How much light each point gives in a lit look,
    pub light: f32,
    /// and how far it reaches (world units).
    pub reach: f32,
}

/// One part of a frame, drawn in order.
pub enum Part<'a> {
    Swarm(Layer<'a>),
    /// `SHAPE_FLOATS` per shape; whether they are on the screen (points
    /// from the top left, y down, no camera, no light) rather than in the
    /// world.
    Shapes(Vec<f32>, bool),
    /// Runs of one picture: `SPRITE_FLOATS` per sprite; on the screen.
    Sprites(Vec<(Arc<crate::stdlib::gui::canvas::Picture>, Vec<f32>)>, bool),
}

/// The camera and the finish.
pub struct View {
    pub size: (u32, u32),
    pub center: (f32, f32),
    pub scale: f32,
    pub rot: f32,
    pub bg: [f32; 3],
    pub exposure: f32,
    pub vignette: f32,
    /// The light's gain (0: no light pass; surfaces show their colour).
    pub gi: f32,
    /// Light everywhere, linear.
    pub ambient: [f32; 3],
    pub bloom: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub tint: [f32; 3],
    pub grain: f32,
    pub seed: f32,
    /// The light pass's size: the frame's over this (2: half, 4: quarter).
    pub gi_down: u32,
    /// Seconds, for patterns that move.
    pub time: f32,
    /// Pixels per point on the screen layers.
    pub ui_scale: f32,
}

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SEEDS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// Cascades: enough that the last reaches across the light scene.
const CASCADES: u32 = 6;
/// Bloom's halvings.
const BLOOM_LEVELS: usize = 6;

/// Every texture a frame of one size draws through: the HDR target and
/// the output, the light scene and its distance field and cascades, and
/// bloom's chain. A live view keeps its set while its size holds.
struct Targets {
    size: (u32, u32),
    gi_down: u32,
    hdr: wgpu::Texture,
    out: wgpu::Texture,
    gi_scene: wgpu::Texture,
    seeds: [wgpu::Texture; 2],
    dist: wgpu::Texture,
    cascades: [wgpu::Texture; 2],
    bloom: Vec<wgpu::Texture>,
}

type Pipe = (wgpu::RenderPipeline, wgpu::BindGroupLayout);

pub struct Renderer {
    gpu: Arc<crate::stdlib::gui::gpu::Gpu>,
    points: Pipe,
    shapes: Pipe,
    sprites: Pipe,
    /// The same two drawing onto the finished frame (the screen).
    shapes_ui: wgpu::RenderPipeline,
    sprites_ui: wgpu::RenderPipeline,
    light_layout: wgpu::BindGroupLayout,
    jfa: Pipe,
    dist: Pipe,
    cascade: Pipe,
    bloom_down: Pipe,
    bloom_up: Pipe,
    tone: Pipe,
    sampler: wgpu::Sampler,
    /// A 1×1 white light, for a frame without the light pass.
    white: wgpu::TextureView,
    /// A live view's textures by its name.
    targets: HashMap<String, Targets>,
    /// Sprites' textures by picture id.
    pictures: HashMap<u64, wgpu::TextureView>,
    frame_no: u64,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty,
        count: None,
    }
}

fn storage() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn texture(filterable: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn sampler() -> wgpu::BindingType {
    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let gpu = crate::stdlib::gui::gpu::gpu(None, None)?;
        let d = &gpu.device;
        // a shader that does not compile is an error here, not a frame of
        // nothing and a line in the log
        let scope = d.push_error_scope(wgpu::ErrorFilter::Validation);
        let (v, f, vf) = (
            wgpu::ShaderStages::VERTEX,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let light_layout = layout("toad light", &[entry(0, f, texture(true)), entry(1, f, sampler())]);
        let pipe = |label: &str,
                    src: String,
                    layouts: &[&wgpu::BindGroupLayout],
                    format,
                    blend|
         -> wgpu::RenderPipeline {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let ls: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(|l| Some(*l)).collect();
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &ls,
                ..Default::default()
            });
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let over = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
        let pl = layout("toad points", &[entry(0, v, storage()), entry(1, vf, uniform()), entry(2, vf, uniform())]);
        let points = (pipe("toad points", format!("{VIEW}{POINTS}"), &[&pl, &light_layout], HDR, Some(add)), pl);
        let sl = layout("toad shapes", &[entry(0, vf, storage()), entry(1, vf, uniform())]);
        let shapes = (
            pipe("toad shapes", format!("{VIEW}{LIGHT}{SHAPES}"), &[&sl, &light_layout], HDR, Some(over)),
            sl,
        );
        let pl2 = layout(
            "toad sprites",
            &[entry(0, vf, storage()), entry(1, vf, uniform()), entry(2, f, texture(true)), entry(3, f, sampler())],
        );
        let sprites = (
            pipe("toad sprites", format!("{VIEW}{LIGHT}{SPRITES}"), &[&pl2, &light_layout], HDR, Some(over)),
            pl2,
        );
        let shapes_ui = pipe("toad screen shapes", format!("{VIEW}{LIGHT}{SHAPES}"), &[&shapes.1, &light_layout], OUT, Some(over));
        let sprites_ui = pipe("toad screen sprites", format!("{VIEW}{LIGHT}{SPRITES}"), &[&sprites.1, &light_layout], OUT, Some(over));
        let jl = layout("toad jfa", &[entry(0, f, texture(false)), entry(1, f, uniform())]);
        let jfa = (pipe("toad jfa", format!("{FULL}{JFA}"), &[&jl], SEEDS, None), jl);
        let dl = layout("toad dist", &[entry(0, f, texture(false))]);
        let dist = (pipe("toad dist", format!("{FULL}{DIST}"), &[&dl], HDR, None), dl);
        let cl = layout(
            "toad cascade",
            &[entry(0, f, texture(true)), entry(1, f, texture(true)), entry(2, f, texture(true)), entry(3, f, uniform())],
        );
        let cascade = (pipe("toad cascade", format!("{FULL}{CASCADE}"), &[&cl], HDR, None), cl);
        let bl = layout("toad bloom", &[entry(0, f, texture(true)), entry(1, f, sampler())]);
        let bloom_down = (
            pipe("toad bloom down", format!("{FULL}{BLOOM_DOWN}"), &[&bl], HDR, None),
            bl,
        );
        let bl2 = layout("toad bloom up", &[entry(0, f, texture(true)), entry(1, f, sampler())]);
        let bloom_up = (pipe("toad bloom up", format!("{FULL}{BLOOM_UP}"), &[&bl2], HDR, Some(add)), bl2);
        let tl = layout(
            "toad tone",
            &[entry(0, f, texture(false)), entry(1, f, uniform()), entry(2, f, texture(true)), entry(3, f, sampler())],
        );
        let tone = (pipe("toad tone", format!("{FULL}{TONE}"), &[&tl], OUT, None), tl);
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("toad"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let white = {
            let t = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("toad white"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            t.create_view(&Default::default())
        };
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("toad: the renderer's shaders: {e}"));
        }
        Ok(Renderer {
            gpu,
            points,
            shapes,
            sprites,
            shapes_ui,
            sprites_ui,
            light_layout,
            jfa,
            dist,
            cascade,
            bloom_down,
            bloom_up,
            tone,
            sampler,
            white,
            targets: HashMap::new(),
            pictures: HashMap::new(),
            frame_no: 0,
        })
    }

    pub fn gpu(&self) -> &Arc<crate::stdlib::gui::gpu::Gpu> {
        &self.gpu
    }

    fn buffer(&self, label: &str, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        wgpu::util::DeviceExt::create_buffer_init(
            &self.gpu.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            },
        )
    }

    fn uniform(&self, label: &str, words: &[f32]) -> wgpu::Buffer {
        self.buffer(label, bytemuck::cast_slice(words), wgpu::BufferUsages::UNIFORM)
    }

    /// A sprite's texture, uploaded once per picture (sRGB, premultiplied).
    fn picture(&mut self, pic: &crate::stdlib::gui::canvas::Picture) -> wgpu::TextureView {
        if let Some(v) = self.pictures.get(&pic.id) {
            return v.clone();
        }
        let d = &self.gpu.device;
        let size = wgpu::Extent3d {
            width: pic.width.max(1),
            height: pic.height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("toad sprite"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if pic.rgba.len() >= (pic.width * pic.height * 4) as usize {
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pic.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pic.width * 4),
                    rows_per_image: Some(pic.height),
                },
                size,
            );
        }
        let view = tex.create_view(&Default::default());
        // a game's sprites are few; a long run of new pictures starts over
        if self.pictures.len() > 4096 {
            self.pictures.clear();
        }
        self.pictures.insert(pic.id, view.clone());
        view
    }

    fn make_targets(&self, w: u32, h: u32, down: u32) -> Targets {
        let d = &self.gpu.device;
        let tex = |label: &str, w: u32, h: u32, format, usage| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let (gw, gh) = (w.div_ceil(down), h.div_ceil(down));
        let mut bloom = Vec::new();
        let (mut bw, mut bh) = (w, h);
        for _ in 0..BLOOM_LEVELS {
            bw = (bw / 2).max(1);
            bh = (bh / 2).max(1);
            bloom.push(tex("toad bloom", bw, bh, HDR, rt));
        }
        Targets {
            size: (w, h),
            gi_down: down,
            hdr: tex("toad frame", w, h, HDR, rt),
            out: tex("toad out", w, h, OUT, rt | wgpu::TextureUsages::COPY_SRC),
            gi_scene: tex("toad light scene", gw, gh, HDR, rt),
            seeds: [tex("toad seeds", gw, gh, SEEDS, rt), tex("toad seeds", gw, gh, SEEDS, rt)],
            dist: tex("toad dist", gw, gh, HDR, rt),
            cascades: [tex("toad cascade", gw, gh, HDR, rt), tex("toad cascade", gw, gh, HDR, rt)],
            bloom,
        }
    }

    /// The draws of the parts for one target: (pipeline, bind group,
    /// vertices, instances), the view uniform in `vbuf`.
    #[allow(clippy::type_complexity)]
    fn draws<'s>(
        &'s self,
        parts: &[Part<'_>],
        vbuf: &wgpu::Buffer,
        screen: Option<&wgpu::Buffer>,
        views: &[wgpu::TextureView],
        light: bool,
    ) -> Vec<(&'s wgpu::RenderPipeline, wgpu::BindGroup, u32, u32)> {
        let d = &self.gpu.device;
        let mut out = Vec::new();
        let mut views = views.iter();
        for p in parts {
            match p {
                Part::Swarm(_) if screen.is_some() => {}
                Part::Swarm(l) => {
                    let mut u = [0u32; 16];
                    u[12] = l.reach.to_bits();
                    u[0] = l.cap as u32;
                    u[1] = l.n as u32;
                    u[2] = l.pos as u32;
                    u[3] = l.glow.map(|g| g as i32).unwrap_or(-1) as u32;
                    u[4] = l.dead.map(|g| g as i32).unwrap_or(-1) as u32;
                    u[6] = l.radius.to_bits();
                    u[7] = l.light.to_bits();
                    for i in 0..4 {
                        u[8 + i] = l.color[i].to_bits();
                    }
                    let ub = self.buffer("toad points", bytemuck::cast_slice(&u), wgpu::BufferUsages::UNIFORM);
                    let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("toad points"),
                        layout: &self.points.1,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: l.buf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: vbuf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: ub.as_entire_binding() },
                        ],
                    });
                    out.push((&self.points.0, bg, 6, l.n as u32));
                }
                Part::Shapes(inst, on_screen) if !inst.is_empty() => {
                    // the screen's are drawn on their own, over the finish
                    if *on_screen != screen.is_some() {
                        continue;
                    }
                    let vb = screen.unwrap_or(vbuf);
                    let sb = self.buffer("toad shapes", bytemuck::cast_slice(inst), wgpu::BufferUsages::STORAGE);
                    let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("toad shapes"),
                        layout: &self.shapes.1,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: sb.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: vb.as_entire_binding() },
                        ],
                    });
                    let pipe = if screen.is_some() { &self.shapes_ui } else { &self.shapes.0 };
                    out.push((pipe, bg, 6, (inst.len() / SHAPE_FLOATS) as u32));
                }
                Part::Sprites(runs, on_screen) => {
                    let skip = *on_screen != screen.is_some();
                    let vb = screen.unwrap_or(vbuf);
                    for (_, inst) in runs {
                        let view = views.next().expect("a view per run");
                        if skip {
                            continue;
                        }
                        // in the light scene only the runs that give or block light
                        if inst.is_empty()
                            || (light && !inst.chunks(SPRITE_FLOATS).any(|r| r[6] > 0.0 || r[7] > 0.0))
                        {
                            continue;
                        }
                        let sb = self.buffer("toad sprites", bytemuck::cast_slice(inst), wgpu::BufferUsages::STORAGE);
                        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("toad sprites"),
                            layout: &self.sprites.1,
                            entries: &[
                                wgpu::BindGroupEntry { binding: 0, resource: sb.as_entire_binding() },
                                wgpu::BindGroupEntry { binding: 1, resource: vb.as_entire_binding() },
                                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view) },
                                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                            ],
                        });
                        let pipe = if screen.is_some() { &self.sprites_ui } else { &self.sprites.0 };
                        out.push((pipe, bg, 6, (inst.len() / SPRITE_FLOATS) as u32));
                    }
                }
                Part::Shapes(..) => {}
            }
        }
        out
    }

    fn light_group(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("toad light"),
            layout: &self.light_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// A pass over a whole target with `pipe` and these bindings.
    fn full_pass(
        &self,
        enc: &mut wgpu::CommandEncoder,
        label: &str,
        pipe: &Pipe,
        target: &wgpu::TextureView,
        bindings: &[wgpu::BindingResource<'_>],
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = bindings
            .iter()
            .enumerate()
            .map(|(i, r)| wgpu::BindGroupEntry { binding: i as u32, resource: r.clone() })
            .collect();
        let bg = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipe.1,
            entries: &entries,
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipe.0);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draw a frame. A `live` one goes to that name's textures and is shown
    /// by `gui` (an image node with the source `("live", name)`), queued
    /// and not waited for; any other is made, waited for, and its pixels
    /// answered when `read`.
    pub fn frame(&mut self, v: &View, parts: &[Part<'_>], read: bool, live: Option<&str>) -> Option<Vec<u8>> {
        let (w, h) = v.size;
        self.frame_no += 1;
        let t = match live {
            Some(name) => {
                if self.targets.get(name).is_none_or(|t| t.size != (w, h) || t.gi_down != v.gi_down) {
                    let t = self.make_targets(w, h, v.gi_down);
                    self.targets.insert(name.to_string(), t);
                }
                self.targets.remove(name).expect("made")
            }
            None => self.make_targets(w, h, v.gi_down),
        };
        // every sprite's texture first (uploads need `&mut self`)
        let mut sprite_views = Vec::new();
        for p in parts {
            if let Part::Sprites(runs, _) = p {
                for (pic, _) in runs {
                    let view = self.picture(pic);
                    sprite_views.push(view);
                }
            }
        }
        let lit = v.gi > 0.0;
        let tv = |t: &wgpu::Texture| t.create_view(&Default::default());
        let view_words = |size: (u32, u32), scale: f32, mode: f32, pxk: f32| -> [f32; 16] {
            [
                size.0 as f32, size.1 as f32, v.center.0, v.center.1, scale, v.rot, mode, pxk,
                v.ambient[0], v.ambient[1], v.ambient[2], 0.0, v.gi, if lit { 1.0 } else { 0.0 }, v.time, 0.0,
            ]
        };
        let mut enc = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("toad frame"),
        });
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        // 1–3. the light: its scene, its distance field, the cascades
        let light_view = if lit {
            let down = v.gi_down as f32;
            let gsize = (w.div_ceil(v.gi_down), h.div_ceil(v.gi_down));
            let gv = self.uniform("toad light view", &view_words(gsize, v.scale / down, 1.0, 1.0 / down));
            let gdraws = self.draws(parts, &gv, None, &sprite_views, true);
            let white = self.light_group(&self.white);
            {
                let scene_v = tv(&t.gi_scene);
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("toad light scene"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &scene_v,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: clear, store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                for (pipe, bg, verts, n) in &gdraws {
                    pass.set_pipeline(pipe);
                    pass.set_bind_group(0, bg, &[]);
                    pass.set_bind_group(1, &white, &[]);
                    pass.draw(0..*verts, 0..*n);
                }
            }
            // jump flooding: seed, then halving steps
            let scene_v = tv(&t.gi_scene);
            let seeds = [tv(&t.seeds[0]), tv(&t.seeds[1])];
            let u = self.uniform("toad jfa", &[0.0, 1.0, 0.0, 0.0]);
            self.full_pass(
                &mut enc,
                "toad jfa seed",
                &self.jfa,
                &seeds[0],
                &[wgpu::BindingResource::TextureView(&scene_v), u.as_entire_binding()],
                clear,
            );
            let mut at = 0;
            let mut step = (gsize.0.max(gsize.1) / 2).next_power_of_two() as f32;
            while step >= 1.0 {
                let u = self.uniform("toad jfa", &[step, 0.0, 0.0, 0.0]);
                self.full_pass(
                    &mut enc,
                    "toad jfa",
                    &self.jfa,
                    &seeds[1 - at],
                    &[wgpu::BindingResource::TextureView(&seeds[at]), u.as_entire_binding()],
                    clear,
                );
                at = 1 - at;
                step /= 2.0;
            }
            let dist_v = tv(&t.dist);
            self.full_pass(
                &mut enc,
                "toad dist",
                &self.dist,
                &dist_v,
                &[wgpu::BindingResource::TextureView(&seeds[at])],
                clear,
            );
            // the cascades, far to near, each over the one above
            let cas = [tv(&t.cascades[0]), tv(&t.cascades[1])];
            let mut upper = 1;
            for n in (0..CASCADES).rev() {
                let target = 1 - upper;
                let u = self.uniform("toad cascade", &[n as f32, CASCADES as f32, 1.0, 0.0]);
                self.full_pass(
                    &mut enc,
                    "toad cascade",
                    &self.cascade,
                    &cas[target],
                    &[
                        wgpu::BindingResource::TextureView(&dist_v),
                        wgpu::BindingResource::TextureView(&scene_v),
                        wgpu::BindingResource::TextureView(&cas[upper]),
                        u.as_entire_binding(),
                    ],
                    clear,
                );
                upper = target;
            }
            // swarms' light: a pool around each point, added (no shadow)
            let pv = self.uniform("toad pools", &view_words(gsize, v.scale / down, 2.0, 1.0 / down));
            let pools: Vec<_> = self
                .draws(parts, &pv, None, &sprite_views, true)
                .into_iter()
                .filter(|d| std::ptr::eq(d.0, &self.points.0))
                .collect();
            if !pools.is_empty() {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("toad pools"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &cas[upper],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                for (pipe, bg, verts, n) in &pools {
                    pass.set_pipeline(pipe);
                    pass.set_bind_group(0, bg, &[]);
                    pass.set_bind_group(1, &white, &[]);
                    pass.draw(0..*verts, 0..*n);
                }
            }
            cas[upper].clone()
        } else {
            self.white.clone()
        };
        // 4. the frame, lit
        let hv = tv(&t.hdr);
        let vbuf = self.uniform("toad view", &view_words((w, h), v.scale, 0.0, 1.0));
        let draws = self.draws(parts, &vbuf, None, &sprite_views, false);
        let light = self.light_group(&light_view);
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toad draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: clear, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            for (pipe, bg, verts, n) in &draws {
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                pass.set_bind_group(1, &light, &[]);
                pass.draw(0..*verts, 0..*n);
            }
        }
        // 5. bloom: halvings down, a tent back up into each larger
        let blooms: Vec<wgpu::TextureView> = t.bloom.iter().map(tv).collect();
        if v.bloom > 0.0 && !blooms.is_empty() {
            let smp = wgpu::BindingResource::Sampler(&self.sampler);
            for i in 0..blooms.len() {
                let src = if i == 0 { &hv } else { &blooms[i - 1] };
                self.full_pass(
                    &mut enc,
                    "toad bloom down",
                    &self.bloom_down,
                    &blooms[i],
                    &[wgpu::BindingResource::TextureView(src), smp.clone()],
                    clear,
                );
            }
            for i in (1..blooms.len()).rev() {
                self.full_pass(
                    &mut enc,
                    "toad bloom up",
                    &self.bloom_up,
                    &blooms[i - 1],
                    &[wgpu::BindingResource::TextureView(&blooms[i]), smp.clone()],
                    wgpu::LoadOp::Load,
                );
            }
        }
        // 6. the finish
        let ov = tv(&t.out);
        let tu = self.uniform(
            "toad tone",
            &[
                v.bg[0], v.bg[1], v.bg[2], 1.0, v.tint[0], v.tint[1], v.tint[2], 1.0, v.exposure, v.vignette,
                if blooms.is_empty() { 0.0 } else { v.bloom }, v.contrast, v.saturation, v.grain,
                (self.frame_no % 997) as f32, 0.0,
            ],
        );
        let bloom_src = blooms.first().cloned().unwrap_or_else(|| self.white.clone());
        self.full_pass(
            &mut enc,
            "toad tone",
            &self.tone,
            &ov,
            &[
                wgpu::BindingResource::TextureView(&hv),
                tu.as_entire_binding(),
                wgpu::BindingResource::TextureView(&bloom_src),
                wgpu::BindingResource::Sampler(&self.sampler),
            ],
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        // 7. the screen: points from the top left (y turned up by the
        // caller), no camera, no light, drawn over the finished frame as
        // written
        let k = v.ui_scale.max(0.01);
        let sbuf = self.uniform(
            "toad screen",
            &[
                w as f32, h as f32, w as f32 / (2.0 * k), -(h as f32) / (2.0 * k), k, 0.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, v.time, 0.0,
            ],
        );
        let ui = self.draws(parts, &vbuf, Some(&sbuf), &sprite_views, false);
        if !ui.is_empty() {
            let white = self.light_group(&self.white);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toad screen"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ov,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            for (pipe, bg, verts, n) in &ui {
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                pass.set_bind_group(1, &white, &[]);
                pass.draw(0..*verts, 0..*n);
            }
        }
        let row = (w * 4).div_ceil(256) * 256;
        let d = &self.gpu.device;
        let rb = read.then(|| {
            let rb = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("toad frame read"),
                size: (row * h) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.out,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &rb,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            rb
        });
        self.gpu.queue.submit([enc.finish()]);
        if let Some(name) = live {
            crate::stdlib::gui::live::publish(name, ov, w, h);
            self.targets.insert(name.to_string(), t);
        }
        let Some(rb) = rb else {
            if live.is_none() {
                let _ = d.poll(wgpu::PollType::wait_indefinitely());
            }
            return None;
        };
        let slice = rb.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = d.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().ok()?;
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let at = (y * row) as usize;
            px.extend_from_slice(&data[at..at + (w * 4) as usize]);
        }
        Some(px)
    }
}
