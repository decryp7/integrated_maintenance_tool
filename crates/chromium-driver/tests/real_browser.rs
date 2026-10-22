//! Launches a real Chromium-based browser when one is installed (skipped otherwise) against a local server that
//! serves a page rendered by script.

use chromium_driver::{find, Chromium, LaunchOptions, Visibility};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

const PAGE: &str = "<!doctype html><html><head><title>Rendered</title></head><body><ul id=r></ul>\
<script>setTimeout(()=>{for(let i=1;i<=3;i++){const l=document.createElement('li');l.textContent='item '+i;r.appendChild(l);}},150)</script></body></html>";

fn serve() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut b = [0u8; 2048];
            let _ = s.read(&mut b);
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
                PAGE.len()
            );
        }
    });
    port
}

#[test]
fn renders_a_scripted_page_and_leaves_nothing_behind() {
    let Some(exe) = find() else {
        eprintln!("skipped: no Chromium-based browser installed");
        return;
    };
    let port = serve();
    let opts = LaunchOptions {
        accent: Some(0x2a7de1),
        visibility: Visibility::OffScreen,
        ..LaunchOptions::default()
    };
    let mut b = Chromium::launch(&exe, &opts).expect("launch");
    let dir = b.profile_dir().to_path_buf();
    let pid = b.pid().unwrap();
    // The theme was written into the profile before launch.
    let prefs = std::fs::read_to_string(dir.join("Default/Preferences")).unwrap();
    assert!(prefs.contains("user_color2"));

    let page = b.new_page().expect("page");
    page.block_heavy(true).unwrap();
    page.navigate(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(15))
        .expect("navigate");
    let mut n = 0;
    for _ in 0..40 {
        n = page
            .evaluate("document.querySelectorAll('#r li').length")
            .unwrap()
            .as_u64()
            .unwrap();
        if n == 3 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(n, 3, "script-rendered content appears");
    assert_eq!(page.info().unwrap().title, "Rendered");
    assert!(page.load_finished().unwrap());
    // The real automation signals stay on (no stealth) ...
    assert_eq!(page.evaluate("navigator.webdriver").unwrap(), json!(true));
    // ... and nothing of ours is in the page: no injected nodes, attributes or script, only what the server sent.
    assert_eq!(
        page.evaluate(
            "document.documentElement.outerHTML.includes('debrief') || document.documentElement.outerHTML.includes('AI is browsing')"
        )
        .unwrap(),
        json!(false)
    );
    assert_eq!(page.evaluate("document.querySelectorAll('script').length").unwrap(), json!(1));
    assert_eq!(page.text_excerpt().unwrap().trim(), "item 1\nitem 2\nitem 3");
    assert!(page.challenge().unwrap().is_none());
    // On and off screen both work.
    page.set_visible(true).unwrap();
    page.set_visible(false).unwrap();

    // Stop ends the process at once; the throwaway profile is removed on drop.
    b.kill();
    assert!(!b.is_running());
    #[cfg(unix)]
    {
        std::thread::sleep(Duration::from_millis(300));
        assert!(!chromium_driver::process::process_exists(pid), "browser process leaked");
    }
    drop(b);
    assert!(!dir.exists(), "throwaway profile left behind");
}

#[test]
fn dropping_the_browser_ends_the_process() {
    let Some(exe) = find() else { return };
    let b = Chromium::launch(&exe, &LaunchOptions::default()).expect("launch");
    let pid = b.pid().unwrap();
    drop(b);
    #[cfg(unix)]
    {
        std::thread::sleep(Duration::from_millis(300));
        assert!(!chromium_driver::process::process_exists(pid), "browser process leaked");
    }
    let _ = pid;
}
