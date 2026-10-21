// Generated from the Rust wire types by `cargo test -p imt-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with IMT_WRITE_TYPES=1.

export type CssRect = { x: number, y: number, width: number, height: number, };

export type Placement = { 
/**
 * The page may show now: the panel's tab is active, nothing of IMT covers it and the window is not minimised.
 */
visible: boolean, 
/**
 * The page area; `None` while the panel is not laid out (closed, or hidden by the dock).
 */
rect: CssRect | null, 
/**
 * `window.devicePixelRatio` of the reporting webview.
 */
scale: number, };
