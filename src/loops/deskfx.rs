//! deskfx loops: wallpaper animations that exist only as baked videos.
//!
//! deskfx is a separate ASCII animation engine (Go). Its animations are not
//! reimplemented here: they are recorded elsewhere, one video per theme,
//! animation, detail level and screen size, and panefx only PLAYS them, through
//! the same media-engine player as its own baked loops (`player`).
//!
//! # Naming
//!
//! A wallpaper effect named `deskfx:NAME` (e.g. `deskfx:torus`) plays deskfx's
//! NAME. The prefix keeps them apart from panefx's own effects, several of which
//! share a name (`rain`, `plasma`, `flames`, ...). They are wallpaper-only: the
//! terminal backdrop draws live and has no use for them, so they are NOT in
//! `animation::EFFECTS`.
//!
//! # The loop folder
//!
//! ```text
//! %LOCALAPPDATA%\panefx\loops\deskfx\
//! ├── screens                       one line per screen: "WIDTH HEIGHT SCALE"
//! │                                 (pixels, and the screen's DPI scale), written here
//! └── WIDTHxHEIGHT\THEME\NAME@PCT.mp4      the loop
//!                       NAME@PCT.mp4.grid  "COLS ROWS" of its glyph grid
//! ```
//!
//! Inside one resolution folder the layout is the recorder's own
//! (`loops/THEME/NAME@PCT.mp4`), so a recorder pointed at that folder needs no
//! knowledge of panefx. `screens` tells the recorder which sizes to record.
//!
//! * THEME = the config's `theme` (the house theme when empty).
//! * PCT   = the glyph size in percent of the animation's design size, one of
//!   [`SCALES`], picked by `wallpaper_detail` (1 = largest glyphs = least
//!   detail, 10 = smallest = most).
//!
//! # No loop yet
//!
//! A surface whose loop is missing shows the plain theme background
//! ([`Backdrop`]) and checks for the file every few seconds, so a loop that is
//! recorded later starts playing without a restart.

use std::path::{Path, PathBuf};

use crate::animation::AsciiAnimation;
use crate::config::Config;
use crate::palette::Rgb;

/// The prefix that marks a wallpaper effect as a deskfx loop.
pub const PREFIX: &str = "deskfx:";

/// Every animation the recorder offers, in its menu order. `flames` is the
/// recorder's own fire; deskfx's flames effect is `amtui-flames`.
pub const NAMES: &[&str] = &[
    "flames",
    "rain",
    "torus",
    "torus-rt",
    "sphere",
    "arch",
    "groovy",
    "blackwaves",
    "amtui-flames",
    "fire",
    "waves",
    "plasma",
    "plasma-square",
    "tunnel",
    "starfield",
    "cube",
];

/// The detail levels: glyph size in percent of the design size, least detail
/// first. `wallpaper_detail` N picks `SCALES[N - 1]`.
pub const SCALES: [u32; 10] = [200, 150, 125, 100, 85, 70, 60, 50, 40, 33];

/// `deskfx:torus` -> `Some("torus")`; anything else (including an unknown
/// deskfx name) -> `None`.
pub fn name_of(effect: &str) -> Option<&'static str> {
    let rest = effect.trim().strip_prefix(PREFIX)?;
    NAMES.iter().copied().find(|n| n.eq_ignore_ascii_case(rest))
}

/// Every deskfx wallpaper effect name, prefixed.
pub fn effect_names() -> Vec<String> {
    NAMES.iter().map(|n| format!("{PREFIX}{n}")).collect()
}

/// The glyph size, in percent, for a `wallpaper_detail` level (clamped 1..=10).
pub fn percent_for(detail: u8) -> u32 {
    SCALES[(detail.clamp(1, 10) - 1) as usize]
}

/// The theme folder a config's loops live in: its `theme`, or the house theme.
pub fn theme_of(cfg: &Config) -> String {
    let t = cfg.theme.trim().to_lowercase();
    if t.is_empty() {
        crate::themes::catalog::HOUSE_ID.to_string()
    } else {
        t
    }
}

/// `%LOCALAPPDATA%\panefx\loops\deskfx`.
pub fn dir() -> PathBuf {
    super::dir().join("deskfx")
}

