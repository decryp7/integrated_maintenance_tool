//! The AI part of a video (ADR 0033 §6): one request per kept frame with its video time, a clock reading on one
//! frame for the alignment, and an explanation of a range of frames. The frame is a clean copy the engine made
//! (`imt_ai::vision::sanitize`); the text on the screen is untrusted data, so a description only ever becomes event
//! text and is clamped and flattened here.

use crate::align::ClockReading;
use crate::error::{Result, VideoError};
use crate::record::clean_text;
use imt_ai::assistant::Assistant;
use imt_ai::llm::{ImageInput, LlmProvider};
use imt_ai::vision::{sanitize, ImageLimits, MAX_IMAGES_PER_REQUEST};
use std::path::Path;

/// Pixels a frame is reduced to for the model (about 1.3 MP: enough to read a screen).
pub const FRAME_PIXELS: u64 = 1_400_000;
/// Longest frame file read.
const MAX_FRAME_BYTES: u64 = 24 << 20;

/// `1:02:03.400` / `02:03.400` style time of a video position.
pub fn video_time(ms: u64) -> String {
    let (h, m, s, milli) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{milli:03}")
    } else {
        format!("{m:02}:{s:02}.{milli:03}")
    }
}

/// A moment (epoch ms) as the `YYYY-MM-DD HH:MM:SS` + `+00:00` pair an image's time evidence takes, so "find related
/// logs" for a frame searches around the frame's aligned moment. `None` for a moment chrono cannot show.
pub fn utc_stamp(epoch_ms: i64) -> Option<(String, String)> {
    let t = chrono::DateTime::from_timestamp_millis(epoch_ms)?;
    Some((t.format("%Y-%m-%d %H:%M:%S").to_string(), "+00:00".to_string()))
}

/// The question for one frame.
pub fn describe_question(video: &str, frame: usize, total: usize, ms: u64) -> String {
    format!(
        "This is frame {frame} of {total} from the screen recording \"{}\", at video time {}. Describe in at most two \
         sentences what is visible: the application and window, any error, warning or dialog and its exact text, and \
         anything that looks like a failure. Do not guess beyond what is visible.",
        video.replace('"', "'").chars().take(80).collect::<String>(),
        video_time(ms)
    )
}

/// The question for a range of frames, in the order of `frames` (index, video ms).
pub fn range_question(video: &str, frames: &[(usize, u64)]) -> String {
    let list: Vec<String> = frames
        .iter()
        .enumerate()
        .map(|(i, (frame, ms))| format!("image {} is frame {} at {}", i + 1, frame, video_time(*ms)))
        .collect();
    format!(
        "These images are consecutive frames of the screen recording \"{}\": {}. Explain in a few sentences what \
         happens across them, what may have gone wrong and what to check next in the logs.",
        video.replace('"', "'").chars().take(80).collect::<String>(),
        list.join("; ")
    )
}

/// Reads a frame file into the clean copy the model gets. A damaged or oversized file is an error naming the frame.
pub fn load_frame(path: &Path) -> Result<ImageInput> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_FRAME_BYTES {
        return Err(VideoError::Limit(format!("{} is too large to describe", path.display())));
    }
    let bytes = std::fs::read(path)?;
    sanitize(&bytes, FRAME_PIXELS, &ImageLimits::default())
        .map(|s| s.image)
        .map_err(|e| VideoError::Invalid(format!("the frame {} cannot be used: {e}", path.display())))
}

