//! Images through the AI commands (ADR 0025): opening and pasting images (decoded in Rust, the clean copy
//! only), the no-vision refusal, explaining an image, "find related logs" end to end with a scripted model that
//! reads images (the engine searches the samples), the chat's image turns, and image support for a model.

use super::*;
use base64::Engine as _;
use imt_ai::vision::image::testing::{claimed_png, screenshot_png};

fn vision(replies: Vec<Reply>) -> Arc<dyn LlmProvider> {
    Arc::new(ScriptedProvider::new(replies).with_vision())
}

/// An error dialog of the samples' payment failure at 10:41:14 on 2026-10-14, as a model reads it.
fn dialog_facts() -> String {
    json!({
        "screen_kind": "error_dialog",
        "window_title": "OrderService - Error",
        "application": "OrderService",
        "timestamps": [{"text": "2026-10-14 10:41:15", "has_date": true}],
        "error_texts": ["Failed to capture payment for order 91698: request timed out"],
        "exception_names": ["System.Net.Http.HttpRequestException"],
        "machine_names": ["WEB01"],
        "summary": "An error dialog of OrderService."
    })
    .to_string()
}

fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, screenshot_png(w, h)).unwrap();
    p
}

#[test]
fn images_open_as_clean_copies_within_limits() {
    let f = Fixture::new("vision-open");
    let core = f.core();
    core.set_fixed_provider(vision(vec![]));
    let app = f.app(core);
    let main = window(&app, "main");
    let shot = write_png(&f.tmp.0, "Screenshot 2026-10-14 104115.png", 2400, 1350);
    // Only a file the user chose.
    assert_eq!(
        code(ipc(&main, "ai_image_open", json!({"path": shot.to_string_lossy()})).unwrap_err()),
        "notGranted"
    );
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    let img = ipc(&main, "ai_image_open", json!({"path": shot.to_string_lossy()})).unwrap();
    assert_eq!(img["name"], "Screenshot 2026-10-14 104115.png");
    assert_eq!(
        (img["originalWidth"].as_u64(), img["originalHeight"].as_u64()),
        (Some(2400), Some(1350))
    );
    let (w, h) = (img["width"].as_u64().unwrap(), img["height"].as_u64().unwrap());
    assert!(w * h <= imt_ai::vision::image::DEFAULT_TARGET_PIXELS && w < 2400, "{w}x{h}");
    assert!(img["dataUrl"].as_str().unwrap().starts_with("data:image/png;base64,"));
    assert_eq!(img["sourceFormat"], "png");
    // Not an image, a bomb's header, an unsupported extension: typed refusals.
    let fake = f.tmp.0.join("fake.png");
    std::fs::write(&fake, b"<svg onload=alert(1)>").unwrap();
    assert_eq!(
        code(ipc(&main, "ai_image_open", json!({"path": fake.to_string_lossy()})).unwrap_err()),
        "invalid"
    );
    let bomb = f.tmp.0.join("bomb.png");
    std::fs::write(&bomb, claimed_png(100_000, 100_000)).unwrap();
    let e = ipc(&main, "ai_image_open", json!({"path": bomb.to_string_lossy()})).unwrap_err();
    assert!(e["message"].as_str().unwrap().contains("claims"), "{e}");
    let txt = f.tmp.0.join("notes.txt");
    std::fs::write(&txt, b"x").unwrap();
    assert!(ipc(&main, "ai_image_open", json!({"path": txt.to_string_lossy()})).is_err());
    // Pasted bytes are decoded in Rust too.
    let data = base64::engine::general_purpose::STANDARD.encode(screenshot_png(40, 30));
    let pasted = ipc(&main, "ai_image_paste", json!({"data": data, "name": "clip"})).unwrap();
    assert_eq!((pasted["width"].as_u64(), pasted["name"].as_str()), (Some(40), Some("clip")));
    assert_eq!(code(ipc(&main, "ai_image_paste", json!({"data": "%%%"})).unwrap_err()), "invalid");
    let not_image = base64::engine::general_purpose::STANDARD.encode(b"MZ\x90\0");
    assert_eq!(
        code(ipc(&main, "ai_image_paste", json!({"data": not_image})).unwrap_err()),
        "invalid"
    );
    assert_eq!(ipc(&main, "ai_image_close", json!({"id": pasted["id"]})).unwrap(), true);
    assert_eq!(ipc(&main, "ai_image_close", json!({"id": pasted["id"]})).unwrap(), false);
}

