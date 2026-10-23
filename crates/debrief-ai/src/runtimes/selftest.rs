//! The self-test a newly downloaded or updated llama-server build passes before Debrief switches to it (ADR 0009):
//! with the user's active model, one short text completion and, when the model has an image projector, one tiny
//! generated image (a red block) whose colour the model must name. A build that fails is never made active; the
//! previous build stays in use and the error says why and what to do.

use crate::error::{AiError, Result};
use crate::llm::{ChatMessage, ChatRequest, ImageInput, LlmProvider};
use crate::runtime::{load_with_fallback, LoadRequest, ModelRuntime};
use crate::vision::{sanitize, ImageLimits};
use crate::worker::protocol::ServerInfo;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The words accepted as naming the test image's colour.
const RED: &[&str] = &["red", "crimson", "scarlet", "maroon"];
/// Side of the generated test image.
const SIDE: u32 = 32;

/// What a passed self-test checked.
#[derive(Debug, Clone, PartialEq)]
pub struct SelfTestReport {
    pub backend: String,
    /// The image check ran (the model has a projector) and passed.
    pub image_checked: bool,
    pub server: Option<ServerInfo>,
}

/// A 32×32 PNG: a red block on white, made here (never a file), as the model sees any image (sanitised).
pub fn test_image() -> Result<ImageInput> {
    use ::image::{ImageFormat, Rgb, RgbImage};
    let img = RgbImage::from_fn(SIDE, SIDE, |x, y| {
        if (6..26).contains(&x) && (6..26).contains(&y) {
            Rgb([220, 20, 20])
        } else {
            Rgb([255, 255, 255])
        }
    });
    let mut png = std::io::Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgb8(img)
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| AiError::Invalid(format!("the self-test image: {e}")))?;
    let clean = sanitize(png.get_ref(), u64::from(SIDE * SIDE), &ImageLimits::default())
        .map_err(|e| AiError::Invalid(format!("the self-test image: {e}")))?;
    Ok(clean.image)
}

/// Whether `answer` names the test image's colour.
pub fn names_red(answer: &str) -> bool {
    let lower = answer.to_lowercase();
    lower.split(|c: char| !c.is_alphabetic()).any(|w| RED.contains(&w))
}

fn failed(build: &str, model: &str, what: &str, why: impl std::fmt::Display) -> AiError {
    AiError::SelfTest(format!(
        "The new {build} inference runtime failed its self-test with {model} ({what}: {why}). Debrief keeps using the previous runtime. \
         Try another build in Settings → AI, or keep the current one until the next update."
    ))
}

fn ask(provider: &dyn LlmProvider, message: ChatMessage, cancel: &Arc<AtomicBool>) -> Result<String> {
    let req = ChatRequest {
        system: String::new(),
        messages: vec![message],
        max_tokens: 16,
        temperature: Some(0.0),
        cancel: Some(cancel.clone()),
        ..ChatRequest::default()
    };
    Ok(provider.chat(&req, &mut |_| {})?.text)
}

