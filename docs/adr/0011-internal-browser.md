# 11. The AI gets web content only through an internal browser the user drives

- Status: accepted
- Date: 2026-10-18

## Context

The assistant could search the web itself: the engine built a redacted query from an "intent", SearXNG
(self-hosted) or Brave Search (API key in the keychain) ran it after an approval, and a `fetch_page` tool
read result pages. In use this proved the wrong shape:

- Search APIs return snippets ranked for crawlers, not the forum thread or vendor page a support engineer
  would open, and every search needs a key, an instance or both.
- Each query, however redacted, is a request IMT makes on the user's behalf to a third party.
- Users look things up in a browser anyway, and then paste text into the chat by hand.

The user's decision: remove all AI-initiated web search and replace it with an internal browser the user
drives, through which the AI gets web content, with guardrails. Automation with stealth or bot-detection
evasion is explicitly out.

## Decision

1. **AI web search is removed**: the SearXNG and Brave adapters, the Brave key in the keychain (and the
   keyring dependency), the `web_search` and `fetch_page` tools with their approval flow, the web search
   settings and their UI. Kept for the browser: the privacy guard (`imt_ai::web::privacy`), names known
   from the logs, host classification and the untrusted-content quoting. The HTML-to-text extractor was
   removed with the page fetcher: the browser reads the rendered page (see below).
2. **An internal browser window** on the system webview (WebView2, WKWebView, WebKitGTK; nothing bundled)
   with its own profile: a `web-profile` folder in IMT's data folder (WebView2 user data folder, WebKitGTK
   data manager) and a fixed WebKit data store id on macOS 14+. The profile is a category of Settings ▸
   Storage. The user drives it from a **Web panel** in their IMT window (back, forward, reload/stop, an
   address and search box with an engine picker: DuckDuckGo, Google, Bing, Stack Overflow or a URL template
   with `{q}`, "Add page to AI context", "Open in your browser"). *Search the web for this error* (event
   context menu, inspector, chat, palette) opens a search text the engine built from the event with the
   privacy guard; the user edits it before anything opens.
3. **The AI gets web content four ways, all through `imt_ai::browser`:**
   - *Hand-off*: "Add page to AI context" captures the visible page's readable text (headings, paragraphs,
     lists, code, tables; never scripts, styles or hidden elements), capped and redacted, and adds it to the
     chat's next question as a cited source quoted as untrusted web content.
   - *Read-along* (opt-in per chat, off by default, ends with the chat): a `browser_read` tool reads the
     page shown; every read is cited.
   - *Suggested links*: `suggest_link` shows a search (redacted) or a page as a card with the full address;
     nothing loads until the user clicks Open.
   - *Agent mode* (opt-in per chat): the model drives the visible browser with `browser_search`,
     `browser_navigate`, `browser_click`, `browser_type`, `browser_scroll`, `browser_back` and
     `browser_read`, on a compact snapshot of the interactive elements with numbered refs.
4. **Agent mode's rules live in Rust** (`policy`, `leak`, `pace`, `snapshot`, `control`), pure and tested:
   a new site needs approval ("allow this site for this chat" covers later visits); typing needs approval
   with the exact text; submitting a form or pressing a button needs approval; links within allowed sites
   need none. Workspace data headed into an address or a field (names known from the logs, values and log
   lines found in the loaded logs, private-looking values) is highlighted on the card and always needs
   approval, even on an allowed site. The AI never goes to this machine or a private network, never types
   into a password field, and stops and hands back on a CAPTCHA, a login or a payment page; it never solves
   or bypasses them. Human pace (an actions-per-minute cap with a minimum gap), a step budget per task
   (default 25, Settings ▸ Web browser; "stopped after 25 steps; continue?"), a visible Stop in the chat and
   the Web panel that halts at once (a waiting approval is denied), the webview's honest user agent with no
   fingerprint changes. While the agent drives, the window refuses navigations to sites the chat may not
   visit. Every action is written to the activity log (action, site, outcome, step; never page or typed
   text), so diagnostics reports show what the AI did. The UI hints (never blocks) when the model is
   smaller than about 7B parameters.