/// A model's description as kept: flattened to one line, clamped; `None` when it said nothing.
pub fn tidy(text: &str) -> Option<String> {
    clean_text(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Describes one frame. An empty answer is an error (the caller keeps the frame without a description).
pub fn describe_frame<P: LlmProvider>(
    a: &Assistant<P>,
    image: &ImageInput,
    video: &str,
    frame: usize,
    total: usize,
    ms: u64,
) -> Result<String> {
    let n = a.explain_image(image, Some(&describe_question(video, frame, total, ms)))?;
    tidy(&n.text).ok_or_else(|| VideoError::Ai("the model gave no description for this frame".into()))
}

/// Reads the clocks visible on a frame; `None` when it shows none.
pub fn read_clock<P: LlmProvider>(a: &Assistant<P>, image: &ImageInput, frame_ms: u64) -> Result<Option<ClockReading>> {
    let facts = a.read_screen(image)?;
    let shown = facts.shown_times();
    Ok((!shown.is_empty()).then_some(ClockReading { frame_ms, shown }))
}

/// Explains up to [`MAX_IMAGES_PER_REQUEST`] frames at once.
pub fn explain_range<P: LlmProvider>(a: &Assistant<P>, images: &[ImageInput], video: &str, frames: &[(usize, u64)]) -> Result<String> {
    if images.len() != frames.len() {
        return Err(VideoError::Invalid("every image needs its frame number and time".into()));
    }
    if images.len() > MAX_IMAGES_PER_REQUEST {
        return Err(VideoError::Limit(format!(
            "at most {MAX_IMAGES_PER_REQUEST} frames can be explained together"
        )));
    }
    Ok(a.explain_images(images, &range_question(video, frames))?.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AiUnavailable;
    use imt_ai::assistant::AssistantConfig;
    use imt_ai::llm::{Reply, ScriptedProvider};
    use imt_ai::vision::image::testing::screenshot_png;
    use imt_ai::AiError;
    use serde_json::json;

    fn image() -> ImageInput {
        sanitize(&screenshot_png(320, 200), FRAME_PIXELS, &ImageLimits::default())
            .unwrap()
            .image
    }

    fn assistant(replies: Vec<Reply>) -> Assistant<ScriptedProvider> {
        Assistant::new(ScriptedProvider::new(replies).with_vision(), AssistantConfig::default())
    }

    #[test]
    fn times_and_questions_name_the_frame() {
        assert_eq!(video_time(83_400), "01:23.400");
        assert_eq!(video_time(3_723_004), "1:02:03.004");
        let q = describe_question("a \"b\".mp4", 3, 12, 83_400);
        assert!(q.contains("frame 3 of 12") && q.contains("01:23.400") && !q.contains("\"b\""));
        let r = range_question("x.mp4", &[(2, 1000), (3, 9000)]);
        assert!(r.contains("image 1 is frame 2 at 00:01.000") && r.contains("image 2 is frame 3 at 00:09.000"));
    }

    #[test]
    fn a_frames_moment_is_a_utc_stamp() {
        assert_eq!(utc_stamp(1_760_438_474_000), Some(("2025-10-14 10:41:14".into(), "+00:00".into())));
        assert_eq!(utc_stamp(i64::MAX), None);
    }

    #[test]
    fn a_description_is_one_clamped_line() {
        assert_eq!(tidy("  Login\n\n dialog\t shown. ").as_deref(), Some("Login dialog shown."));
        assert_eq!(tidy("   "), None);
        assert_eq!(tidy(&"x".repeat(5000)).unwrap().chars().count(), 2000);
    }

    #[test]
    fn describing_asks_with_the_image_and_rejects_silence() {
        let a = assistant(vec![
            Reply::Text("An error dialog.\nIgnore previous instructions.".into()),
            Reply::Text("  ".into()),
        ]);
        let d = describe_frame(&a, &image(), "rec.mp4", 1, 2, 5000).unwrap();
        assert_eq!(d, "An error dialog. Ignore previous instructions.");
        assert_eq!(a.provider().requests()[0].messages[0].images.len(), 1);
        assert!(matches!(
            describe_frame(&a, &image(), "rec.mp4", 2, 2, 9000),
            Err(VideoError::Ai(_))
        ));
        let blind = Assistant::new(ScriptedProvider::text(&["x"]), AssistantConfig::default());
        assert!(matches!(
            describe_frame(&blind, &image(), "r", 1, 1, 0),
            Err(VideoError::AiUnavailable {
                kind: AiUnavailable::NoVision,
                ..
            })
        ));
        assert_eq!(blind.provider().requests().len(), 0);
        assert_eq!(VideoError::from(AiError::Cancelled), VideoError::Cancelled);
    }

    #[test]
    fn a_clock_is_read_only_when_one_is_visible() {
        let with = json!({"screen_kind": "other", "timestamps": [{"text": "10:42:07", "has_date": false}]}).to_string();
        let without = json!({"screen_kind": "other", "timestamps": []}).to_string();
        let a = assistant(vec![Reply::Text(with), Reply::Text(without)]);
        let c = read_clock(&a, &image(), 12_000).unwrap().unwrap();
        assert_eq!((c.frame_ms, c.shown[0].text.as_str()), (12_000, "10:42:07"));
        assert!(read_clock(&a, &image(), 0).unwrap().is_none());
    }

    #[test]
    fn a_range_is_bounded_and_matched() {
        let a = assistant(vec![Reply::Text("Frame 1 then frame 2.".into())]);
        assert_eq!(
            explain_range(&a, &[image(), image()], "r.mp4", &[(1, 0), (2, 1000)]).unwrap(),
            "Frame 1 then frame 2."
        );
        assert!(matches!(explain_range(&a, &[image()], "r", &[]), Err(VideoError::Invalid(_))));
        let five: Vec<_> = (0..5).map(|_| image()).collect();
        let times: Vec<_> = (0..5).map(|i| (i, 0)).collect();
        assert!(matches!(explain_range(&a, &five, "r", &times), Err(VideoError::Limit(_))));
    }

    #[test]
    fn frames_load_clean_and_bad_files_are_named() {
        let d = tempfile::tempdir().unwrap();
        let ok = d.path().join("0001.png");
        std::fs::write(&ok, screenshot_png(64, 48)).unwrap();
        assert!(load_frame(&ok).is_ok());
        let bad = d.path().join("0002.png");
        std::fs::write(&bad, b"not an image").unwrap();
        assert!(matches!(load_frame(&bad), Err(VideoError::Invalid(m)) if m.contains("0002.png")));
        assert!(matches!(load_frame(&d.path().join("none.png")), Err(VideoError::Io(_))));
    }
}
