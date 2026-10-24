//! Playing a baked loop on the GPU, with no per-frame work on our side.
//!
//! Media Foundation's media engine runs in "windowless swapchain" mode: it
//! decodes on the GPU's video decoder (the D3D11 device is handed over through
//! a DXGI device manager), presents into a composition swapchain of its own,
//! keeps the time and loops by itself. We put that swapchain on the
//! wallpaper's composition visual once (`Surface::show_external`) and are done.
//!
//! # Why not "frame server" mode
//!
//! The obvious design asks the engine each frame for a new picture and copies
//! it into our own swapchain (`OnVideoStreamTick` + `TransferVideoFrame` +
//! `Present`). It works, and it was measured first: on a laptop's Intel
//! iGPU that copy-and-present path cost ~10% of a core on our thread, most of
//! it inside the graphics driver, on top of the decoder's own ~10%. Letting
//! the engine present for itself removes our share entirely.
//!
//! # Looping without a hitch
//!
//! The engine's own loop (`SetLoop`) wraps by seeking back to the start, which
//! flushes and restarts the decoder: measured on a desktop GPU, the clock stood still
//! for ~50 ms at every wrap (2 ms anywhere else) -- a visible jolt once per
//! loop. So a player holds TWO engines on the same file. One plays; the other
//! waits paused on the first frame, already decoded. When the playing one
//! reaches its end, the waiting one's swapchain goes on screen and starts
//! playing from a frame that is already there, and the first one is rewound
//! (out of sight) to wait for the next wrap. The cost is a second, idle
//! decoder per screen: memory, no CPU while paused.
//!
//! # When the end is
//!
//! The engine hands its swapchain frames ahead of time, each to be shown at
//! its own moment, and reports the end (the ENDED event, the position jumping
//! to the duration) once the last one has been handed over -- up to ~10
//! frames before it is on screen. Handing over at that report cut those
//! frames off: measured on a desktop with four screens, every wrap skipped
//! 4-13 frames, a visible jump in anything that turns (donut, groovy). So
//! the player keeps its own clock of where the end is, from the position
//! while the engine is still playing, and hands over at that moment.
//!
//! # Known: the first handover stalls
//!
//! Once per open, at the first wrap, the second engine presents nothing for
//! ~250 ms after Play() (its rendered-frame count sits at 1 while its clock
//! runs), so the screen holds frame 0 and then jumps to frame 7-8. Every
//! later handover, in either direction, is clean. Measured on 25 restarts.
//! None of these changed it, each tried 2-3 times: playing the second engine
//! out of sight first (1 s, one tick, or into the end of the file) and
//! rewinding it; rewinding it from Load(); showing its swapchain first,
//! paused or playing; rewinding it while still on screen; making the first
//! wrap a plain rewind (the stall then moved to the first handover, at the
//! second wrap). Accepted for now: one short hitch after each start.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, BSTR};
use windows::Win32::Foundation::{BOOL, HANDLE, RECT};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::compositor::{ExternalContent, Surface};