### Architecture: a separate window controlled from a dock panel

Two shapes were possible: (a) an IMT dock panel that controls a separate browser window, or (b) Tauri's
multi-webview, a local toolbar webview and a remote content webview in one window. Tauri 2.12 (our
`Cargo.lock`) still ships multi-webview (`Window::add_child`) behind the `unstable` feature, which would
apply to the whole app, and it has open issues with focus, resizing and DPI on Linux and macOS. **We chose
(a)**: stable APIs only (`WebviewWindowBuilder` with `on_navigation`, `on_new_window`, `on_download`,
`on_page_load`, `data_directory`, `data_store_identifier`, `eval_with_callback`), and the remote page never
shares a window with IMT's own UI. One browser per IMT window (`browser-<window>`), closed with it.

### Security model

- **No IPC for pages.** The browser window has no Tauri capability (no capability file matches
  `browser-*`), so Tauri's ACL refuses every command from a remote origin. In addition the shell's invoke
  handler is wrapped by a guard that refuses every command from a `browser-*` webview before anything else
  runs, whatever the origin. IMT's own origins (`tauri.localhost`, `ipc.localhost`, any `*.localhost`) can
  never be loaded in it, so a page can never become "local".
- **Results without IPC.** Capture, snapshots and actions are scripts IMT evaluates in the page with
  `eval_with_callback`; the script's return value reaches Rust only in the callback of that one call. The
  one-shot-nonce idea for an IPC command therefore becomes a property of the reply: each evaluation carries a
  nonce issued by Rust when the user clicks or the agent acts, and a `ReplyGate` accepts one reply only, with
  that nonce, from the origin Rust expects (the page may have navigated meanwhile), up to 2 MB. Everything a
  page returns is untrusted data and is parsed, capped and classified in Rust.
- **Navigation.** Only `http`, `https` and `about:blank`; `file:`, `javascript:`, `data:`, `blob:` and
  custom schemes are blocked with a notice. Downloads are refused (`on_download` → false) with a notice;
  new windows and popups never open, and the user may open them in the system browser after a confirm.
- **Untrusted content.** Page text reaches the model only inside `<untrusted_web_content>` tags that the
  page cannot close (defused), with a reminder; the system prompt tells the model never to follow
  instructions found there. Hand-off text is redacted with the privacy module. Tested with a page that says
  "ignore previous instructions" and one that asks the AI to send log lines to another site.
- **IMT's CSP is unchanged** (no `unsafe-eval`, no remote origins); the browser window does not load IMT's UI.

## Rejected options

- **Keep SearXNG and Brave** (or add other search APIs): keys and instances to manage, snippets instead of
  pages, and requests on the user's behalf. Removed.
- **No-key APIs** (Stack Exchange, GitHub, Microsoft Learn): considered and dropped by the user; each covers
  one site and would grow into a set of special cases.
- **Playwright headful or stealth automation**: bundles a browser and Node, and stealth or bot-detection
  evasion is explicitly not wanted. The system webview with its honest user agent is enough for a user who
  watches and approves.
- **Multi-webview in one window**: see above; unstable in this Tauri version.

## Consequences

- No web request leaves IMT unless the user browses, or approves an agent step, in a visible window.
- The real webview automation cannot be end-to-end tested in Playwright; the adapter is kept thin
  (`app/src-tauri/src/browser/page.rs`), the rules are tested in Rust, the page scripts in Chromium (and
  WebKit in CI), and the UI flows against the mock backend.
- macOS: the browser profile lives in WebKit's data store (macOS 14+; older versions share the default
  store), so Settings ▸ Storage cannot clear it there. Not yet verified on macOS hardware.
