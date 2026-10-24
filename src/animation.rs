//! The animation abstraction.
//!
//! A panel does not know it is showing fire. It owns something that implements
//! [`AsciiAnimation`], asks it to advance one frame, and asks it for a glyph +
//! colour per cell. Adding a new effect means writing one more implementor —
//! no change to the panel, the renderer, or the supervisor.
//!
//! Note for anyone coming from a class-based language: this is Rust, so there
//! is no subclassing. Effects are separate types that each `impl
//! AsciiAnimation`, held behind `Box<dyn AsciiAnimation>`. The dispatch is
//! dynamic, which is what makes the swap possible at runtime, but the
//! relationship is "implements an interface", not "inherits from a base
//! class".

use crate::palette::Rgb;
use serde::{Deserialize, Serialize};

/// A live-tunable value belonging to one effect.
///
/// Deliberately NOT ascii-flavoured: a future GIF or shader effect would expose
/// `speed`/`loop`/`path` through the same mechanism — ascii is one subsystem
/// of panefx, not its definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParamValue {
    Int { v: i64 },
    Text { v: String },
    Colour { r: u8, g: u8, b: u8 },
}

impl ParamValue {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            ParamValue::Int { v } => Some(*v),
            _ => None,
        }
    }
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ParamValue::Text { v } => Some(v),
            _ => None,
        }
    }
    pub fn as_rgb(&self) -> Option<Rgb> {
        match self {
            ParamValue::Colour { r, g, b } => Some(Rgb(*r, *g, *b)),
            _ => None,
        }
    }
    pub fn from_rgb(c: Rgb) -> Self {
        ParamValue::Colour {
            r: c.0,
            g: c.1,
            b: c.2,
        }
    }
    /// Render for a TUI row.
    pub fn display(&self) -> String {
        match self {
            ParamValue::Int { v } => v.to_string(),
            ParamValue::Text { v } => v.clone(),
            ParamValue::Colour { r, g, b } => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

/// One tunable parameter, as declared by the effect that owns it.
///
/// The TUI renders whatever it is handed — it must never hardcode which knobs
/// belong to which effect, so a new effect's controls appear for free.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub key: String,
    pub label: String,
    pub value: ParamValue,
    /// Inclusive bounds for `Int` params; ignored otherwise.
    pub min: i64,
    pub max: i64,
}

impl Param {
    pub fn int(key: &str, label: &str, v: i64, min: i64, max: i64) -> Self {
        Param {
            key: key.into(),
            label: label.into(),
            value: ParamValue::Int { v },
            min,
            max,
        }
    }
    pub fn text(key: &str, label: &str, v: &str) -> Self {
        Param {
            key: key.into(),
            label: label.into(),
            value: ParamValue::Text { v: v.into() },
            min: 0,
            max: 0,
        }
    }
    pub fn colour(key: &str, label: &str, c: Rgb) -> Self {
        Param {
            key: key.into(),
            label: label.into(),
            value: ParamValue::from_rgb(c),
            min: 0,
            max: 0,
        }
    }
}

/// Clamp helper for implementors — a TUI or a bad config must not be able to
/// push a value outside the range the effect declared.
pub fn clamp_int(v: i64, min: i64, max: i64) -> i64 {
    v.max(min).min(max)
}

/// One animated ASCII effect, rendered into a grid of character cells.
///
/// Implementors own their own simulation state and palette. The renderer only
/// ever asks for a glyph and a colour per cell, so an effect is free to use
/// any internal representation — a heat field, a set of falling columns, a
/// noise function.
pub trait AsciiAnimation {
    /// Human-readable name, used for logging and for selecting the effect by
    /// configuration.
    fn name(&self) -> &'static str;

    /// Resize the simulation grid. Called whenever the panel's cell dimensions
    /// change (window resized, font/DPI changed).
    ///
    /// Implementors may discard state on resize; callers must not rely on
    /// content being preserved.
    fn resize(&mut self, cols: usize, rows: usize);

    /// Current grid size in cells, as `(cols, rows)`.
    fn dimensions(&self) -> (usize, usize);

    /// Advance the simulation by one frame.
    fn step(&mut self);

    /// The character cell size this effect wants, in pixels.
    ///
    /// `None` (the default) means "use the configured size" — that is what
    /// `rain`, `flames` and `fire` do, because they are designed to sit on the
    /// terminal's own text grid.
    ///
    /// `waves` overrides it: the blackwaves field wants chunky cells so the
    /// glyphs read as marks rather than as text, and it looks wrong at the
    /// terminal's 10x15. Making this per-effect means one effect can be chunky
    /// without dragging the others with it.
    ///
    /// An explicit `PANEFX_CELL_W`/`_H` (or a `cell_w`/`cell_h` set live from
    /// the TUI) still wins — a user asking for a size means it.
    fn preferred_cell(&self) -> Option<(i32, i32)> {
        None
    }

    /// Did the last `step()` actually change what `cell_at` will return?
    ///
    /// Defaults to `true` (always redraw), which is always correct — an effect
    /// that does not implement this simply gets redrawn every frame as before.
    ///
    /// Effects that advance on their own clock can do much better: `rain` ticks
    /// every 55ms regardless of the panel's fps, so at 20fps roughly one frame
    /// in ten produces an identical grid. Reporting that lets the supervisor
    /// skip the glyph loop AND the BitBlt for every panel — and profiling shows
    /// the renderer, not the simulation, is where the time goes.
    fn changed(&self) -> bool {
        true
    }

    /// The glyph and colour to draw at a cell, in ONE call.
    ///
    /// Returning `None` means "draw nothing here" — the renderer skips the
    /// cell entirely rather than drawing a space. This matters: these panels
    /// sit behind a semi-transparent terminal, so an unnecessary painted cell
    /// is a visible smudge, not a no-op.
    ///
    /// Glyph and colour are returned together deliberately. Splitting them
    /// into `glyph_at` + `color_at` doubles the per-cell work, because an
    /// effect usually derives both from the same computed value — for the
    /// fire that is a dithered ramp index, and computing it twice per cell
    /// measurably raised CPU (12.7% -> 22.5% of a core when it was split).
    fn cell_at(&self, col: usize, row: usize) -> Option<(char, Rgb)>;

    /// Background the panel is cleared to before drawing.
    fn background(&self) -> Rgb;

    /// The effect's live-tunable parameters, current values included.
    ///
    /// Default: none. `fire` uses the default and simply shows no knobs.
    fn params(&self) -> Vec<Param> {
        Vec::new()
    }

    /// Apply one parameter. Returns whether it was recognised and applied, so
    /// the caller can report an unknown key instead of silently doing nothing.
    ///
    /// Implementors MUST clamp to the bounds they declared in `params()`.
    fn set_param(&mut self, _key: &str, _v: &ParamValue) -> bool {
        false
    }

    /// Seconds of animation after which the frames repeat EXACTLY, if they do.
    ///
    /// Only the baked-loop recorder (`loops::bake`) asks. An effect that
    /// answers can be recorded for exactly one period and the loop is seamless
    /// by construction; one that does not (rain, fire: anything driven by
    /// randomness) is recorded longer and crossfaded at the seam instead.
    ///
    /// Measured in the effect's own clock, which runs in real seconds (every
    /// periodic effect scales its step by the frame time), so at `fps` frames a
    /// second the loop is `period * fps` frames long.
    fn period_secs(&self) -> Option<f64> {
        None
    }

    /// Jump straight to `phase` (0..1) of that period, as if stepped there.
    ///
    /// Recording by seeking rather than stepping is what makes the loop exact:
    /// frame `i` of `n` is phase `i / n`, with no accumulated rounding, so the
    /// last frame leads into the first exactly as any frame leads into the
    /// next. Only called when `period_secs` answered.
    fn seek(&mut self, _phase: f64) {}
}

/// The common period of two rotations, in seconds, given each one's rate in
/// MILLI-radians per second -- or `None` when they never line up again soon.
///
/// A turn at `r` mrad/s repeats every `TAU * 1000 / r` seconds; two turns
/// repeat together every `TAU * 1000 / gcd(r1, r2)`. A zero rate is a turn that
/// never moves, which every period satisfies.
pub fn common_turn_period(rates_milli: &[i64]) -> Option<f64> {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = rates_milli
        .iter()
        .map(|r| r.unsigned_abs())
        .filter(|r| *r != 0)
        .fold(0u64, gcd);
    (g != 0).then(|| std::f64::consts::TAU * 1000.0 / g as f64)
}

/// Every effect that can be named in the rotation.
///
/// **To add a new background:** write a module implementing `AsciiAnimation`,
/// add its name here and a line in `build`. Nothing else in the program needs
/// to change — the supervisor, panel and renderer are all trait-only.
pub const EFFECTS: &[&str] = &[
    "flames",
    "rain",
    "waves",
    "fire",
    "plasma",
    "tunnel",
    "starfield",
    // One module, four surfaces -- see `spin3d::Shape`. Named separately here
    // because "pick an effect" is the user-facing choice; that they share a
    // renderer is an implementation detail.
    "donut",
    "sphere",
    "cube",
    "galaxy",
    // The michael.slop mascot, spinning ghostty-style.
    "skullspin",
];

/// Can `name` be a monitor's wallpaper effect? Everything in [`EFFECTS`],
/// plus the deskfx loops (`deskfx:NAME`, see `loops::deskfx`), which exist
/// only on the desktop. `off` is checked separately by the callers.
pub fn is_wallpaper_effect(name: &str) -> bool {
    EFFECTS.contains(&name) || crate::loops::deskfx::name_of(name).is_some()
}

/// Construct an effect by name. Unknown names fall back to the first entry in
/// `EFFECTS` rather than failing, so a typo in a config file degrades to a
/// working backdrop instead of a blank screen.
pub fn build(
    name: &str,
    cols: usize,
    rows: usize,
    seed: u64,
    cfg: &crate::config::Config,
) -> Box<dyn AsciiAnimation> {
    match name.trim().to_lowercase().as_str() {
        // A deskfx loop's stand-in: the plain background until its video plays.
        n if crate::loops::deskfx::name_of(n).is_some() => Box::new(crate::loops::deskfx::Backdrop::new(
            cols,
            rows,
            crate::loops::deskfx::background_of(cfg),
        )),
        "rain" => {
            let mut r = crate::rain::Rain::new(cols, rows, seed, cfg.chars_override.as_deref());
            r.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(r)
        }
        "waves" => {
            let mut w = crate::waves::Waves::new(cols, rows, cfg.chars_override.as_deref());
            w.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(w)
        }
        "fire" => Box::new(crate::fire::Fire::new(cols, rows, seed)),
        "skullspin" => {
            let mut s = crate::skullspin::SkullSpin::new(cols, rows);
            s.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(s)
        }
        "plasma" => {
            let mut p = crate::plasma::Plasma::new(cols, rows, cfg.chars_override.as_deref());
            p.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(p)
        }
        "tunnel" => {
            let mut t = crate::tunnel::Tunnel::new(cols, rows, cfg.chars_override.as_deref());
            t.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(t)
        }
        "starfield" => {
            let mut s = crate::starfield::Starfield::new(cols, rows, seed);
            s.set_frame_ms(cfg.frame_time().as_millis() as u64);
            Box::new(s)
        }
        "donut" | "sphere" | "cube" | "galaxy" => {
            let mut s = crate::spin3d::Spin3d::new(cols, rows);
            s.set_frame_ms(cfg.frame_time().as_millis() as u64);
            s.set_shape(match name.trim().to_lowercase().as_str() {
                "sphere" => crate::spin3d::Shape::Sphere,
                "cube" => crate::spin3d::Shape::Cube,
                "galaxy" => crate::spin3d::Shape::Galaxy,
                _ => crate::spin3d::Shape::Donut,
            });
            Box::new(s)
        }
        _ => {
            let mut f = crate::flames::Flames::new(cols, rows, seed);
            // Same shape as rain/waves above: the rate is a construction-time
            // fact the effect cannot read for itself, and flames needs it so
            // `osc_secs` counts seconds rather than frames.
            f.set_fps(cfg.fps as f32);
            Box::new(f)
        }
    }
}

/// Which saved-parameter namespace a simulation draws from.
///
/// The same effect runs in two places with different jobs: behind a terminal it
/// has to stay legible under 60%-opaque text, on the desktop it has a whole
/// screen to itself. They keep separate saved values, and this says which set to
/// read.
///
/// Deliberately NOT defaulted anywhere — every call site must state its scope,
/// so adding a new one cannot silently inherit the wrong parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The backdrop behind terminal windows.
    Pane,
    /// The desktop wallpaper, for ONE monitor.
    ///
    /// Carries the `DISPLAY<n>` index because wallpaper params resolve
    /// per-monitor: the shared `[wallpaper.<effect>]` block with that monitor's
    /// `[wallpaper.<n>.<effect>]` overrides laid on top. Without the index every
    /// screen running an effect read the same values, so tuning one tuned all
    /// of them.
    Wallpaper(usize),
}

