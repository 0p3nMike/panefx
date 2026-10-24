//! Baked loops: the wallpaper played back from a recorded video instead of
//! drawn live.
//!
//! # Why
//!
//! Drawing the wallpaper live is GDI text calls for every lit cell, every
//! frame, on the CPU -- the renderer is this program's hot path (see
//! `render.rs`). Most of the time nothing about the animation needs to be live:
//! it is the same effect, in the same colours, at the same size, forever. So it
//! is recorded ONCE into a seamless loop and played back by the GPU's video
//! decoder, which costs almost nothing.
//!
//! # The rule
//!
//! * `visualizer` OFF (the default): play the baked loop. While a loop does
//!   not exist yet it is recorded in the background, at low priority, and the
//!   wallpaper draws live until it is ready.
//! * `visualizer` ON: draw live, every frame -- the mode a music-reactive
//!   wallpaper needs. Nothing switches this automatically; the switch decides.
//! * A baked loop is NOT paused when windows cover the desktop. Playing it
//!   costs next to nothing, and the point is to keep it moving.
//!
//! # What makes a loop
//!
//! A loop is keyed by everything that decides its pixels -- see [`Recipe`]. The
//! colours are baked in, so a theme change, a tweaked param or a different
//! `wallpaper_detail` (the glyph size) is a different loop. Switching back to
//! a look that was baked before plays its loop again at once, which is what
//! makes "several glyph sizes per animation" free: each size you pick is baked
//! once and kept.
//!
//! The pieces:
//!
//! * [`Recipe`] -- what to record, and the id (file name) it records to;
//! * [`bake`] -- records a recipe deterministically (fixed time step, no wall
//!   clock), seamlessly, through the same renderer the live wallpaper uses;
//! * [`encode`] -- Windows' own H.264 encoder (Media Foundation), so there is
//!   no ffmpeg to install;
//! * [`player`] -- Windows' own media engine, decoding on the wallpaper's D3D11
//!   device straight into its swapchain;
//! * [`Baker`] -- the background thread that runs bakes one at a time;
//! * [`deskfx`] -- loops recorded by another program (deskfx's animations),
//!   which panefx only plays.

pub mod bake;
pub mod deskfx;
pub mod encode;
pub mod player;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::animation::AsciiAnimation;
use crate::config::Config;

/// Bumped whenever the recording itself changes (timing, crossfade, encoder
/// settings), so loops recorded the old way are not mistaken for current ones.
pub const FORMAT: u32 = 2;

/// Everything that decides a wallpaper's pixels, and therefore one loop.
#[derive(Clone)]
pub struct Recipe {
    /// The config the surface was built from. Carries the font, the frame
    /// rates and every effect param, theme colours included.
    pub cfg: Config,
    /// `DISPLAY<n>`: params resolve per monitor, and so does the seed.
    pub monitor: usize,
    /// Effects, bottom layer first.
    pub stack: Vec<String>,
    pub cols: usize,
    pub rows: usize,
    /// Glyph cell, in pixels.
    pub cell: (i32, i32),
    /// Frame size, in pixels: the monitor's.
    pub size: (i32, i32),
    /// Playback rate: the wallpaper's own.
    pub fps: u64,
}

impl Recipe {
    /// Build the layer stack exactly as the live wallpaper does.
    pub fn build_layers(&self) -> Vec<Box<dyn AsciiAnimation>> {
        self.stack
            .iter()
            .map(|e| crate::wallpaper::build_sim(&self.cfg, e, self.cols, self.rows, self.monitor))
            .collect()
    }

    /// The canonical description of this loop. Every line is something that
    /// changes the pixels; two recipes with the same text record the same
    /// video.
    ///
    /// Params are read back from freshly built layers rather than from the
    /// config, so defaults, clamping and theme colours all count exactly as
    /// the effect itself sees them.
    pub fn describe(&self) -> String {
        let c = &self.cfg;
        let mut s = format!(
            "panefx baked loop, format {FORMAT}\n\
             size {}x{} cell {}x{} grid {}x{}\n\
             fps {} tick_fps {} seed {:#x}\n\
             font {:?} chars {:?}\n",
            self.size.0,
            self.size.1,
            self.cell.0,
            self.cell.1,
            self.cols,
            self.rows,
            self.fps,
            c.fps,
            crate::wallpaper::seed_for(self.monitor),
            c.font,
            c.chars_override,
        );
        for (i, layer) in self.build_layers().iter().enumerate() {
            let mut params: Vec<(String, String)> = layer
                .params()
                .into_iter()
                .map(|p| (p.key, p.value.display()))
                .collect();
            params.sort();
            s.push_str(&format!("layer {i} {}:", layer.name()));
            for (k, v) in params {
                s.push_str(&format!(" {k}={v}"));
            }
            s.push('\n');
        }
        s
    }