/// Where the loop for NAME at PCT in THEME, at a WIDTH x HEIGHT screen, lives
/// under `root` (normally [`dir`]).
pub fn loop_path_in(root: &Path, theme: &str, name: &str, percent: u32, size: (i32, i32)) -> PathBuf {
    root.join(format!("{}x{}", size.0, size.1))
        .join(theme)
        .join(format!("{name}@{percent}.mp4"))
}

/// The loop a surface of `size` should play for `name` under `cfg`.
pub fn loop_path(cfg: &Config, name: &str, size: (i32, i32)) -> PathBuf {
    loop_path_in(&dir(), &theme_of(cfg), name, percent_for(cfg.wallpaper_detail), size)
}

/// A short, readable id for a loop, for the log and the snapshot:
/// `1920x1080/nord/torus@85`.
pub fn loop_id(cfg: &Config, name: &str, size: (i32, i32)) -> String {
    format!(
        "{}x{}/{}/{}@{}",
        size.0,
        size.1,
        theme_of(cfg),
        name,
        percent_for(cfg.wallpaper_detail)
    )
}

/// The `screens` file's text for these screens: one sorted, de-duplicated line
/// per distinct `(width, height, scale)`, scale rounded to hundredths.
pub fn screens_text(screens: &[(i32, i32, f64)]) -> String {
    let mut lines: Vec<String> = screens
        .iter()
        .filter(|(w, h, _)| *w > 0 && *h > 0)
        .map(|(w, h, s)| format!("{w} {h} {}", (s * 100.0).round() / 100.0))
        .collect();
    lines.sort();
    lines.dedup();
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Write `screens` under `root`, only when its content changes (it is read by
/// another program, and rewriting an unchanged file on every monitor poll
/// would be noise).
pub fn write_screens_in(root: &Path, screens: &[(i32, i32, f64)]) -> std::io::Result<bool> {
    let text = screens_text(screens);
    let path = root.join("screens");
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return Ok(false);
    }
    std::fs::create_dir_all(root)?;
    let tmp = root.join("screens.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// One line about each animation, for the picker.
pub fn about(name: &str) -> &'static str {
    match name {
        "flames" => "the desktop fire (house) — the fire along the bottom edge",
        "rain" => "panefx's ASCII rain; ignores the music",
        "torus" => "amtui's torus — the ring's thickness is the spectrum",
        "torus-rt" => "a real torus, ray-marched cell by cell: solid at every detail level, no holes, no seam",
        "sphere" => "amtui's sphere — a wireframe globe the kick pumps",
        "arch" => "the Arch logo — a solid A spinning like the torus; the bands climb it, the kick swells it",
        "groovy" => "psychadelic style animated plasma",
        "blackwaves" => "panefx's blackwaves — ASCII water: a dark creased sheet that breathes in place; ignores the music",
        "amtui-flames" => "amtui's flames — the panefx flames, one column per band",
        "fire" => "fire — the song's spectra scrolling up as heat",
        "waves" => "waves — the spectrum as a rolling sea",
        "plasma" => "plasma — interference rings the bands colour",
        "plasma-square" => "plasma (square) — the same, on a square lattice",
        "tunnel" => "tunnel — rings of past spectra flying at you",
        "starfield" => "starfield — stars launched by the bands",
        "cube" => "cube — six faces, six band groups",
        _ => "",
    }
}

/// The distinct `(width, height)` sizes in a `screens` file, in order.
pub fn parse_screens(text: &str) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(w), Some(h)) = (f.next(), f.next()) else {
            continue;
        };
        let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) else {
            continue;
        };
        if w > 0 && h > 0 && !out.contains(&(w, h)) {
            out.push((w, h));
        }
    }
    out
}

/// The screen sizes panefx last wrote to `screens`.
pub fn screens() -> Vec<(i32, i32)> {
    parse_screens(&std::fs::read_to_string(dir().join("screens")).unwrap_or_default())
}

/// Whether one animation at one detail level is baked for the screens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoopStatus {
    /// Screen sizes that have the loop.
    pub have: usize,
    /// Screen sizes that need it.
    pub screens: usize,
    /// The glyph grid of the first loop found (from its `.grid`), 0 x 0 if none.
    pub cols: usize,
    pub rows: usize,
}