/// Apply any `[effect]` params persisted in config.toml to a freshly built
/// effect. Values are stored as strings, so each is tried as an int, then a
/// colour, then plain text — whichever the effect accepts.
///
/// Lives here rather than in `main.rs` because both the terminal supervisor and
/// the wallpaper's simulation pool need it, and neither should own it.
pub fn apply_saved_params(
    sim: &mut dyn AsciiAnimation,
    cfg: &crate::config::Config,
    scope: Scope,
) {
    let name = sim.name().to_lowercase();
    // No saved entry means this effect has never been tuned in this scope, so
    // the effect's own constructor defaults stand. That is how the desktop
    // starts from the effect's defaults rather than a copy of the pane's.
    let merged;
    let saved = match scope {
        Scope::Pane => match cfg.effect_params.get(&name) {
            Some(m) => m,
            None => return,
        },
        Scope::Wallpaper(monitor) => {
            merged = cfg.wallpaper_params_for(monitor, &name);
            if merged.is_empty() {
                return;
            }
            &merged
        }
    };
    // Collect first: `params()` borrows, `set_param` needs &mut.
    let saved: Vec<(String, String)> = saved.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, raw) in saved {
        let applied = if let Ok(n) = raw.parse::<i64>() {
            sim.set_param(&k, &ParamValue::Int { v: n })
        } else if let Some(c) = crate::palette::Rgb::parse_hex(&raw) {
            sim.set_param(&k, &ParamValue::from_rgb(c))
        } else {
            sim.set_param(&k, &ParamValue::Text { v: raw.clone() })
        };
        if !applied {
            // Name the SCOPE too: "[waves]" and "[wallpaper.waves]" are
            // different sections and the message must say which one is wrong.
            let sec = match scope {
                Scope::Pane => name.clone(),
                Scope::Wallpaper(m) => format!("wallpaper.{m}.{name}"),
            };
            eprintln!("[panefx] config [{sec}]: effect ignored '{k}'");
        }
    }
}

