//! A stand-in for `ffmpeg` and `ffprobe` used only by the tests (copied as `ffmpeg` / `ffprobe` into a runtime folder).
//! It understands exactly the argument lists `imt_video::analyze` builds and answers like FFmpeg does, from a tiny
//! scripted "video": a text file `FAKEVIDEO duration_ms=<n> [created=<rfc3339>]` followed by `<ms> <score>` lines, or a
//! YUV4MPEG2 file (the self-test clip). A file `fake-mode` beside the executable makes it misbehave: `hang`, `memory`,
//! `badexit`, `noframes`, `badprobe`. Never bundled.

use std::io::Write;
use std::path::Path;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for d in data {
        a = (a + u32::from(*d)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend((body.len() as u32).to_be_bytes());
    let mut crc_in = kind.to_vec();
    crc_in.extend(body);
    out.extend(&crc_in);
    out.extend(crc32(&crc_in).to_be_bytes());
}

/// A valid 32x24 RGB PNG whose colour depends on `n`.
fn png(n: usize) -> Vec<u8> {
    let (w, h) = (32usize, 24usize);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let block = x / 8 == n % 4 && y / 8 == n % 3;
            raw.extend(if block {
                [250, 250, 250]
            } else {
                [(n * 37 % 256) as u8, (n * 91 % 256) as u8, 80]
            });
        }
    }
    let mut z = vec![0x78, 0x01, 0x01];
    z.extend((raw.len() as u16).to_le_bytes());
    z.extend((!(raw.len() as u16)).to_le_bytes());
    z.extend(&raw);
    z.extend(adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

struct Clip {
    duration_ms: u64,
    created: Option<String>,
    frames: Vec<(u64, f32)>,
    w: u32,
    h: u32,
}

fn read_clip(path: &str) -> Option<Clip> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"YUV4MPEG2") {
        let head_end = bytes.iter().position(|b| *b == b'\n')?;
        let head = String::from_utf8_lossy(&bytes[..head_end]).to_string();
        let num = |p: char| {
            head.split(' ')
                .find_map(|t| t.strip_prefix(p))
                .and_then(|v| v.split(':').next()?.parse::<u64>().ok())
        };
        let (w, h, fps) = (num('W')?, num('H')?, num('F')?.max(1));
        let n = bytes.windows(6).filter(|w| *w == b"FRAME\n").count() as u64;
        return Some(Clip {
            duration_ms: n * 1000 / fps,
            created: None,
            frames: (0..n).map(|i| (i * 1000 / fps, 0.5)).collect(),
            w: w as u32,
            h: h as u32,
        });
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let mut lines = text.lines();
    let head = lines.next()?.strip_prefix("FAKEVIDEO")?.to_string();
    let get = |k: &str| head.split_whitespace().find_map(|t| t.strip_prefix(k)).map(str::to_string);
    let frames = lines
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((p.next()?.parse().ok()?, p.next().and_then(|s| s.parse().ok()).unwrap_or(0.3)))
        })
        .collect();
    Some(Clip {
        duration_ms: get("duration_ms=")?.parse().ok()?,
        created: get("created="),
        frames,
        w: 1920,
        h: 1080,
    })
}

fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn report(line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("ffreport.log") {
        let _ = writeln!(f, "{line}");
    }
}