impl LoopStatus {
    pub fn baked(&self) -> bool {
        self.screens > 0 && self.have == self.screens
    }
}

pub fn status_in(root: &Path, theme: &str, name: &str, percent: u32, screens: &[(i32, i32)]) -> LoopStatus {
    let mut st = LoopStatus {
        screens: screens.len(),
        ..Default::default()
    };
    for s in screens {
        let p = loop_path_in(root, theme, name, percent, *s);
        if !p.exists() {
            continue;
        }
        st.have += 1;
        if st.cols == 0 {
            let mut grid = PathBuf::from(&p).into_os_string();
            grid.push(".grid");
            let text = std::fs::read_to_string(grid).unwrap_or_default();
            let mut f = text.split_whitespace().map(|x| x.parse::<usize>().unwrap_or(0));
            st.cols = f.next().unwrap_or(0);
            st.rows = f.next().unwrap_or(0);
        }
    }
    st
}

/// A Windows path as WSL mounts it: `C:\Users\x` -> `/mnt/c/Users/x`.
pub fn wsl_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        format!("/mnt/{}{}", s[..1].to_lowercase(), &s[2..])
    } else {
        s
    }
}

/// The recorder's command line (`deskfx-bake`, see the panefx repo's
/// tools/deskfx-bake): name at each percent, in theme, into the loop folder
/// `loops`, with the palettes in `themes` (both WSL paths).
pub fn bake_args(loops: &str, themes: &str, theme: &str, name: &str, percents: &[u32]) -> Vec<String> {
    let mut a = vec!["-o".into(), loops.into(), "-T".into(), themes.into(), theme.into(), name.into()];
    a.extend(percents.iter().map(|p| p.to_string()));
    a
}

/// The theme palettes the recorder reads: `~\.config\omarchy\themes`, the
/// same folder panefx's own theme list overlays.
pub fn themes_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config")
        .join("omarchy")
        .join("themes")
}

/// Start the recorder in WSL for `name` at `percents` in `theme`, detached:
/// its own process group and no window, so it outlives whoever started it.
/// Its output is appended to the loop folder's `bake.log`. Fails when this
/// machine has no WSL, or no `deskfx-bake` on WSL's PATH.
#[cfg(windows)]
pub fn start_bake(theme: &str, name: &str, percents: &[u32]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let quiet = |c: &mut std::process::Command| {
        c.creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    };
    let mut probe = std::process::Command::new("wsl.exe");
    probe.args(["-e", "bash", "-lc", "command -v deskfx-bake"]);
    quiet(&mut probe);
    match probe.status() {
        Ok(s) if s.success() => {}
        _ => return Err("no baker here: deskfx-bake runs in WSL".into()),
    }
    let _ = std::fs::create_dir_all(dir());
    let log = wsl_path(&dir().join("bake.log"));
    let mut args: Vec<String> = vec![
        "-e".into(),
        "bash".into(),
        "-lc".into(),
        r#"log=$1; shift; exec deskfx-bake "$@" >>"$log" 2>&1"#.into(),
        "bash".into(),
        log,
    ];
    args.extend(bake_args(&wsl_path(&dir()), &wsl_path(&themes_dir()), theme, name, percents));
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(&args);
    quiet(&mut cmd);
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    cmd.spawn().map(|_| ()).map_err(|e| format!("could not start wsl.exe: {e}"))
}

#[cfg(not(windows))]
pub fn start_bake(_theme: &str, _name: &str, _percents: &[u32]) -> Result<(), String> {
    Err("no baker here: deskfx-bake runs in WSL".into())
}

