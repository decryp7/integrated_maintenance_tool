//! Which text encoding a source file is in: detected from its first bytes (a bounded sniff window), set by the
//! profile (`[files] encoding`) or chosen by the user for one source (re-imported with it).
//!
//! Detection order, first match wins: a byte-order mark (UTF-8, UTF-16 LE/BE); UTF-16 without a mark
//! (mostly-zero high or low bytes, or aligned UTF-16 newlines); pure ASCII and valid (or almost valid) UTF-8;
//! otherwise `chardetng` guesses among the legacy encodings of `encoding_rs` (Shift-JIS, GBK/GB18030, EUC-KR,
//! Big5, EUC-JP, Windows-125x...). Legacy encodings keep `0x0A` as a lone newline byte, so lines are split on
//! bytes exactly as for UTF-8 and file offsets stay exact. The result carries a confidence (0 to 100) and where
//! the decision came from, so the UI can say "Shift_JIS, probably" and offer a different choice.

use super::chunk::Encoding;
use crate::importers::looks_utf16le;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How many bytes of a file's start the detector looks at.
pub const SNIFF_BYTES: usize = 64 * 1024;

/// Where an encoding decision came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EncodingOrigin {
    /// A byte-order mark.
    Bom,
    /// Guessed from the content.
    Detected,
    /// Set by the profile's `[files] encoding`.
    Profile,
    /// Chosen by the user for this source.
    Manual,
}

/// The encoding a source was read with, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingInfo {
    /// Canonical name (`UTF-8`, `UTF-16LE`, `Shift_JIS`, `windows-1252`...); accepted back as a label.
    pub label: String,
    /// How sure the decision is, 0 to 100 (100 for a mark or a chosen encoding).
    pub confidence: u8,
    pub origin: EncodingOrigin,
}

/// What to read a file as: decide from its content, or a fixed encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingChoice {
    #[default]
    Auto,
    Fixed(Encoding),
}

impl EncodingChoice {
    /// Parses a profile value or a UI choice: `auto` (or empty) or a label `encoding_rs` knows.
    pub fn parse(label: &str) -> Result<EncodingChoice, String> {
        let label = label.trim();
        if label.is_empty() || label.eq_ignore_ascii_case("auto") {
            return Ok(EncodingChoice::Auto);
        }
        Encoding::from_label(label)
            .map(EncodingChoice::Fixed)
            .ok_or_else(|| format!("unknown text encoding \"{label}\"; use \"auto\" or one of the labels Settings lists (such as Shift_JIS, GBK, windows-1252)"))
    }
}

/// How the files of an import are read: the profile's default plus the user's per-source choices.
#[derive(Debug, Clone, Default)]
pub struct EncodingPolicy {
    pub default: EncodingChoice,
    /// Chosen encodings by source path (the path of the file, or `<archive>!/<entry>`).
    pub overrides: HashMap<String, Encoding>,
}

impl EncodingPolicy {
    /// The policy that detects every file's encoding (no profile default, no choices).
    pub fn auto() -> &'static EncodingPolicy {
        static AUTO: std::sync::LazyLock<EncodingPolicy> = std::sync::LazyLock::new(EncodingPolicy::default);
        &AUTO
    }

    /// What to read `source` (a source path) as, and the origin to report for a fixed choice.
    pub fn request_for(&self, source: &str) -> EncodingRequest {
        match self.overrides.get(source) {
            Some(e) => EncodingRequest {
                choice: EncodingChoice::Fixed(*e),
                origin: EncodingOrigin::Manual,
            },
            None => EncodingRequest {
                choice: self.default,
                origin: EncodingOrigin::Profile,
            },
        }
    }
}

/// One file's choice and the origin a fixed one is reported with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingRequest {
    pub choice: EncodingChoice,
    pub origin: EncodingOrigin,
}

impl Default for EncodingRequest {
    fn default() -> Self {
        EncodingRequest {
            choice: EncodingChoice::Auto,
            origin: EncodingOrigin::Detected,
        }
    }
}

/// The decision for one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub encoding: Encoding,
    /// Length of the byte-order mark to skip.
    pub bom_len: u64,
    pub confidence: u8,
    pub origin: EncodingOrigin,
}

impl Detected {
    pub fn info(&self) -> EncodingInfo {
        EncodingInfo {
            label: self.encoding.label().to_string(),
            confidence: self.confidence,
            origin: self.origin,
        }
    }
}

