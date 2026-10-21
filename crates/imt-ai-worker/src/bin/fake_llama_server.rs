//! A stand-in for llama-server, for the worker's tests only (never bundled). It speaks just enough of the
//! real server's HTTP API: `GET /health` (503 while "loading"), `POST /tokenize` (one token per word plus
//! BOS), `GET /props`, and `POST /completion` streamed as chunked SSE, with the API key checked from
//! `LLAMA_API_KEY`.
//!
//! Behaviour is scripted by files, because the worker starts it with an empty environment:
//! - `<model>.fake.json` next to the model: `{"mode": "normal" | "slow" | "crash_on_chat" | "never_ready"
//!   | "exit_on_start", "reply": "...", "loading_ms": 100, "n_ctx": 4096}`;
//! - next to this executable: `devices.txt` (printed by `--list-devices`), `list-fail` (`--list-devices`
//!   fails), `start-fail` (exits at once when serving).
//!
//! It records what it saw next to the model: `<model>.args.json`, `<model>.requests.jsonl`, `<model>.cancelled`.
//!
//! Images: started with `--mmproj`, `/completion` accepts the multimodal prompt (`prompt_string` with one
//! `<__media__>` per entry of `multimodal_data`) and starts its reply with "I see N image(s)."; without it, it
//! refuses images like llama-server does.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Script {
    model: PathBuf,
    mode: String,
    reply: String,
    n_ctx: usize,
    ready_at: Instant,
    key: Option<String>,
    mmproj: bool,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn side(model: &Path, ext: &str) -> PathBuf {
    let mut s = model.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let here = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    if std::env::vars().any(|(k, _)| k.starts_with("LLAMA_ARG_")) {
        eprintln!("fake: LLAMA_ARG_* leaked into the environment");
        std::process::exit(7);
    }
    if args.iter().any(|a| a == "--list-devices") {
        if here.join("list-fail").exists() {
            eprintln!("error while loading shared libraries: libcudart.so.13: cannot open shared object file");
            std::process::exit(1);
        }
        println!("Available devices:");
        match std::fs::read_to_string(here.join("devices.txt")) {
            Ok(d) => print!("{d}"),
            Err(_) => println!("  (none)"),
        }
        return;
    }
    if here.join("start-fail").exists() {
        eprintln!("ggml_vulkan: failed to initialize the device");
        std::process::exit(2);
    }
    let model = PathBuf::from(arg(&args, "-m").unwrap_or_default());
    let _ = std::fs::write(side(&model, ".args.json"), json!(args).to_string());
    let cfg: Value = std::fs::read_to_string(side(&model, ".fake.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(json!({}));
    let mode = cfg["mode"].as_str().unwrap_or("normal").to_string();
    if mode == "exit_on_start" {
        eprintln!("llama_model_load: error loading model: unknown model architecture: 'fancy'");
        std::process::exit(1);
    }
    let port: u16 = arg(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(9931);
    let script = Arc::new(Script {
        n_ctx: cfg["n_ctx"]
            .as_u64()
            .map(|n| n as usize)
            .or_else(|| arg(&args, "-c").and_then(|c| c.parse().ok()))
            .unwrap_or(4096),
        reply: cfg["reply"].as_str().unwrap_or("Hello from the fake server.").to_string(),
        ready_at: Instant::now() + Duration::from_millis(cfg["loading_ms"].as_u64().unwrap_or(100)),
        key: std::env::var("LLAMA_API_KEY").ok(),
        mmproj: arg(&args, "--mmproj").is_some(),
        model,
        mode,
    });
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("couldn't bind HTTP server socket: {e}");
            std::process::exit(1);
        }
    };
    for stream in listener.incoming().flatten() {
        let script = script.clone();
        std::thread::spawn(move || handle(stream, &script));
    }
}

fn respond(s: &mut TcpStream, status: u16, body: &Value) {
    let text = body.to_string();
    let _ = write!(
        s,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn chunk(s: &mut TcpStream, data: &str) -> std::io::Result<()> {
    write!(s, "{:x}\r\n{data}\r\n", data.len())?;
    s.flush()
}

fn handle(mut s: TcpStream, script: &Script) {
    let mut r = BufReader::new(match s.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut first = String::new();
    if r.read_line(&mut first).is_err() {
        return;
    }
    let (mut length, mut auth) = (0usize, String::new());
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "authorization" => auth = v.trim().to_string(),
                _ => {}
            }
        }
    }
    let mut body = vec![0u8; length.min(64 * 1024 * 1024)];
    if r.read_exact(&mut body).is_err() {
        return;
    }
    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(side(&script.model, ".requests.jsonl"))
    {
        let _ = writeln!(f, "{}", json!({"path": path, "body": req}));
    }
    if path == "/health" {
        if script.mode == "never_ready" || Instant::now() < script.ready_at {
            return respond(
                &mut s,
                503,
                &json!({"error": {"code": 503, "message": "Loading model", "type": "unavailable_error"}}),
            );
        }
        return respond(&mut s, 200, &json!({"status": "ok"}));
    }
    if let Some(key) = &script.key {
        if auth != format!("Bearer {key}") {
            return respond(
                &mut s,
                401,
                &json!({"error": {"code": 401, "message": "Invalid API Key", "type": "authentication_error"}}),
            );
        }
    }
    let prompt = req["prompt"]
        .as_str()
        .or_else(|| req["prompt"]["prompt_string"].as_str())
        .or_else(|| req["content"].as_str())
        .unwrap_or("");
    let tokens = prompt.split_whitespace().count() + 1;
    match path.as_str() {
        "/props" => respond(
            &mut s,
            200,
            &json!({"default_generation_settings": {"n_ctx": script.n_ctx}, "modalities": {"vision": script.mmproj}}),
        ),
        "/tokenize" => respond(&mut s, 200, &json!({"tokens": (0..tokens).collect::<Vec<_>>()})),
        "/completion" => completion(&mut s, script, &req, prompt, tokens),
        _ => respond(
            &mut s,
            404,
            &json!({"error": {"code": 404, "message": "File Not Found", "type": "not_found_error"}}),
        ),
    }
}

fn completion(s: &mut TcpStream, script: &Script, req: &Value, prompt: &str, tokens: usize) {
    if tokens > script.n_ctx {
        return respond(
            s,
            400,
            &json!({"error": {"code": 400, "type": "exceed_context_size_error", "n_prompt_tokens": tokens, "n_ctx": script.n_ctx,
                "message": format!("request ({tokens} tokens) exceeds the available context size ({} tokens)", script.n_ctx)}}),
        );
    }
    if req["json_schema"]["type"] == "bogus" {
        return respond(
            s,
            400,
            &json!({"error": {"code": 400, "type": "invalid_request_error", "message": "Field 'json_schema': JSON schema conversion failed"}}),
        );
    }
    let images = req["prompt"]["multimodal_data"].as_array().map_or(0, Vec::len);
    if images > 0 && !script.mmproj {
        return respond(
            s,
            500,
            &json!({"error": {"code": 500, "type": "server_error", "message": "image input is not supported - hint: if this is unexpected, you may need to provide the mmproj"}}),
        );
    }
    if images != prompt.matches("<__media__>").count() {
        return respond(
            s,
            400,
            &json!({"error": {"code": 400, "type": "invalid_request_error", "message": "number of bitmaps does not match number of markers"}}),
        );
    }
    let crashed = side(&script.model, ".crashed");
    if script.mode == "crash_on_chat" && !crashed.exists() {
        let _ = std::fs::write(&crashed, b"1");
        eprintln!("GGML_ASSERT(fake) failed");
        std::process::exit(3);
    }
    let preserved: Vec<&str> = req["preserved_tokens"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut pieces: Vec<String> = if prompt.contains("TOOLTAG") {
        // The tags are special tokens here: printed only when preserved, like llama-server does.
        ["<tool_call>", "{\"name\":\"x\"}", "</tool_call>"]
            .iter()
            .filter(|p| !p.starts_with('<') || preserved.contains(p))
            .map(|p| p.to_string())
            .collect()
    } else if images > 0 {
        format!("I see {images} image(s). {}", script.reply)
            .split_inclusive(' ')
            .map(str::to_string)
            .collect()
    } else {
        script.reply.split_inclusive(' ').map(str::to_string).collect()
    };
    let (n_predict, mut stop_type) = (req["n_predict"].as_u64().unwrap_or(u64::MAX) as usize, "eos");
    if pieces.len() > n_predict {
        pieces.truncate(n_predict);
        stop_type = "limit";
    }
    let slow = script.mode == "slow";
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
    if s.write_all(head.as_bytes()).is_err() {
        return;
    }
    let total = if slow { 10_000 } else { pieces.len() };
    for i in 0..total {
        let piece = pieces.get(i % pieces.len().max(1)).cloned().unwrap_or_default();
        let event = json!({"content": piece, "stop": false, "tokens_predicted": i + 1, "tokens_evaluated": tokens});
        if chunk(s, &format!("data: {event}\n\n")).is_err() {
            let _ = std::fs::write(side(&script.model, ".cancelled"), b"1");
            return;
        }
        if slow {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let done = json!({"content": "", "stop": true, "stop_type": stop_type, "tokens_predicted": total, "tokens_evaluated": tokens});
    let _ = chunk(s, &format!("data: {done}\n\n"));
    let _ = s.write_all(b"0\r\n\r\n");
}