/// How long a build gets to print its version.
pub const SMOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The check every new build passes, with or without a model: `server --version` starts, exits successfully within
/// `timeout` (missing libraries, a wrong CPU architecture or a blocked binary fail here) and is then reaped.
/// Cancelling stops it with [`AiError::Cancelled`]; every failure is [`AiError::SelfTest`] saying what to do.
pub fn smoke_check(server: &std::path::Path, build: &str, timeout: std::time::Duration, cancel: &Arc<AtomicBool>) -> Result<()> {
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    let fail = |why: String| {
        AiError::SelfTest(format!(
            "the new {build} inference runtime does not start on this computer ({why}). Choose another build in the AI settings."
        ))
    };
    let mut child = std::process::Command::new(server)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| fail(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(fail(format!("it exited with {status}"))),
            Ok(None) => {}
            Err(e) => return Err(fail(e.to_string())),
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AiError::Cancelled);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail("it did not answer in time".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Runs the self-test of the build `runtime` starts (`build` names it in messages) with the model of `req`.
/// Cancelling stops it with [`AiError::Cancelled`]; every failure is [`AiError::SelfTest`].
pub fn self_test(runtime: &dyn ModelRuntime, req: LoadRequest, build: &str, cancel: &Arc<AtomicBool>) -> Result<SelfTestReport> {
    let model = req.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let with_projector = req.projector.is_some();
    let check = |r: Result<String>, what: &str| match r {
        Err(AiError::Cancelled) => Err(AiError::Cancelled),
        Err(e) => Err(failed(build, &model, what, e)),
        Ok(t) => Ok(t),
    };
    let (loaded, _) = load_with_fallback(runtime, req).map_err(|e| match e {
        AiError::Cancelled => AiError::Cancelled,
        e => failed(build, &model, "it did not load the model", e),
    })?;
    let provider = loaded.provider.as_ref();
    let text = check(
        ask(provider, ChatMessage::user("Reply with the single word OK."), cancel),
        "a short answer",
    )?;
    if text.trim().is_empty() {
        return Err(failed(build, &model, "a short answer", "it answered nothing"));
    }
    if with_projector {
        if !provider.vision() {
            return Err(failed(build, &model, "reading an image", "it reports that it cannot read images"));
        }
        let question = ChatMessage::user_with_images(
            "What colour is the square in this image? Answer with one word.",
            vec![test_image()?],
        );
        let answer = check(ask(provider, question, cancel), "reading an image")?;
        if !names_red(&answer) {
            let mut shown: String = answer.trim().chars().take(60).collect();
            if shown.is_empty() {
                shown = "nothing".into();
            }
            return Err(failed(
                build,
                &model,
                "reading an image",
                format!("asked for the colour of a red square it answered \"{shown}\""),
            ));
        }
    }
    Ok(SelfTestReport {
        backend: loaded.backend,
        image_checked: with_projector,
        server: loaded.server,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf;
    use crate::llm::{Reply, ScriptedProvider};
    use crate::runtime::{GpuMode, Loaded};
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[cfg(unix)]
    fn script(dir: &std::path::Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("llama-server");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn the_smoke_check_passes_a_build_that_prints_its_version_and_names_every_failure() {
        let d = tempfile::tempdir().unwrap();
        let no = Arc::new(AtomicBool::new(false));
        let t = std::time::Duration::from_secs(5);
        assert!(smoke_check(&script(d.path(), "echo version: 1"), "CPU", t, &no).is_ok());
        let e = smoke_check(&script(d.path(), "exit 3"), "CPU", t, &no).unwrap_err();
        assert!(
            matches!(&e, AiError::SelfTest(m) if m.contains("CPU") && m.contains("exited")),
            "{e}"
        );
        let e = smoke_check(&script(d.path(), "sleep 5"), "CPU", std::time::Duration::from_millis(100), &no).unwrap_err();
        assert!(matches!(&e, AiError::SelfTest(m) if m.contains("in time")), "{e}");
        let yes = Arc::new(AtomicBool::new(true));
        assert!(matches!(
            smoke_check(&script(d.path(), "sleep 5"), "CPU", t, &yes),
            Err(AiError::Cancelled)
        ));
    }

    #[test]
    fn the_smoke_check_reports_a_missing_server() {
        let e = smoke_check(
            std::path::Path::new("/nonexistent/llama-server"),
            "CPU",
            SMOKE_TIMEOUT,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(matches!(e, AiError::SelfTest(_)));
    }

    struct Fake {
        loads: Result<()>,
        vision: bool,
        replies: Mutex<Vec<Reply>>,
        provider: Mutex<Option<Arc<ScriptedProvider>>>,
    }

    impl Fake {
        fn new(vision: bool, replies: Vec<Reply>) -> Fake {
            Fake {
                loads: Ok(()),
                vision,
                replies: Mutex::new(replies),
                provider: Mutex::new(None),
            }
        }
    }

    impl ModelRuntime for Fake {
        fn load(&self, _req: &LoadRequest) -> Result<Loaded> {
            self.loads.clone()?;
            let mut p = ScriptedProvider::new(std::mem::take(&mut *self.replies.lock().unwrap()));
            if self.vision {
                p = p.with_vision();
            }
            let p = Arc::new(p);
            *self.provider.lock().unwrap() = Some(p.clone());
            Ok(Loaded {
                provider: p,
                backend: "CPU".into(),
                server: Some(ServerInfo {
                    build: Some("b2".into()),
                    ..ServerInfo::default()
                }),
            })
        }
    }

    fn req(projector: bool) -> LoadRequest {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.gguf");
        std::fs::write(&path, crate::testing::gguf::qwen_like()).unwrap();
        LoadRequest {
            info: gguf::inspect(&path).unwrap(),
            path,
            projector: projector.then(|| PathBuf::from("tiny.mmproj.gguf")),
            context: 2048,
            gpu: GpuMode::Cpu,
            threads: 0,
        }
    }

    fn run(f: &Fake, projector: bool) -> Result<SelfTestReport> {
        self_test(f, req(projector), "Vulkan", &Arc::new(AtomicBool::new(false)))
    }

    fn text(s: &str) -> Reply {
        Reply::Text(s.into())
    }

    #[test]
    fn a_text_model_passes_with_one_short_answer() {
        let f = Fake::new(false, vec![text("OK")]);
        let r = run(&f, false).unwrap();
        assert_eq!((r.backend.as_str(), r.image_checked), ("CPU", false));
        assert_eq!(r.server.unwrap().build.as_deref(), Some("b2"));
        let sent = f.provider.lock().unwrap().clone().unwrap().requests();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].messages[0].images.is_empty());
    }

    #[test]
    fn a_vision_model_must_name_the_colour_of_the_test_image() {
        let f = Fake::new(true, vec![text("OK"), text("Red.")]);
        assert!(run(&f, true).unwrap().image_checked);
        let sent = f.provider.lock().unwrap().clone().unwrap().requests();
        let image = &sent[1].messages[0].images[0];
        assert_eq!((image.width(), image.height()), (32, 32));
        assert!(image.bytes().starts_with(b"\x89PNG"));

        let wrong = Fake::new(true, vec![text("OK"), text("It is blue")]);
        let e = run(&wrong, true).unwrap_err();
        let AiError::SelfTest(m) = &e else { panic!("{e:?}") };
        assert!(
            m.contains("Vulkan") && m.contains("tiny.gguf") && m.contains("\"It is blue\""),
            "{m}"
        );
        assert!(m.contains("keeps using the previous runtime") && m.contains("Settings → AI"), "{m}");
        let silent = Fake::new(true, vec![text("OK"), text("  ")]);
        assert!(run(&silent, true).unwrap_err().to_string().contains("answered \"nothing\""));
        let blind = Fake::new(false, vec![text("OK")]);
        assert!(run(&blind, true).unwrap_err().to_string().contains("cannot read images"));
    }

    #[test]
    fn load_and_answer_failures_are_self_test_failures() {
        let mut f = Fake::new(false, vec![]);
        f.loads = Err(AiError::Engine("llama-server could not load the model".into()));
        let e = run(&f, false).unwrap_err();
        assert!(matches!(&e, AiError::SelfTest(m) if m.contains("did not load the model")), "{e:?}");
        let broken = Fake::new(false, vec![Reply::Fail(AiError::Engine("crashed".into()))]);
        assert!(run(&broken, false)
            .unwrap_err()
            .to_string()
            .contains("a short answer: the model engine failed: crashed"));
        let empty = Fake::new(false, vec![text("")]);
        assert!(run(&empty, false).unwrap_err().to_string().contains("answered nothing"));
        let cancelled = Fake::new(false, vec![Reply::Fail(AiError::Cancelled)]);
        assert_eq!(run(&cancelled, false).unwrap_err(), AiError::Cancelled);
        let mut f = Fake::new(false, vec![]);
        f.loads = Err(AiError::Cancelled);
        assert_eq!(run(&f, false).unwrap_err(), AiError::Cancelled);
    }

    #[test]
    fn colour_words() {
        for yes in ["Red", "red.", "The square is RED", "crimson"] {
            assert!(names_red(yes), "{yes}");
        }
        // Whole words only.
        for no in ["", "blue", "bored", "Fred", "reddish"] {
            assert!(!names_red(no), "{no}");
        }
    }
}