/// The byte-order mark at the start of `bytes`: its encoding and length.
fn bom(bytes: &[u8]) -> Option<(Encoding, u64)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((Encoding::Utf8, 3))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some((Encoding::Utf16Le, 2))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((Encoding::Utf16Be, 2))
    } else {
        None
    }
}

/// UTF-16 big-endian text without a mark whose characters are mostly ASCII (the high byte, first, is zero).
fn looks_utf16be(bytes: &[u8]) -> bool {
    let n = bytes.len().min(512) & !1;
    n >= 8 && bytes[..n].iter().step_by(2).filter(|b| **b == 0).count() * 10 > n / 2 * 9
}

/// UTF-16 without a mark in a non-Latin script, with ASCII line ends: many zero bytes in one position and the
/// newlines of one byte order on unit boundaries, none on the other.
fn utf16_by_newlines(bytes: &[u8]) -> Option<Encoding> {
    let n = bytes.len().min(4096) & !1;
    if n < 16 {
        return None;
    }
    let units = &bytes[..n];
    let le = units.as_chunks::<2>().0.iter().filter(|u| u[0] == b'\n' && u[1] == 0).count();
    let be = units.as_chunks::<2>().0.iter().filter(|u| u[0] == 0 && u[1] == b'\n').count();
    // A text whose newlines are all `0A 00` (or `00 0A`) units on unit boundaries. Legacy text has no NUL bytes
    // at all, so a few aligned units are enough; stray `0A` bytes in the other position (a character that happens
    // to contain one) are tolerated in small numbers.
    let odd_newlines = units.iter().enumerate().filter(|(i, b)| **b == b'\n' && i % 2 == 1).count();
    let even_newlines = units.iter().enumerate().filter(|(i, b)| **b == b'\n' && i % 2 == 0).count();
    if le >= 2 && odd_newlines * 4 <= le && even_newlines <= le * 4 {
        Some(Encoding::Utf16Le)
    } else if be >= 2 && even_newlines * 4 <= be && odd_newlines <= be * 4 {
        Some(Encoding::Utf16Be)
    } else {
        None
    }
}

/// Whether `window` is UTF-8 up to a character cut by the end of the window; otherwise the share of invalid
/// sequences (per mille).
fn utf8_flaws(window: &[u8]) -> u32 {
    match std::str::from_utf8(window) {
        Ok(_) => 0,
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > window.len() => 0,
        Err(_) => {
            let lossy = String::from_utf8_lossy(window);
            let bad = lossy.chars().filter(|c| *c == '\u{FFFD}').count();
            let total = lossy.chars().count().max(1);
            (bad * 1000 / total).max(1) as u32
        }
    }
}

/// Decides the encoding of a file from its first bytes (`head`; `whole` when that is the entire file).
pub fn detect(head: &[u8], whole: bool, request: EncodingRequest) -> Detected {
    if let EncodingChoice::Fixed(encoding) = request.choice {
        let bom_len = match bom(head) {
            Some((e, n)) if e == encoding => n,
            _ => 0,
        };
        return Detected {
            encoding,
            bom_len,
            confidence: 100,
            origin: request.origin,
        };
    }
    if let Some((encoding, bom_len)) = bom(head) {
        return Detected {
            encoding,
            bom_len,
            confidence: 100,
            origin: EncodingOrigin::Bom,
        };
    }
    let guessed = |encoding, confidence| Detected {
        encoding,
        bom_len: 0,
        confidence,
        origin: EncodingOrigin::Detected,
    };
    if looks_utf16le(head) {
        return guessed(Encoding::Utf16Le, 95);
    }
    if looks_utf16be(head) {
        return guessed(Encoding::Utf16Be, 95);
    }
    if let Some(e) = utf16_by_newlines(head) {
        return guessed(e, 70);
    }
    let cut = head.len() > SNIFF_BYTES;
    let window = &head[..head.len().min(SNIFF_BYTES)];
    if window.is_ascii() {
        return guessed(Encoding::Utf8, 100);
    }
    match utf8_flaws(window) {
        0 => return guessed(Encoding::Utf8, 99),
        // A few stray bytes in otherwise UTF-8 text: keep UTF-8 (the stray ones become U+FFFD).
        1..=5 => return guessed(Encoding::Utf8, 70),
        _ => {}
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(window, whole && !cut);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
    let non_ascii = window.iter().filter(|b| !b.is_ascii()).count();
    // More non-ASCII evidence, more certainty; a guess is never presented as certain.
    let confidence = (45 + non_ascii.min(100) / 2).min(95) as u8;
    guessed(Encoding::from_rs(encoding), confidence)
}

/// A choice offered to the user: its label and a readable name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingOption {
    pub label: String,
    /// English name with the languages it is used for.
    pub name: String,
}

