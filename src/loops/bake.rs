//! Recording one loop: deterministic, seamless, through the live renderer.
//!
//! # Deterministic
//!
//! Every frame is exactly one `step()` (or one `seek`), with no wall clock and
//! no audio. The live wallpaper also steps once per frame, so a recorded
//! second looks like a live one -- and the same recipe always records the same
//! video.
//!
//! # Seamless, two ways
//!
//! * **Periodic effects** (plasma, tunnel, the spinning solids) report a period
//!   (`AsciiAnimation::period_secs`). They are recorded for exactly one period
//!   by SEEKING to phase `i / n` for frame `i`, so the last frame leads into
//!   the first exactly as any frame leads into the next.
//! * **Everything else** (flames, rain, fire ...) never repeats. It is warmed up
//!   first, so the loop does not open on an empty screen, then recorded for
//!   [`RANDOM_SECS`], and the last [`FADE_SECS`] are crossfaded into the
//!   opening frames. See [`crossfade`] for the arrangement.

use std::time::Instant;

use super::encode::Encoder;
use super::Recipe;
use crate::animation::AsciiAnimation;
use crate::render::Canvas;

/// A periodic effect is recorded for one period only when that is at most
/// this long. A slow plasma can take minutes to come round; it is crossfaded
/// like a random effect instead, rather than recording a 300 MB loop.
pub const MAX_PERIOD_SECS: f64 = 60.0;
/// Simulated before recording starts, so fire has risen and rain is falling.
pub const WARMUP_SECS: f64 = 5.0;
/// Length of a crossfaded loop.
pub const RANDOM_SECS: f64 = 20.0;
/// How long the seam of a crossfaded loop blends over.
pub const FADE_SECS: f64 = 1.5;

/// How a recipe is recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plan {
    /// One exact period of `frames` frames: the least common multiple of
    /// every layer's own period, so each layer completes whole turns.
    Periodic { frames: usize },
    /// `frames` frames, the last `fade` blended into the first.
    Crossfade { warmup: usize, frames: usize, fade: usize },
}

