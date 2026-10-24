//! Measure how a baked loop wraps from its end back to its start.
//!
//!     loop_wrap FILE.mp4 SECONDS OUT.txt
//!
//! Plays FILE through the wallpaper's own player (`loops::player`) without
//! showing it, driving its looping at the daemon's pace (every 33 ms), polls
//! the playback position every millisecond for SECONDS, and writes to OUT.txt
//! how long the position stood still around each wrap against the longest
//! pause anywhere else. A seamless wrap stands still no longer than any other
//! moment of playback.
//!
//! A windowless program: the media engine only presents in the desktop
//! session, so it is run there (a scheduled task) and must not open a console.

#![windows_subsystem = "windows"]

use std::io::Write;
use std::time::{Duration, Instant};

macro_rules! say {
    ($out:expr, $($t:tt)*) => {{
        let _ = writeln!($out, $($t)*);
        let _ = $out.flush();
    }};
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        std::process::exit(2);
    }
    let path = &args[1];
    let secs: f64 = args[2].parse().unwrap_or(30.0);
    let Ok(mut out) = std::fs::File::create(&args[3]) else {
        std::process::exit(2);
    };
    let mut player = match panefx::loops::player::Player::open(std::path::Path::new(path), 640, 360) {
        Ok(p) => p,
        Err(e) => {
            say!(out, "loop_wrap: {e}");
            std::process::exit(1);
        }
    };
    let start = Instant::now();
    while !(player.duration() > 0.0) {
        let _ = player.advance();
        if start.elapsed() > Duration::from_secs(10) || player.failed() {
            say!(out, "loop_wrap: the file did not load");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let dur = player.duration();
    say!(out, "{path}: {dur:.3} s");

    let t0 = Instant::now();
    let mut last_tick = t0;
    let mut last_pos = player.position();
    let mut last_move = t0;
    let mut elsewhere = Duration::ZERO;
    let mut at_wrap: Vec<f64> = Vec::new();
    while t0.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_millis(1));
        let now = Instant::now();
        if now - last_tick >= Duration::from_millis(33) {
            last_tick = now;
            if let Err(e) = player.advance() {
                say!(out, "advance: {e}");
            }
        }
        let pos = player.position();
        if pos == last_pos {
            continue;
        }
        let still = now - last_move;
        // A stall that ends at the wrap, or just after it (the new start
        // standing still), belongs to the wrap.
        if pos < last_pos || last_pos < 0.15 || dur - last_pos < 0.15 {
            if pos < last_pos || at_wrap.is_empty() {
                at_wrap.push(0.0);
            }
            let w = at_wrap.last_mut().unwrap();
            *w = w.max(still.as_secs_f64() * 1000.0);
        } else {
            elsewhere = elsewhere.max(still);
        }
        last_pos = pos;
        last_move = now;
    }
    say!(
        out,
        "wraps {}; longest stall away from the wrap {:.1} ms; at each wrap: {:?} ms; failed {}",
        player.wraps(),
        elsewhere.as_secs_f64() * 1000.0,
        at_wrap.iter().map(|w| (w * 10.0).round() / 10.0).collect::<Vec<_>>(),
        player.failed()
    );
}