/// The encodings Settings and the source menu offer (any label `encoding_rs` knows is also accepted).
pub fn options() -> Vec<EncodingOption> {
    const LIST: &[(&str, &str)] = &[
        ("UTF-8", "Unicode (UTF-8)"),
        ("UTF-16LE", "Unicode (UTF-16 little-endian)"),
        ("UTF-16BE", "Unicode (UTF-16 big-endian)"),
        ("windows-1252", "Western European (Windows-1252)"),
        ("windows-1250", "Central European (Windows-1250)"),
        ("windows-1251", "Cyrillic (Windows-1251)"),
        ("windows-1253", "Greek (Windows-1253)"),
        ("windows-1254", "Turkish (Windows-1254)"),
        ("windows-1255", "Hebrew (Windows-1255)"),
        ("windows-1256", "Arabic (Windows-1256)"),
        ("windows-1257", "Baltic (Windows-1257)"),
        ("windows-1258", "Vietnamese (Windows-1258)"),
        ("Shift_JIS", "Japanese (Shift-JIS)"),
        ("EUC-JP", "Japanese (EUC-JP)"),
        ("GBK", "Simplified Chinese (GBK)"),
        ("gb18030", "Simplified Chinese (GB18030)"),
        ("Big5", "Traditional Chinese (Big5)"),
        ("EUC-KR", "Korean (EUC-KR)"),
        ("KOI8-R", "Russian (KOI8-R)"),
        ("ISO-8859-2", "Central European (ISO-8859-2)"),
        ("ISO-8859-15", "Western European (ISO-8859-15)"),
    ];
    LIST.iter()
        .map(|(label, name)| EncodingOption {
            label: (*label).into(),
            name: (*name).into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(bytes: &[u8]) -> Detected {
        detect(bytes, true, EncodingRequest::default())
    }

    fn encode(label: &str, text: &str) -> Vec<u8> {
        encoding_rs::Encoding::for_label(label.as_bytes())
            .unwrap()
            .encode(text)
            .0
            .into_owned()
    }

    #[test]
    fn marks_decide_with_full_confidence() {
        let d = auto(b"\xEF\xBB\xBFhello");
        assert_eq!(
            (d.encoding, d.bom_len, d.confidence, d.origin),
            (Encoding::Utf8, 3, 100, EncodingOrigin::Bom)
        );
        assert_eq!(auto(b"\xFF\xFEa\0").encoding, Encoding::Utf16Le);
        assert_eq!(auto(b"\xFE\xFF\0a").encoding, Encoding::Utf16Be);
    }

    #[test]
    fn utf16_without_a_mark_is_found_in_both_orders_and_in_cjk_text() {
        let text = "2026-10-14 10:00:00 INFO started\n";
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(auto(&le).encoding, Encoding::Utf16Le);
        assert_eq!(auto(&be).encoding, Encoding::Utf16Be);
        // Mostly non-Latin: found by its aligned newlines.
        let cjk = "起動しました。設定を読み込んでいます。\n接続に失敗しました。再試行します。\n完了\n";
        let le: Vec<u8> = cjk.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = cjk.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let (l, b) = (auto(&le), auto(&be));
        assert_eq!((l.encoding, b.encoding), (Encoding::Utf16Le, Encoding::Utf16Be));
        assert!(l.confidence < 100 && l.origin == EncodingOrigin::Detected);
    }

    #[test]
    fn ascii_and_utf8_stay_utf8_even_when_the_window_cuts_a_character() {
        assert_eq!(
            (auto(b"plain ascii").encoding, auto(b"plain ascii").confidence),
            (Encoding::Utf8, 100)
        );
        let text = "caf\u{e9} \u{65e5}\u{672c}\n";
        assert_eq!(auto(text.as_bytes()).encoding, Encoding::Utf8);
        let bytes = text.as_bytes();
        let cut = &bytes[..bytes.len() - 2];
        assert_eq!(detect(cut, false, EncodingRequest::default()).encoding, Encoding::Utf8);
    }

    #[test]
    fn stray_bytes_in_utf8_text_do_not_make_it_legacy() {
        let mut bytes = "ordinary \u{65e5}\u{672c}\u{8a9e} text, ".repeat(400).into_bytes();
        bytes[100] = 0xFF;
        let d = auto(&bytes);
        assert_eq!(d.encoding, Encoding::Utf8);
        assert!(d.confidence < 99);
    }

    #[test]
    fn legacy_encodings_are_guessed_from_invented_text() {
        let samples: [(&str, &str, &str); 5] = [
            (
                "Shift_JIS",
                "Shift_JIS",
                "接続に失敗しました。サーバーが応答しません。再試行します。設定ファイルを読み込んでいます。",
            ),
            (
                "GBK",
                "GBK",
                "连接失败，服务器没有响应。正在重试。正在读取配置文件，请稍候。订单处理完成。",
            ),
            (
                "EUC-KR",
                "EUC-KR",
                "연결에 실패했습니다. 서버가 응답하지 않습니다. 다시 시도합니다. 설정 파일을 읽는 중입니다.",
            ),
            (
                "Big5",
                "Big5",
                "連線失敗，伺服器沒有回應。正在重試。正在讀取設定檔案，請稍候。訂單處理完成。",
            ),
            (
                "windows-1251",
                "windows-1251",
                "Не удалось подключиться к серверу. Повторная попытка. Чтение файла настроек. Заказ обработан.",
            ),
        ];
        for (label, expect, text) in samples {
            let line = format!("2026-10-14 10:00:00 INFO {text}\n");
            let bytes = encode(label, &line.repeat(20));
            let d = auto(&bytes);
            assert_eq!(d.encoding.label(), expect, "{label}");
            assert_eq!(d.origin, EncodingOrigin::Detected);
            assert!(d.confidence >= 50 && d.confidence <= 95, "{label}: {}", d.confidence);
            let mut out = String::new();
            d.encoding
                .decode_into(&bytes[..bytes.iter().position(|b| *b == b'\n').unwrap()], &mut out);
            assert_eq!(out, line.trim_end());
        }
    }

    #[test]
    fn a_fixed_choice_wins_over_the_content_and_skips_a_matching_mark() {
        let req = EncodingRequest {
            choice: EncodingChoice::Fixed(Encoding::from_label("Shift_JIS").unwrap()),
            origin: EncodingOrigin::Manual,
        };
        let d = detect(b"plain", true, req);
        assert_eq!(
            (d.encoding.label(), d.confidence, d.origin, d.bom_len),
            ("Shift_JIS", 100, EncodingOrigin::Manual, 0)
        );
        let utf8 = EncodingRequest {
            choice: EncodingChoice::Fixed(Encoding::Utf8),
            origin: EncodingOrigin::Profile,
        };
        assert_eq!(detect(b"\xEF\xBB\xBFx", true, utf8).bom_len, 3);
        assert_eq!(detect(b"x", true, utf8).bom_len, 0);
    }

    #[test]
    fn choices_parse_labels_and_refuse_unknown_ones_with_a_hint() {
        assert_eq!(EncodingChoice::parse(" Auto "), Ok(EncodingChoice::Auto));
        assert_eq!(EncodingChoice::parse(""), Ok(EncodingChoice::Auto));
        assert_eq!(
            EncodingChoice::parse("sjis").map(|c| matches!(c, EncodingChoice::Fixed(_))),
            Ok(true)
        );
        assert_eq!(EncodingChoice::parse("utf-16le"), Ok(EncodingChoice::Fixed(Encoding::Utf16Le)));
        let err = EncodingChoice::parse("klingon").unwrap_err();
        assert!(err.contains("klingon") && err.contains("auto"));
    }

    #[test]
    fn the_policy_prefers_a_sources_override_over_the_profile_default() {
        let sjis = Encoding::from_label("shift_jis").unwrap();
        let mut policy = EncodingPolicy {
            default: EncodingChoice::Fixed(Encoding::Utf8),
            ..EncodingPolicy::default()
        };
        policy.overrides.insert("/a.log".into(), sjis);
        assert_eq!(policy.request_for("/a.log").origin, EncodingOrigin::Manual);
        assert_eq!(policy.request_for("/b.log").origin, EncodingOrigin::Profile);
    }

    #[test]
    fn every_offered_option_is_a_known_label_and_round_trips() {
        for o in options() {
            let EncodingChoice::Fixed(e) = EncodingChoice::parse(&o.label).unwrap() else {
                panic!("{}", o.label)
            };
            assert_eq!(e.label(), o.label);
        }
    }

    #[test]
    fn detection_never_panics_on_odd_input() {
        for len in 0..40usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
            let _ = detect(&bytes, true, EncodingRequest::default());
            let _ = detect(&bytes, false, EncodingRequest::default());
        }
    }
}