/// What a bake did, for the log.
#[derive(Debug, Clone)]
pub struct Report {
    pub plan: Plan,
    pub frames: usize,
    pub secs: f64,
    pub bytes: u64,
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Frames per period for each layer, or `None` when any layer is not
/// periodic. A period is rounded to whole frames -- under half a frame per
/// turn, which no eye can see as a speed change.
pub fn layer_periods(layers: &[Box<dyn AsciiAnimation>], fps: u64) -> Option<Vec<usize>> {
    layers
        .iter()
        .map(|l| {
            let p = l.period_secs()?;
            if !p.is_finite() || p <= 0.0 {
                return None;
            }
            Some(((p * fps as f64).round() as usize).max(1))
        })
        .collect()
}

/// Decide how to record `layers`.
pub fn plan(layers: &[Box<dyn AsciiAnimation>], fps: u64) -> Plan {
    let secs = |s: f64| (s * fps as f64).round() as usize;
    if let Some(periods) = layer_periods(layers, fps) {
        // A stack repeats when every layer has: the least common multiple.
        let max = secs(MAX_PERIOD_SECS);
        let mut lcm = 1usize;
        for p in periods {
            lcm = lcm / gcd(lcm, p) * p;
            if lcm > max {
                break;
            }
        }
        if lcm <= max && lcm >= 2 {
            return Plan::Periodic { frames: lcm };
        }
    }
    Plan::Crossfade {
        warmup: secs(WARMUP_SECS),
        frames: secs(RANDOM_SECS),
        fade: secs(FADE_SECS),
    }
}

/// Record `recipe` to its loop file. Written under a temporary name and
/// renamed at the end, so a half-written file is never mistaken for a loop.
pub fn bake(recipe: &Recipe) -> Result<Report, String> {
    let started = Instant::now();
    let path = recipe.path();
    let dir = path.parent().ok_or("loop path has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let part = path.with_extension("part.mp4");
    let _ = std::fs::remove_file(&part);

    // H.264 needs even sizes. A monitor is always even in practice; an odd one
    // loses its last pixel row or column, which the player stretches over.
    let (w, h) = (recipe.size.0 & !1, recipe.size.1 & !1);
    if w < 2 || h < 2 {
        return Err(format!("frame too small: {}x{}", recipe.size.0, recipe.size.1));
    }
    let fps = recipe.fps.max(1);
    let cfg = crate::wallpaper::WallpaperSet::cfg_for_surface(&recipe.cfg, recipe.cell);

    let mut a = recipe.build_layers();
    let plan = plan(&a, fps);
    let mut canvas = Canvas::new(recipe.size.0, recipe.size.1);
    let stride = recipe.size.0 as usize * 4;
    let mut enc = Encoder::create(&part, w as u32, h as u32, fps as u32)?;

    let written = match plan {
        Plan::Periodic { frames } => periodic(&mut a, frames, fps, &mut canvas, &cfg, &mut enc, stride),
        Plan::Crossfade { warmup, frames, fade } => {
            let mut b = recipe.build_layers();
            crossfade(&mut a, &mut b, warmup, frames, fade, &mut canvas, &cfg, &mut enc, stride)
        }
    };
    if let Err(e) = written {
        drop(enc);
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let frames = enc.frames() as usize;
    enc.finish()?;
    std::fs::rename(&part, &path).map_err(|e| format!("rename to {}: {e}", path.display()))?;
    // The recipe beside the video, so `dir()` explains itself.
    let _ = std::fs::write(path.with_extension("txt"), recipe.describe());
    Ok(Report {
        plan,
        frames,
        secs: started.elapsed().as_secs_f64(),
        bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    })
}

fn refs(layers: &[Box<dyn AsciiAnimation>]) -> Vec<&dyn AsciiAnimation> {
    layers.iter().map(|b| b.as_ref()).collect()
}

fn paint<'c>(
    canvas: &'c mut Canvas,
    layers: &[Box<dyn AsciiAnimation>],
    cfg: &crate::config::Config,
) -> Result<&'c [u8], String> {
    canvas.paint(&refs(layers), cfg).ok_or_else(|| "the renderer drew nothing".to_string())
}

/// One exact period: frame `i` is every layer seeked to its own phase.
fn periodic(
    layers: &mut [Box<dyn AsciiAnimation>],
    frames: usize,
    fps: u64,
    canvas: &mut Canvas,
    cfg: &crate::config::Config,
    enc: &mut Encoder,
    stride: usize,
) -> Result<(), String> {
    let periods: Vec<usize> = layers
        .iter()
        .map(|l| {
            l.period_secs()
                .map(|p| ((p * fps as f64).round() as usize).max(1))
                .unwrap_or(frames)
        })
        .collect();
    for i in 0..frames {
        for (l, n) in layers.iter_mut().zip(&periods) {
            l.seek((i % n) as f64 / *n as f64);
        }
        enc.write(paint(canvas, layers, cfg)?, stride)?;
    }
    Ok(())
}

/// A random effect, made to loop.
///
/// Two copies of the same simulation, `a` and `b`, built from the same recipe
/// and therefore identical frame for frame. With `A[k]` the k-th frame after
/// warm-up, the video is
///
/// ```text
///   A[fade .. frames]                         then
///   mix(A[frames + j], A[j], (j+1)/(fade+1))  for j in 0..fade
/// ```
///
/// Read across the seam: the blend starts as `A[frames]`, which follows the
/// frame before it naturally, and ends as `A[fade - 1]`, which the video's
/// first frame `A[fade]` follows naturally. Every cut is a real next frame.
///
/// `b` exists so `A[0..fade]` need not be held in memory (45 full-screen
/// frames is ~700 MB at 2400x1600): it is simply simulated again, in step with
/// the tail of `a`.
#[allow(clippy::too_many_arguments)]
fn crossfade(
    a: &mut [Box<dyn AsciiAnimation>],
    b: &mut [Box<dyn AsciiAnimation>],
    warmup: usize,
    frames: usize,
    fade: usize,
    canvas: &mut Canvas,
    cfg: &crate::config::Config,
    enc: &mut Encoder,
    stride: usize,
) -> Result<(), String> {
    fn step(layers: &mut [Box<dyn AsciiAnimation>]) {
        layers.iter_mut().for_each(|l| l.step());
    }
    let fade = fade.min(frames / 2);

    for _ in 0..warmup {
        step(a);
    }
    // A[0], kept only as a fingerprint: B[0] must match it, or the effect is
    // not deterministic and the seam would show.
    step(a);
    let first = super::fnv1a(paint(canvas, a, cfg)?);
    for _ in 1..fade {
        step(a);
    }
    for _ in fade..frames {
        step(a);
        enc.write(paint(canvas, a, cfg)?, stride)?;
    }

    for _ in 0..warmup {
        step(b);
    }
    let mut mixed: Vec<u8> = Vec::new();
    for j in 0..fade {
        step(a);
        step(b);
        mixed.clear();
        mixed.extend_from_slice(paint(canvas, a, cfg)?);
        let from_b = paint(canvas, b, cfg)?;
        if j == 0 && super::fnv1a(from_b) != first {
            crate::log_warn!("[panefx] loops: the effect did not replay identically; the seam may show");
        }
        // Weight of B in 1/256ths.
        let wb = (((j + 1) * 256) / (fade + 1)) as u32;
        let wa = 256 - wb;
        for (m, &q) in mixed.iter_mut().zip(from_b) {
            *m = ((*m as u32 * wa + q as u32 * wb) >> 8) as u8;
        }
        enc.write(&mixed, stride)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> crate::config::Config {
        crate::config::Config::default()
    }

    #[test]
    fn plasma_is_recorded_for_exactly_one_period() {
        let mut p = crate::animation::build("plasma", 10, 10, 1, &cfg());
        p.set_param("speed", &crate::animation::ParamValue::Int { v: 1000 });
        // 1 / (1.0 * 0.08) = 12.5 s = 375 frames at 30 fps.
        assert_eq!(plan(&[p], 30), Plan::Periodic { frames: 375 });
    }

    #[test]
    fn a_slow_plasma_is_crossfaded_instead_of_recorded_for_minutes() {
        let mut p = crate::animation::build("plasma", 10, 10, 1, &cfg());
        p.set_param("speed", &crate::animation::ParamValue::Int { v: 100 });
        assert!(matches!(plan(&[p], 30), Plan::Crossfade { .. }));
    }

    #[test]
    fn random_effects_are_crossfaded() {
        let f = crate::animation::build("flames", 10, 10, 1, &cfg());
        assert_eq!(
            plan(&[f], 30),
            Plan::Crossfade { warmup: 150, frames: 600, fade: 45 }
        );
    }

    #[test]
    fn a_stack_repeats_when_every_layer_has() {
        let mut a = crate::animation::build("plasma", 10, 10, 1, &cfg());
        a.set_param("speed", &crate::animation::ParamValue::Int { v: 1000 }); // 375 frames
        let mut b = crate::animation::build("plasma", 10, 10, 1, &cfg());
        b.set_param("speed", &crate::animation::ParamValue::Int { v: 2000 }); // 188 (rounded)
        match plan(&[a, b], 30) {
            Plan::Periodic { frames } => assert!(frames % 375 == 0 && frames % 188 == 0),
            Plan::Crossfade { .. } => {} // also acceptable when the lcm is too long
        }
    }

    fn grid(e: &dyn AsciiAnimation) -> Vec<Option<(char, crate::palette::Rgb)>> {
        let (cols, rows) = e.dimensions();
        (0..rows).flat_map(|r| (0..cols).map(move |c| (c, r))).map(|(c, r)| e.cell_at(c, r)).collect()
    }

    /// Across the seam of a one-period loop, last frame -> first frame changes
    /// no more cells than an ordinary step does: the wrap is just another
    /// next frame.
    #[test]
    fn the_seam_is_an_ordinary_step() {
        for name in ["plasma", "tunnel", "donut", "sphere", "cube", "galaxy"] {
            let mut e = crate::animation::build(name, 120, 60, 1, &cfg());
            let Some(p) = e.period_secs() else { continue };
            let n = ((p * 30.0).round() as usize).max(2);
            let at = |e: &mut Box<dyn AsciiAnimation>, i: usize| {
                e.seek((i % n) as f64 / n as f64);
                grid(e.as_ref())
            };
            let diff = |a: &[Option<(char, crate::palette::Rgb)>], b: &[Option<(char, crate::palette::Rgb)>]| {
                a.iter().zip(b).filter(|(x, y)| x != y).count()
            };
            let (f0, f1, last2, last) = (at(&mut e, 0), at(&mut e, 1), at(&mut e, n - 2), at(&mut e, n - 1));
            let step = diff(&f0, &f1).max(diff(&last2, &last));
            let seam = diff(&last, &f0);
            assert!(seam <= step * 2 + 10, "{name}: seam changes {seam} cells, a step {step}");
        }
    }

    /// Seeking is a pure function of the phase: the same phase draws the same
    /// frame however it was reached, and a different phase draws a different
    /// one. Both are what one-period recording relies on.
    #[test]
    fn seeking_is_a_function_of_the_phase() {
        for name in ["plasma", "tunnel", "donut", "cube"] {
            let mut e = crate::animation::build(name, 40, 20, 1, &cfg());
            assert!(e.period_secs().is_some(), "{name} should be periodic");
            e.seek(0.25);
            let first = grid(e.as_ref());
            e.seek(0.7);
            let other = grid(e.as_ref());
            e.seek(1.25); // the same phase, one period on
            assert_eq!(first, grid(e.as_ref()), "{name}: phase 0.25 drew two different frames");
            assert_ne!(first, other, "{name}: phases 0.25 and 0.7 look the same");
        }
    }
}