#[test]
fn a_model_without_vision_is_refused_before_anything_is_queued() {
    let f = Fixture::new("vision-none");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![text("unused")]));
    let app = f.app(core);
    let main = window(&app, "main");
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["vision"], false);
    let shot = write_png(&f.tmp.0, "a.png", 64, 40);
    let img = ipc(&main, "ai_image_open", json!({"path": shot.to_string_lossy()})).unwrap();
    for op in [
        json!({"op": "explainImage", "imageId": img["id"]}),
        json!({"op": "relatedLogs", "imageId": img["id"]}),
    ] {
        assert_eq!(code(ipc(&main, "ai_start", json!({ "request": op })).unwrap_err()), "noVision");
    }
    let chat = ipc(&main, "ai_chat_new", json!({})).unwrap();
    assert_eq!(
        code(
            ipc(
                &main,
                "ai_chat_send",
                json!({"sessionId": chat["id"], "text": "what is this?", "images": [img["id"]]})
            )
            .unwrap_err()
        ),
        "noVision"
    );
    assert!(active_ai(&app).is_empty());
    // An image that is gone is a typed error too.
    assert_eq!(
        code(ipc(&main, "ai_start", json!({"request": {"op": "explainImage", "imageId": 999}})).unwrap_err()),
        "notFound"
    );
}

#[test]
fn explain_and_find_related_logs_run_as_jobs_and_the_engine_finds_the_event() {
    let f = Fixture::new("vision-related");
    let core = f.core();
    core.set_fixed_provider(vision(vec![
        text("An OrderService error dialog: a payment capture timed out."),
        text("not json"),
        text(&dialog_facts()),
        text("The dialog is the payment failure of order 91698."),
    ]));
    let app = f.app(core);
    let main = window(&app, "main");
    ipc(
        &main,
        "open_paths",
        json!({ "paths": [samples().join("serilog-clef/web01/orders-20261014.clef").to_string_lossy()] }),
    )
    .unwrap();
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["vision"], true);
    let shot = write_png(&f.tmp.0, "dialog.png", 800, 450);
    let img = ipc(&main, "ai_image_open", json!({"path": shot.to_string_lossy()})).unwrap();
    // Explain: a narrative.
    let id = start(&main, json!({"op": "explainImage", "imageId": img["id"]}));
    let done = ipc(&main, "jobs_wait", json!({"id": id})).unwrap();
    assert_eq!(done["state"], "done");
    assert!(done["result"]["narrative"]["text"].as_str().unwrap().contains("OrderService"));
    // Related logs: the model reads (one retry for its invalid JSON), the engine searches, the model explains.
    let id = start(&main, json!({"op": "relatedLogs", "imageId": img["id"]}));
    let done = ipc(&main, "jobs_wait", json!({"id": id})).unwrap();
    assert_eq!(done["state"], "done", "{done}");
    let r = &done["result"]["result"];
    assert_eq!(done["result"]["type"], "relatedLogs");
    assert_eq!(r["facts"]["windowTitle"], "OrderService - Error");
    let top = &r["result"]["links"][0];
    assert!(top["event"]["message"].as_str().unwrap().contains("91698"), "{top}");
    let reasons: Vec<&str> = top["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(reasons[0], "time");
    assert!(reasons.contains(&"text"));
    assert_eq!(r["result"]["time"]["source"], "screen");
    assert!(r["explanation"]["text"].as_str().unwrap().contains("91698"));
    // The job is linked to the image file.
    let job = ended(&app, id).unwrap();
    assert_eq!(job.title, "Find the logs related to an image");
}