/// Build an effect and restore its persisted params.
///
/// Used wherever a change cannot be applied in place — an effect switch, or a
/// value captured at construction such as rain's frame_ms.
pub fn rebuild(
    cfg: &crate::config::Config,
    name: &str,
    cols: usize,
    rows: usize,
    seed: u64,
    scope: Scope,
) -> Box<dyn AsciiAnimation> {
    let mut s = build(name, cols, rows, seed, cfg);
    apply_saved_params(s.as_mut(), cfg, scope);
    s
}

#[cfg(test)]
mod name_tests {
    use super::*;

    /// Saved params and theme colours are looked up by `name()`, and written
    /// under the effect's `EFFECTS` name: the two must be the same word, or
    /// that effect never wears a theme.
    #[test]
    fn every_effect_is_named_what_it_is_listed_as() {
        let cfg = crate::config::Config::default();
        for e in EFFECTS {
            assert_eq!(build(e, 8, 8, 1, &cfg).name(), *e);
        }
    }

    #[test]
    fn a_theme_reaches_the_donut_on_the_desktop() {
        let mut cfg = crate::config::Config::default();
        cfg.wallpaper_effect_params
            .entry("donut".into())
            .or_default()
            .insert("hi".into(), "#123456".into());
        let sim = rebuild(&cfg, "donut", 8, 8, 1, Scope::Wallpaper(1));
        let hi = sim.params().into_iter().find(|p| p.key == "hi").unwrap();
        assert_eq!(hi.value.display(), "#123456");
    }
}
