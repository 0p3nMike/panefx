//! Writing frames to an H.264 .mp4 with Media Foundation's sink writer.
//!
//! Windows' own encoder rather than ffmpeg: nothing to install, nothing to
//! ship to a friend's machine, and the sink writer picks the GPU's hardware
//! encoder when there is one (`MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS`). It
//! also converts our BGRX frames to the encoder's own format by itself.

use std::path::Path;

use windows::core::{GUID, HSTRING};
use windows::Win32::Media::MediaFoundation::*;

/// Bits per pixel per frame. Glyph art is sharp edges on flat colour, which
/// needs more than film (~0.1) to stay crisp; at 2400x1600x30 this is ~17
/// Mbit/s, ~40 MB for a 20 s loop. Measured on a 2400x1600 laptop 2026-10-23.
const BITS_PER_PIXEL: f64 = 0.15;

/// 100-nanosecond units, Media Foundation's clock.
const HNS: i64 = 10_000_000;

pub struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
    fps: u32,
    frame: i64,
}

/// Media Foundation packs two u32s into one u64 attribute (frame size, rate).
fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

impl Encoder {
    /// `width` and `height` must be even: H.264 works in 2x2 chroma blocks.
    pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let e = |what: &str| {
            let what = what.to_string();
            move |err: windows::core::Error| format!("{what}: {err}")
        };
        unsafe {
            super::media_foundation()?;

            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 3).map_err(e("MFCreateAttributes"))?;
            let attrs = attrs.ok_or("MFCreateAttributes gave nothing")?;
            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(e("hardware transforms"))?;
            // Not a live source: let it take frames as fast as we make them.
            attrs
                .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)
                .map_err(e("throttling"))?;
            // Say MP4 outright, because the file is written under a temporary
            // name whose extension the writer would otherwise guess from.
            attrs
                .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
                .map_err(e("container"))?;

            let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None::<&IMFByteStream>, &attrs)
                .map_err(e("MFCreateSinkWriterFromURL"))?;

            let video_type = |subtype: &GUID| -> Result<IMFMediaType, String> {
                let t = MFCreateMediaType().map_err(e("MFCreateMediaType"))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(e("major type"))?;
                t.SetGUID(&MF_MT_SUBTYPE, subtype).map_err(e("subtype"))?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                    .map_err(e("interlace"))?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height)).map_err(e("frame size"))?;
                t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1)).map_err(e("frame rate"))?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)).map_err(e("aspect"))?;
                Ok(t)
            };

            let out = video_type(&MFVideoFormat_H264)?;
            let bitrate = (width as f64 * height as f64 * fps as f64 * BITS_PER_PIXEL) as u32;
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate).map_err(e("bitrate"))?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .map_err(e("profile"))?;
            let stream = writer.AddStream(&out).map_err(e("AddStream"))?;

            let input = video_type(&MFVideoFormat_RGB32)?;
            // POSITIVE stride = top-down rows, which is what the renderer's
            // DIB section holds. Negative would mean bottom-up and an
            // upside-down video.
            input
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)
                .map_err(e("stride"))?;
            writer
                .SetInputMediaType(stream, &input, None::<&IMFAttributes>)
                .map_err(e("SetInputMediaType"))?;
            writer.BeginWriting().map_err(e("BeginWriting"))?;

            Ok(Encoder {
                writer,
                stream,
                width,
                height,
                fps,
                frame: 0,
            })
        }
    }

    /// Append one frame: top-down BGRX rows, `stride` bytes apart.
    pub fn write(&mut self, pixels: &[u8], stride: usize) -> Result<(), String> {
        let row = self.width as usize * 4;
        let len = row * self.height as usize;
        if stride < row || pixels.len() < stride * (self.height as usize - 1) + row {
            return Err("frame smaller than the video".into());
        }
        let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
        unsafe {
            let buffer = MFCreateMemoryBuffer(len as u32).map_err(e("MFCreateMemoryBuffer"))?;
            let mut dst: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut dst, None, None).map_err(e("Lock"))?;
            for y in 0..self.height as usize {
                std::ptr::copy_nonoverlapping(pixels.as_ptr().add(y * stride), dst.add(y * row), row);
            }
            buffer.Unlock().map_err(e("Unlock"))?;
            buffer.SetCurrentLength(len as u32).map_err(e("SetCurrentLength"))?;

            let sample = MFCreateSample().map_err(e("MFCreateSample"))?;
            sample.AddBuffer(&buffer).map_err(e("AddBuffer"))?;
            // Exact frame times from the frame NUMBER, never accumulated, so
            // the loop length is exactly frames / fps.
            let t = self.frame * HNS / self.fps as i64;
            let next = (self.frame + 1) * HNS / self.fps as i64;
            sample.SetSampleTime(t).map_err(e("SetSampleTime"))?;
            sample.SetSampleDuration(next - t).map_err(e("SetSampleDuration"))?;
            self.writer.WriteSample(self.stream, &sample).map_err(e("WriteSample"))?;
        }
        self.frame += 1;
        Ok(())
    }

    pub fn frames(&self) -> i64 {
        self.frame
    }

    /// Flush and close the file. Without this the .mp4 has no index and will
    /// not play.
    pub fn finish(self) -> Result<(), String> {
        unsafe { self.writer.Finalize() }.map_err(|e| format!("Finalize: {e}"))
    }
}