fn arg_after<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn misbehave(mode: &str) {
    match mode {
        "hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        },
        "memory" => {
            let mut hog: Vec<Vec<u8>> = Vec::new();
            loop {
                let mut chunk = vec![0u8; 64 << 20];
                for i in (0..chunk.len()).step_by(4096) {
                    chunk[i] = 1;
                }
                hog.push(chunk);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        "badexit" => {
            report("[in#0 @ 0x1] Error opening input: Invalid data found when processing input");
            std::process::exit(1);
        }
        _ => {}
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Like FFmpeg, start the report afresh.
    let _ = std::fs::remove_file("ffreport.log");
    let exe = std::env::current_exe().unwrap_or_default();
    let name = exe.file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let mode = exe
        .parent()
        .and_then(|d| std::fs::read_to_string(d.join("fake-mode")).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    if mode == "hang" || mode == "memory" || mode == "badexit" {
        misbehave(&mode);
    }
    let tool = if name.contains("ffprobe") { "ffprobe" } else { "ffmpeg" };
    if args.iter().any(|a| a == "-version") {
        let line = format!("{tool} version n9.0.2-fake Copyright (c) the tests\nconfiguration: --disable-gpl");
        println!("{line}");
        report(&line);
        return;
    }
    let Some(input) = arg_after(&args, "-i").map(str::to_string).or_else(|| args.last().cloned()) else {
        std::process::exit(2)
    };
    let Some(clip) = read_clip(&input) else {
        report(&format!("[in#0 @ 0x1] Error opening input file {input}: Invalid data found"));
        std::process::exit(1)
    };
    if tool == "ffmpeg" && !args.iter().any(|a| a == "-hide_banner") {
        report("ffmpeg version n9.0.2-fake Copyright (c) the tests");
        report("  configuration: --disable-gpl");
    }
    if tool == "ffprobe" {
        if mode == "badprobe" {
            let _ = std::fs::write("probe.json", "{ not json");
            return;
        }
        let created = clip
            .created
            .as_deref()
            .map(|c| format!(r#","tags":{{"creation_time":"{c}"}}"#))
            .unwrap_or_default();
        let count = if args.iter().any(|a| a == "-count_frames") {
            format!(r#","nb_read_frames":"{}""#, clip.frames.len())
        } else {
            String::new()
        };
        let json = format!(
            r#"{{"streams":[{{"codec_type":"video","codec_name":"fake","width":{},"height":{},"avg_frame_rate":"5/1"{count}}}],"format":{{"format_name":"fake","duration":"{}"{created}}}}}"#,
            clip.w,
            clip.h,
            secs(clip.duration_ms)
        );
        let _ = std::fs::write(arg_after(&args, "-o").unwrap_or("probe.json"), json);
        return;
    }
    let vf = arg_after(&args, "-vf").unwrap_or("").to_string();
    if let Some(p) = arg_after(&args, "-progress") {
        let _ = std::fs::write(p, format!("out_time_us={}\nprogress=end\n", clip.duration_ms * 1000));
    }
    if vf.contains("metadata=mode=print") {
        if mode == "noframes" {
            let _ = std::fs::write("candidates.txt", "");
            return;
        }
        let mut text = String::new();
        for (i, (ms, score)) in clip.frames.iter().enumerate() {
            text.push_str(&format!(
                "frame:{i:<4} pts:{} pts_time:{}\nlavfi.scene_score={score:.6}\n",
                ms * 90,
                secs(*ms)
            ));
        }
        let _ = std::fs::write("candidates.txt", text);
        return;
    }
    // Extraction: the times asked for in `select='lt(abs(t-X),tol)+...'` or `gte(t,X)`.
    let wanted: Vec<u64> = if vf.contains("gte(t,") {
        let from = vf
            .split("gte(t,")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let from_ms = (from * 1000.0).round() as u64;
        clip.frames.iter().map(|f| f.0).filter(|ms| *ms >= from_ms).take(1).collect()
    } else {
        vf.split("abs(t-")
            .skip(1)
            .filter_map(|r| r.split(')').next()?.parse::<f64>().ok())
            .map(|s| (s * 1000.0).round() as u64)
            .filter_map(|want| clip.frames.iter().map(|f| f.0).find(|ms| ms.abs_diff(want) <= 1))
            .collect()
    };
    if mode == "noframes" {
        return;
    }
    for (n, ms) in wanted.iter().enumerate() {
        let _ = std::fs::write(format!("frame-{:04}.png", n + 1), png(n));
        report(&format!(
            "[Parsed_showinfo_1 @ 0x1] n: {n:>3} pts: {} pts_time:{} duration: 1",
            ms * 90,
            secs(*ms)
        ));
    }
    let _ = Path::new("progress.txt");
}