/// Ask for one missing loop to be recorded, in the background: once per loop
/// per process (a failed or slow recorder is not started again every time the
/// surface looks), on a thread of its own so the caller never waits for WSL.
pub fn request_bake(theme: &str, name: &str, percent: u32) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static ASKED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let key = format!("{theme}/{name}@{percent}");
    {
        let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if !asked.get_or_insert_with(HashSet::new).insert(key.clone()) {
            return;
        }
    }
    let (theme, name) = (theme.to_string(), name.to_string());
    let _ = std::thread::Builder::new()
        .name("panefx-deskfx-bake".into())
        .spawn(move || match start_bake(&theme, &name, &[percent]) {
            Ok(()) => crate::log_info!("[panefx] loops: recording deskfx {key} in the background"),
            Err(e) => crate::log_info!("[panefx] loops: deskfx {key} not recorded: {e}"),
        });
}

/// The DPI scale of the monitor containing the centre of this rectangle
/// (1.0 = 96 DPI). 1.0 when Windows cannot say.
#[cfg(windows)]
pub fn scale_at(x: i32, y: i32, w: i32, h: i32) -> f64 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let mon = MonitorFromPoint(
            POINT {
                x: x + w / 2,
                y: y + h / 2,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let (mut dx, mut dy) = (0u32, 0u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 {
            return dx as f64 / 96.0;
        }
    }
    1.0
}

#[cfg(not(windows))]
pub fn scale_at(_x: i32, _y: i32, _w: i32, _h: i32) -> f64 {
    1.0
}

/// The colour a deskfx surface shows while it has no loop: the theme
/// background, which the colour applier writes as the flames' `bg` (the
/// desktop's copy first, then the backdrop's). Black when neither has one.
pub fn background_of(cfg: &Config) -> Rgb {
    let from = |m: Option<&std::collections::BTreeMap<String, String>>| {
        m.and_then(|m| m.get("bg")).and_then(|v| Rgb::parse_hex(v))
    };
    from(cfg.wallpaper_effect_params.get("flames"))
        .or_else(|| from(cfg.effect_params.get("flames")))
        .unwrap_or(Rgb(0, 0, 0))
}

/// The stand-in simulation of a deskfx surface: draws no glyphs, only the
/// background, and never changes. It exists so a deskfx surface has the same
/// shape (a panel and a pooled simulation) as every other wallpaper surface;
/// while its loop plays it is never stepped or drawn.
pub struct Backdrop {
    cols: usize,
    rows: usize,
    bg: Rgb,
}

impl Backdrop {
    pub fn new(cols: usize, rows: usize, bg: Rgb) -> Self {
        Backdrop { cols, rows, bg }
    }
}

impl AsciiAnimation for Backdrop {
    fn name(&self) -> &'static str {
        "deskfx"
    }
    fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
    }
    fn dimensions(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }
    fn step(&mut self) {}
    fn changed(&self) -> bool {
        false
    }
    fn cell_at(&self, _col: usize, _row: usize) -> Option<(char, Rgb)> {
        None
    }
    fn background(&self) -> Rgb {
        self.bg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_need_the_prefix_and_a_known_animation() {
        assert_eq!(name_of("deskfx:torus"), Some("torus"));
        assert_eq!(name_of("deskfx:torus-rt"), Some("torus-rt"));
        assert_eq!(name_of("deskfx:amtui-flames"), Some("amtui-flames"));
        assert_eq!(name_of("torus"), None);
        assert_eq!(name_of("plasma"), None);
        assert_eq!(name_of("deskfx:nope"), None);
        assert_eq!(name_of("deskfx:"), None);
    }

    #[test]
    fn sixteen_animations_none_of_them_a_panefx_effect_name_once_prefixed() {
        assert_eq!(NAMES.len(), 16);
        for n in effect_names() {
            assert!(!crate::animation::EFFECTS.contains(&n.as_str()));
            assert!(name_of(&n).is_some(), "{n}");
        }
    }

    #[test]
    fn detail_levels_map_onto_the_glyph_sizes() {
        assert_eq!(percent_for(1), 200);
        assert_eq!(percent_for(4), 100);
        assert_eq!(percent_for(10), 33);
        assert_eq!(percent_for(0), 200, "clamped low");
        assert_eq!(percent_for(42), 33, "clamped high");
    }

    #[test]
    fn a_loop_path_is_resolution_then_the_recorders_layout() {
        let p = loop_path_in(Path::new("root"), "nord", "torus", 85, (1920, 1080));
        assert_eq!(
            p,
            Path::new("root").join("1920x1080").join("nord").join("torus@85.mp4")
        );
    }

    #[test]
    fn the_theme_folder_falls_back_to_the_house_theme() {
        let mut cfg = Config::default();
        cfg.theme = String::new();
        assert_eq!(theme_of(&cfg), crate::themes::catalog::HOUSE_ID);
        cfg.theme = "Nord".into();
        assert_eq!(theme_of(&cfg), "nord");
    }

    #[test]
    fn screens_are_one_line_per_distinct_size() {
        let t = screens_text(&[(1920, 1080, 1.0), (1440, 2560, 1.0), (1920, 1080, 1.0), (2400, 1600, 1.5), (0, 0, 1.0)]);
        assert_eq!(t, "1440 2560 1\n1920 1080 1\n2400 1600 1.5\n");
    }

    #[test]
    fn screens_are_rewritten_only_when_they_change() {
        let root = std::env::temp_dir().join(format!("panefx-deskfx-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(write_screens_in(&root, &[(800, 600, 1.0)]).unwrap());
        assert!(!write_screens_in(&root, &[(800, 600, 1.0)]).unwrap());
        assert!(write_screens_in(&root, &[(800, 600, 1.25)]).unwrap());
        assert_eq!(std::fs::read_to_string(root.join("screens")).unwrap(), "800 600 1.25\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_animation_has_an_about_line() {
        for n in NAMES {
            assert!(!about(n).is_empty(), "{n}");
        }
    }

    #[test]
    fn screens_parse_to_distinct_sizes() {
        assert_eq!(
            parse_screens("1440 2560 1\n1920 1080 1\nbad\n1920 1080 1\n2400 1600 1.5\n0 4 1\n"),
            vec![(1440, 2560), (1920, 1080), (2400, 1600)]
        );
    }

    #[test]
    fn loop_status_counts_screens_and_reads_the_grid() {
        let root = std::env::temp_dir().join(format!("panefx-deskfx-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let screens = [(1920, 1080), (720, 2560)];
        let st = status_in(&root, "nord", "torus", 85, &screens);
        assert_eq!((st.have, st.screens, st.baked()), (0, 2, false));
        let p = loop_path_in(&root, "nord", "torus", 85, screens[0]);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        std::fs::write(root.join("1920x1080").join("nord").join("torus@85.mp4.grid"), "147 54\n").unwrap();
        let st = status_in(&root, "nord", "torus", 85, &screens);
        assert_eq!((st.have, st.cols, st.rows, st.baked()), (1, 147, 54, false));
        let q = loop_path_in(&root, "nord", "torus", 85, screens[1]);
        std::fs::create_dir_all(q.parent().unwrap()).unwrap();
        std::fs::write(&q, b"x").unwrap();
        assert!(status_in(&root, "nord", "torus", 85, &screens).baked());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wsl_paths_and_the_bake_command() {
        assert_eq!(
            wsl_path(Path::new(r"C:\Users\x\AppData\Local\panefx\loops\deskfx")),
            "/mnt/c/Users/x/AppData/Local/panefx/loops/deskfx"
        );
        assert_eq!(
            bake_args("/mnt/c/l", "/mnt/c/t", "nord", "torus", &[200, 33]).join(" "),
            "-o /mnt/c/l -T /mnt/c/t nord torus 200 33"
        );
    }

    #[test]
    fn the_backdrop_is_the_flames_background() {
        let mut cfg = Config::default();
        cfg.effect_params.clear();
        cfg.wallpaper_effect_params.clear();
        assert_eq!(background_of(&cfg), Rgb(0, 0, 0));
        cfg.effect_params
            .entry("flames".into())
            .or_default()
            .insert("bg".into(), "#112233".into());
        assert_eq!(background_of(&cfg), Rgb(0x11, 0x22, 0x33));
        cfg.wallpaper_effect_params
            .entry("flames".into())
            .or_default()
            .insert("bg".into(), "#445566".into());
        assert_eq!(background_of(&cfg), Rgb(0x44, 0x55, 0x66));
        let b = Backdrop::new(3, 2, background_of(&cfg));
        assert_eq!(b.cell_at(0, 0), None);
        assert!(!b.changed());
    }
}