/// The engine's callback. All it does is notice errors (so the wallpaper can
/// fall back to drawing live instead of freezing) and the end of the file (so
/// the other engine can take over).
#[implement(IMFMediaEngineNotify)]
struct Notify {
    failed: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.failed.store(true, Ordering::Relaxed);
        } else if event == MF_MEDIA_ENGINE_EVENT_ENDED.0 as u32 {
            self.ended.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

thread_local! {
    /// One DXGI device manager per D3D11 device, shared by every monitor's
    /// player. Keyed by the device pointer, so a rebuilt device (after a
    /// driver reset) gets a fresh manager.
    static MANAGER: std::cell::RefCell<Option<(usize, IMFDXGIDeviceManager)>> =
        const { std::cell::RefCell::new(None) };
}

fn manager_for(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager, String> {
    let key = device.as_raw() as usize;
    MANAGER.with(|m| {
        if let Some((k, mgr)) = m.borrow().as_ref() {
            if *k == key {
                return Ok(mgr.clone());
            }
        }
        unsafe {
            let mut token = 0u32;
            let mut mgr: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut token, &mut mgr)
                .map_err(|e| format!("MFCreateDXGIDeviceManager: {e}"))?;
            let mgr = mgr.ok_or("MFCreateDXGIDeviceManager gave nothing")?;
            mgr.ResetDevice(device, token)
                .map_err(|e| format!("ResetDevice (does the device have video support?): {e}"))?;
            *m.borrow_mut() = Some((key, mgr.clone()));
            Ok(mgr)
        }
    })
}

/// One media engine on the loop file.
struct Engine {
    engine: IMFMediaEngineEx,
    failed: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    /// Its swapchain once it has one, sized to the surface.
    handle: Option<HANDLE>,
}

impl Engine {
    /// Load `path`, muted and not looping; playing at once if `play`, else
    /// waiting paused on the first frame.
    fn open(path: &Path, manager: &IMFDXGIDeviceManager, play: bool) -> Result<Self, String> {
        let failed = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let notify: IMFMediaEngineNotify = Notify {
            failed: failed.clone(),
            ended: ended.clone(),
        }
        .into();
        let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
        unsafe {
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 3).map_err(e("MFCreateAttributes"))?;
            let attrs = attrs.ok_or("MFCreateAttributes gave nothing")?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(e("callback"))?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, manager).map_err(e("device manager"))?;
            attrs
                .SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)
                .map_err(e("output format"))?;

            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(e("media engine factory"))?;
            let engine: IMFMediaEngineEx = factory
                .CreateInstance(0, &attrs)
                .map_err(e("media engine"))?
                .cast()
                .map_err(e("IMFMediaEngineEx"))?;
            // Before SetSource: the engine decides how to present while it
            // loads the file.
            engine
                .EnableWindowlessSwapchainMode(BOOL::from(true))
                .map_err(e("EnableWindowlessSwapchainMode"))?;
            engine.SetMuted(BOOL::from(true)).map_err(e("SetMuted"))?;
            engine.SetLoop(BOOL::from(false)).map_err(e("SetLoop"))?;
            engine.SetAutoPlay(BOOL::from(play)).map_err(e("SetAutoPlay"))?;
            engine.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC).map_err(e("SetPreload"))?;
            engine
                .SetSource(&BSTR::from(path.to_string_lossy().as_ref()))
                .map_err(e("SetSource"))?;
            if play {
                engine.Play().map_err(e("Play"))?;
            } else {
                // Without a load the engine waits for Play() before decoding
                // anything; this gets the first frame decoded and presented
                // now, while nobody is looking.
                engine.Load().map_err(e("Load"))?;
            }
            Ok(Engine {
                engine,
                failed,
                ended,
                handle: None,
            })
        }
    }

    /// Its swapchain, once the file is loaded, sized to `width` x `height`.
    fn swapchain(&mut self, width: i32, height: i32) -> Result<Option<HANDLE>, String> {
        if self.handle.is_some() {
            return Ok(self.handle);
        }
        unsafe {
            // Fails, or hands back a null handle, until the media is loaded.
            let Ok(handle) = self.engine.GetVideoSwapchainHandle() else {
                return Ok(None);
            };
            if handle.is_invalid() {
                return Ok(None);
            }
            let dst = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let black = MFARGB {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbAlpha: 255,
            };
            self.engine
                .UpdateVideoStream(None, Some(&dst), Some(&black))
                .map_err(|e| format!("UpdateVideoStream: {e}"))?;
            self.handle = Some(handle);
        }
        Ok(self.handle)
    }

    /// Paused on the first frame, decoded and not seeking: ready to take over.
    fn ready(&self) -> bool {
        unsafe {
            self.engine.GetReadyState() >= MF_MEDIA_ENGINE_READY_HAVE_CURRENT_DATA.0 as u16
                && !self.engine.IsSeeking().as_bool()
        }
    }

    fn position(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }

    fn duration(&self) -> f64 {
        unsafe { self.engine.GetDuration() }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Shutdown releases the decoder and the file NOW; without it they live
        // until the engine's last reference goes, which the callback can hold.
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

pub struct Player {
    /// Two engines on the same file; `active` is the one on screen.
    engines: [Engine; 2],
    active: usize,
    width: i32,
    height: i32,
    /// When `advance` last ran, and the gap before that: the next call is
    /// expected about one gap later, so an end nearer to now than to that
    /// call is handed over now.
    last_advance: Option<Instant>,
    gap: Duration,
    /// When the last frame of the engine on screen ends, from its position
    /// while it plays (not from its end report, which comes early).
    end_at: Option<Instant>,
    /// Handovers so far.
    wraps: u64,
    /// The swapchain on screen, once it is. Dropping it puts the wallpaper's
    /// own swapchain back. Declared after `engines` only for reading order:
    /// the engines shut down first either way.
    shown: Option<ExternalContent>,
}

impl Player {
    /// Open `path` and start it playing, looped and muted, to fill a
    /// `width` x `height` surface.
    pub fn open(path: &Path, width: i32, height: i32) -> Result<Self, String> {
        super::media_foundation()?;
        let device = crate::compositor::shared_device()?;
        let manager = manager_for(&device)?;
        let first = Engine::open(path, &manager, true)?;
        let second = Engine::open(path, &manager, false)?;
        Ok(Player {
            engines: [first, second],
            active: 0,
            width,
            height,
            last_advance: None,
            gap: Duration::from_millis(33),
            end_at: None,
            wraps: 0,
            shown: None,
        })
    }

    /// Did either engine report an error? The caller should drop the player
    /// and draw live.
    pub fn failed(&self) -> bool {
        self.engines.iter().any(|e| e.failed.load(Ordering::Relaxed))
    }

    /// Is the loop on screen yet?
    pub fn showing(&self) -> bool {
        self.shown.is_some()
    }

    /// Seconds into the loop, of the engine on screen.
    pub fn position(&self) -> f64 {
        self.engines[self.active].position()
    }

    /// The loop's length in seconds (NaN until it is loaded).
    pub fn duration(&self) -> f64 {
        self.engines[self.active].duration()
    }

    /// How many times the loop has wrapped.
    pub fn wraps(&self) -> u64 {
        self.wraps
    }

    /// The looping, apart from the screen: when the engine on screen is at its
    /// end, or will be before the next call, start the waiting one and rewind
    /// this one. Returns the swapchain to show when the engines swapped.
    ///
    /// Called every daemon frame by `tick`; public so the wrap can be measured
    /// without a desktop surface (examples/loop_wrap.rs).
    pub fn advance(&mut self) -> Result<Option<HANDLE>, String> {
        let now = Instant::now();
        if let Some(prev) = self.last_advance {
            self.gap = (now - prev).min(Duration::from_millis(250));
        }
        self.last_advance = Some(now);
        let (w, h) = (self.width, self.height);
        for e in self.engines.iter_mut() {
            e.swapchain(w, h)?;
        }
        let cur = &self.engines[self.active];
        let ended = cur.ended.load(Ordering::Relaxed);
        let left = cur.duration() - cur.position();
        if !ended && left.is_finite() && left > 0.0 {
            self.end_at = Some(now + Duration::from_secs_f64(left));
        }
        let at_end = match self.end_at {
            Some(end) => end <= now + self.gap / 2,
            None => ended,
        };
        if !at_end {
            return Ok(None);
        }
        let next = 1 - self.active;
        let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
        unsafe {
            if !self.engines[next].ready() || self.engines[next].handle.is_none() {
                // The other one is not there yet: wrap this one the plain way
                // rather than stop at its end.
                let cur = &self.engines[self.active];
                if cur.ended.swap(false, Ordering::Relaxed) {
                    cur.engine.SetCurrentTime(0.0).map_err(e("SetCurrentTime"))?;
                    cur.engine.Play().map_err(e("Play"))?;
                    self.end_at = None;
                    self.wraps += 1;
                }
                return Ok(None);
            }
            self.engines[next].engine.Play().map_err(e("Play"))?;
            let old = &self.engines[self.active];
            old.engine.Pause().map_err(e("Pause"))?;
            old.engine.SetCurrentTime(0.0).map_err(e("SetCurrentTime"))?;
            old.ended.store(false, Ordering::Relaxed);
        }
        self.active = next;
        self.end_at = None;
        self.wraps += 1;
        Ok(self.engines[next].handle)
    }

    /// Put the loop on `surface` as soon as its swapchain exists -- a few
    /// frames after opening, once the file is loaded -- and keep it looping.
    pub fn tick(&mut self, surface: &Surface) -> Result<(), String> {
        let swapped = self.advance()?;
        if let Some(shown) = self.shown.as_mut() {
            if let Some(handle) = swapped {
                shown.replace(handle)?;
            }
            return Ok(());
        }
        if let Some(handle) = self.engines[self.active].handle {
            self.shown = Some(surface.show_external(handle)?);
        }
        Ok(())
    }
}
