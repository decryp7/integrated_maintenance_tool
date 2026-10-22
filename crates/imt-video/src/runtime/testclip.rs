//! The clip the self-test analyses (ADR 0032 §1): generated, never bundled and never real footage. A YUV4MPEG2 file
//! (FFmpeg reads it without any codec library): 64×48 pixels, 5 frames per second, six frames, each with another
//! background and a white block in another place, so every frame is a change and `mpdecimate` keeps all of them.

/// Frames of the clip.
pub const FRAMES: usize = 6;
/// Frames per second.
pub const FPS: u64 = 5;
pub const WIDTH: usize = 64;
pub const HEIGHT: usize = 48;

/// The frame times the self-test expects (ms).
pub fn frame_times() -> Vec<u64> {
    (0..FRAMES as u64).map(|i| i * 1000 / FPS).collect()
}

/// The clip's length (ms).
pub fn duration_ms() -> u64 {
    FRAMES as u64 * 1000 / FPS
}

/// The clip's bytes.
pub fn y4m() -> Vec<u8> {
    let mut out = format!("YUV4MPEG2 W{WIDTH} H{HEIGHT} F{FPS}:1 Ip A1:1 C420jpeg\n").into_bytes();
    for i in 0..FRAMES {
        out.extend_from_slice(b"FRAME\n");
        let background = 16 + (i as u8) * 36;
        let (bx, by) = ((i * 8) % (WIDTH - 16), (i * 5) % (HEIGHT - 16));
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let block = (bx..bx + 16).contains(&x) && (by..by + 16).contains(&y);
                out.push(if block { 235 } else { background });
            }
        }
        // Chroma: neutral grey, a quarter of the luma each.
        out.extend(std::iter::repeat_n(128u8, WIDTH * HEIGHT / 2));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clip_is_a_valid_y4m_of_six_different_frames() {
        let b = y4m();
        let header_len = b.iter().position(|c| *c == b'\n').unwrap() + 1;
        assert_eq!(&b[..header_len], b"YUV4MPEG2 W64 H48 F5:1 Ip A1:1 C420jpeg\n");
        let frame = 6 + WIDTH * HEIGHT * 3 / 2;
        assert_eq!(b.len(), header_len + FRAMES * frame);
        let lumas: Vec<&[u8]> = (0..FRAMES).map(|i| &b[header_len + i * frame + 6..header_len + i * frame + 6 + WIDTH * HEIGHT]).collect();
        assert!(lumas.windows(2).all(|w| w[0] != w[1]));
        assert_eq!(frame_times(), [0, 200, 400, 600, 800, 1000]);
        assert_eq!(duration_ms(), 1200);
        assert_eq!(y4m(), b, "deterministic");
    }
}