    /// The loop's name, which is also its file name:
    /// `EFFECT@CELLWxCELLH-WIDTHxHEIGHT-HASH`, e.g.
    /// `flames@14x21-2400x1600-2f2979a5e2636cc5`.
    ///
    /// The readable part is the cache layout -- animation, glyph size,
    /// resolution -- so a listing of `dir()` says what is there. The hash is
    /// FNV-1a over [`describe`] and covers everything else (theme colours,
    /// params, fps, font); it is stable across builds and Rust versions, which
    /// `std`'s hasher is not promised to be.
    ///
    /// The resolution is part of the name on purpose: a monitor that changes
    /// mode (or a second monitor of another size) gets its own loop recorded
    /// at its own size, never an old one stretched.
    pub fn id(&self) -> String {
        format!(
            "{}@{}x{}-{}x{}-{:016x}",
            self.stack.join("+"),
            self.cell.0,
            self.cell.1,
            self.size.0,
            self.size.1,
            fnv1a(self.describe().as_bytes())
        )
    }

    pub fn path(&self) -> PathBuf {
        dir().join(format!("{}.mp4", self.id()))
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Where loops live: `%LOCALAPPDATA%\panefx\loops`. A cache, not config --
/// deleting the folder only costs re-recording.
pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("panefx")
        .join("loops")
}

/// A finished bake, as reported back to the daemon.
pub struct Done {
    pub id: String,
    pub result: Result<bake::Report, String>,
}

/// Runs bakes on ONE background thread, one at a time, at background priority.
///
/// One thread, not one per monitor: a bake is a burst of rendering, and
/// recording four screens at once would be four bursts on a machine someone is
/// using. They queue instead.
pub struct Baker {
    tx: Option<Sender<Recipe>>,
    rx: Option<Receiver<Done>>,
    /// Submitted and not yet finished.
    pending: HashSet<String>,
    /// Failed this session: not retried until the daemon restarts, so a recipe
    /// the encoder cannot handle is not re-recorded forever.
    failed: HashSet<String>,
}

impl Default for Baker {
    fn default() -> Self {
        Self::new()
    }
}

impl Baker {
    pub fn new() -> Self {
        Baker {
            tx: None,
            rx: None,
            pending: HashSet::new(),
            failed: HashSet::new(),
        }
    }

    /// Ask for a recipe to be recorded. A no-op when it is already on disk,
    /// already queued, or failed earlier.
    pub fn want(&mut self, recipe: Recipe) {
        let id = recipe.id();
        if self.pending.contains(&id) || self.failed.contains(&id) || recipe.path().exists() {
            return;
        }
        if self.tx.is_none() {
            self.start();
        }
        if let Some(tx) = &self.tx {
            if tx.send(recipe).is_ok() {
                self.pending.insert(id);
            }
        }
    }

    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains(id)
    }

    pub fn has_failed(&self, id: &str) -> bool {
        self.failed.contains(id)
    }

    /// Bakes finished since the last call.
    pub fn finished(&mut self) -> Vec<Done> {
        let Some(rx) = &self.rx else {
            return Vec::new();
        };
        let done: Vec<Done> = rx.try_iter().collect();
        for d in &done {
            self.pending.remove(&d.id);
            if d.result.is_err() {
                self.failed.insert(d.id.clone());
            }
        }
        done
    }

    fn start(&mut self) {
        let (job_tx, job_rx) = channel::<Recipe>();
        let (done_tx, done_rx) = channel::<Done>();
        let spawned = std::thread::Builder::new()
            .name("panefx-baker".into())
            .spawn(move || {
                background_priority();
                for recipe in job_rx {
                    let id = recipe.id();
                    let result = bake::bake(&recipe);
                    if done_tx.send(Done { id, result }).is_err() {
                        return;
                    }
                }
            });
        match spawned {
            Ok(_) => {
                self.tx = Some(job_tx);
                self.rx = Some(done_rx);
            }
            Err(e) => crate::log_warn!("[panefx] loops: could not start the baker: {e}"),
        }
    }
}

/// Lower this thread's CPU AND I/O priority, so a bake yields to everything
/// the user is doing. `THREAD_MODE_BACKGROUND_BEGIN` is the Windows way to say
/// "only when nothing else wants the machine".
fn background_priority() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

/// Start Media Foundation (and COM, which it needs) on the calling thread.
///
/// `MFStartup` is reference counted and cheap to repeat; it is never paired
/// with `MFShutdown` because the daemon uses it until it exits.
pub(crate) fn media_foundation() -> Result<(), String> {
    unsafe {
        use windows::Win32::Media::MediaFoundation::{MFStartup, MFSTARTUP_LITE, MF_VERSION};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        // Already initialised in another mode is fine: Media Foundation's
        // objects are free-threaded and do not care which apartment they are
        // created in.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).map_err(|e| format!("MFStartup failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_the_published_function() {
        // Reference values for FNV-1a 64.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