#[test]
fn chat_turns_show_images_and_a_related_request_runs_the_search() {
    let f = Fixture::new("vision-chat");
    let core = f.core();
    let provider =
        Arc::new(ScriptedProvider::new(vec![text("A frozen window."), text(&dialog_facts()), text("Event #1 matches.")]).with_vision());
    core.set_fixed_provider(provider.clone());
    let app = f.app(core);
    let main = window(&app, "main");
    ipc(
        &main,
        "open_paths",
        json!({ "paths": [samples().join("serilog-clef/web01/orders-20261014.clef").to_string_lossy()] }),
    )
    .unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(screenshot_png(120, 80));
    let img = ipc(&main, "ai_image_paste", json!({"data": data})).unwrap();
    let chat = ipc(&main, "ai_chat_new", json!({})).unwrap();
    let sid = chat["id"].as_str().unwrap().to_string();
    // A plain question: the image goes to the model with it; the history keeps a note, not the image.
    ipc(
        &main,
        "ai_chat_send",
        json!({"sessionId": sid, "text": "what is this?", "images": [img["id"]]}),
    )
    .unwrap();
    let events = finish_turn(&main, &sid);
    assert!(kinds(&events).contains(&"done".to_string()));
    let last = provider.requests().last().unwrap().clone();
    assert_eq!(last.messages.last().unwrap().images.len(), 1);
    let record = ipc(&main, "ai_chat_get", json!({"sessionId": sid})).unwrap();
    assert_eq!(record["turns"][0]["user"], "what is this? [image]");
    // "Find the logs related to this screenshot": the engine's search, cited.
    ipc(
        &main,
        "ai_chat_send",
        json!({"sessionId": sid, "text": "find the logs related to this screenshot", "images": [img["id"]]}),
    )
    .unwrap();
    let events = finish_turn(&main, &sid);
    let done = events.iter().rev().find(|e| e["type"] == "done").unwrap();
    let text = done["text"].as_str().unwrap();
    assert!(text.contains("Linked events:") && text.contains("91698"), "{text}");
    assert!(done["citations"].as_array().unwrap().iter().any(|c| c["kind"] == "record"));
    assert!(events
        .iter()
        .any(|e| e["type"] == "toolStarted" && e["name"] == "find_related_logs"));
    // Too many images, or one that is gone.
    let many: Vec<Value> = (0..5).map(|_| img["id"].clone()).collect();
    assert_eq!(
        code(ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "x", "images": many})).unwrap_err()),
        "invalid"
    );
    assert_eq!(
        code(ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "x", "images": [77]})).unwrap_err()),
        "notFound"
    );
}

#[test]
fn image_support_is_added_to_an_installed_model_and_reported() {
    let f = Fixture::new("vision-projector");
    let core = f.core();
    let app = f.app(core);
    let main = window(&app, "main");
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    let src = f.tmp.0.join("model.gguf");
    std::fs::write(&src, model_bytes()).unwrap();
    let added = ipc(&main, "ai_model_add_local", json!({"path": src.to_string_lossy()})).unwrap();
    assert_eq!((added["vision"].as_bool(), added["projector"].is_null()), (Some(false), true));
    // The test catalog's model has no projector: "Add image support" says so instead of downloading anything.
    let e = ipc(&main, "ai_model_install_projector", json!({"fileName": TEST_FILE})).unwrap_err();
    assert_eq!(code(e.clone()), "notFound");
    assert!(e["message"].as_str().unwrap().contains("Text + images"), "{e}");
    // A projector file the user picked: one of another model is refused, the right one pairs.
    let wrong = f.tmp.0.join("other-mmproj.gguf");
    std::fs::write(&wrong, imt_ai::testing::gguf::projector_like(4096)).unwrap();
    let e = ipc(
        &main,
        "ai_model_add_projector",
        json!({"fileName": TEST_FILE, "path": wrong.to_string_lossy()}),
    )
    .unwrap_err();
    assert!(e["message"].as_str().unwrap().contains("another model"), "{e}");
    let proj = f.tmp.0.join("mmproj-F16.gguf");
    std::fs::write(&proj, imt_ai::testing::gguf::projector_like(2560)).unwrap();
    let m = ipc(
        &main,
        "ai_model_add_projector",
        json!({"fileName": TEST_FILE, "path": proj.to_string_lossy()}),
    )
    .unwrap();
    assert_eq!(m["vision"], true);
    assert_eq!(m["projector"]["fileName"], "Test-4B-Q4_K_M.mmproj.gguf");
    assert_eq!(
        ipc(&main, "ai_models", json!({})).unwrap().as_array().unwrap().len(),
        1,
        "the projector is not a model"
    );
    // The model's load request now carries the projector.
    ipc(&main, "ai_model_activate", json!({"fileName": TEST_FILE})).unwrap();
    wait_until("loaded with the projector", || f.runtime.projectors().iter().any(Option::is_some));
}
