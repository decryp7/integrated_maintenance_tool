//! The fonts used for report text, embedded in the PDF so any script they
//! cover prints and stays searchable.
//!
//! The fonts are reduced builds of the UI's IBM Plex Sans (regular, bold,
//! italic) and JetBrains Mono, both SIL OFL 1.1, vendored in `fonts/` with
//! their licences and the script that makes them (`fonts/build-fonts.sh`).
//! They cover Latin (incl. Extended-A/B and Vietnamese), Greek, Cyrillic,
//! punctuation, currency, arrows, maths, box drawing and geometric shapes.
//!
//! Each document embeds only the glyphs it uses ([`Subsets`]), as a
//! `CIDFontType2` font with `Identity-H` encoding and a `ToUnicode` CMap, so
//! copied or extracted text comes back as the original characters.

use anyhow::{anyhow, Result};
use pdf_writer::types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap};
use pdf_writer::{Filter, Finish, Name, Pdf, Rect, Ref, Str};
use skrifa::charmap::Charmap;
use skrifa::metrics::{GlyphMetrics, Metrics};
use skrifa::prelude::{FontRef, LocationRef, MetadataProvider, Size};
use skrifa::GlyphId;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use subsetter::GlyphRemapper;

/// What a character the fonts lack prints as (and extracts as): U+FFFD.
/// CJK, emoji and other scripts outside the fonts' coverage print as this
/// replacement glyph, one per character; invisible format characters
/// (joiners, variation selectors, emoji modifiers) are dropped instead.
pub const REPLACEMENT: char = '\u{FFFD}';

/// A report font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Regular,
    Bold,
    Italic,
    Mono,
}

/// A glyph of an embedded font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Glyph id in the bundled font.
    pub id: u16,
    /// Advance width in 1/1000 of the font size.
    pub advance: f32,
    /// The character the glyph stands for (in text extraction).
    pub ch: char,
}

/// A bundled font, parsed once.
struct Loaded {
    face: Option<Face>,
    /// Font units to 1/1000 em.
    scale: f32,
}

/// What the PDF writer needs of a parsed font, in font units.
struct Face {
    charmap: Charmap<'static>,
    glyphs: GlyphMetrics<'static>,
    metrics: Metrics,
    weight: f32,
}

impl Face {
    fn parse(data: &'static [u8]) -> Option<Face> {
        let font = FontRef::new(data).ok()?;
        Some(Face {
            charmap: font.charmap(),
            glyphs: font.glyph_metrics(Size::unscaled(), LocationRef::default()),
            metrics: font.metrics(Size::unscaled(), LocationRef::default()),
            weight: font.attributes().weight.value(),
        })
    }

    /// The glyph of `c`, if the font has one (and it fits a CID).
    fn glyph_index(&self, c: char) -> Option<u16> {
        let id = self.charmap.map(c)?.to_u32();
        u16::try_from(id).ok().filter(|&id| id != 0)
    }

    /// Advance width of a glyph, in font units.
    fn advance(&self, id: u16) -> f32 {
        self.glyphs.advance_width(GlyphId::new(u32::from(id))).unwrap_or(0.0)
    }
}

impl Font {
    pub const ALL: [Font; 4] = [Font::Regular, Font::Bold, Font::Italic, Font::Mono];

    fn index(self) -> usize {
        self as usize
    }

    /// Resource name in the page dictionaries.
    pub fn resource(self) -> &'static [u8] {
        match self {
            Font::Regular => b"F1",
            Font::Bold => b"F2",
            Font::Italic => b"F3",
            Font::Mono => b"F4",
        }
    }

    /// PostScript name of the bundled font.
    pub fn base(self) -> &'static str {
        match self {
            Font::Regular => "IMTReportSans-Regular",
            Font::Bold => "IMTReportSans-Bold",
            Font::Italic => "IMTReportSans-Italic",
            Font::Mono => "IMTReportMono-Regular",
        }
    }

    /// The bundled TrueType file.
    pub fn data(self) -> &'static [u8] {
        match self {
            Font::Regular => include_bytes!("../fonts/IMTReportSans-Regular.ttf"),
            Font::Bold => include_bytes!("../fonts/IMTReportSans-Bold.ttf"),
            Font::Italic => include_bytes!("../fonts/IMTReportSans-Italic.ttf"),
            Font::Mono => include_bytes!("../fonts/IMTReportMono-Regular.ttf"),
        }
    }

    fn loaded(self) -> &'static Loaded {
        static FONTS: [OnceLock<Loaded>; 4] = [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
        FONTS[self.index()].get_or_init(|| load(self.data()))
    }

    /// The glyph `c` prints as, or `None` for characters that print nothing
    /// (control and invisible format characters). Tabs print as spaces and
    /// characters the font lacks as [`REPLACEMENT`] (or `?` should the font
    /// lack that too).
    pub fn glyph(self, c: char) -> Option<Glyph> {
        let c = if c == '\t' { ' ' } else { c };
        if c.is_control() || invisible(c) {
            return None;
        }
        let loaded = self.loaded();
        let found = |c: char| {
            let face = loaded.face.as_ref()?;
            let id = face.glyph_index(c)?;
            let advance = face.advance(id) * loaded.scale;
            Some(Glyph { id, advance, ch: c })
        };
        Some(found(c).or_else(|| found(REPLACEMENT)).or_else(|| found('?')).unwrap_or(Glyph {
            id: 0,
            advance: 500.0,
            ch: REPLACEMENT,
        }))
    }

    /// The glyphs `text` prints as.
    pub fn glyphs(self, text: &str) -> impl Iterator<Item = Glyph> + '_ {
        text.chars().filter_map(move |c| self.glyph(c))
    }

    /// Width of `text` at `size` points.
    pub fn width(self, text: &str, size: f32) -> f32 {
        self.glyphs(text).map(|g| g.advance).sum::<f32>() * size / 1000.0
    }
}

fn load(data: &'static [u8]) -> Loaded {
    let face = Face::parse(data);
    let scale = face.as_ref().map_or(1.0, |f| 1000.0 / f32::from(f.metrics.units_per_em.max(1)));
    Loaded { face, scale }
}

/// Default-ignorable characters, which print nothing (there is no shaping
/// to apply them): soft hyphens, joiners, directional marks, variation selectors, tag
/// characters and emoji skin-tone modifiers.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The glyphs one font has printed in a document.
#[derive(Default)]
struct Subset {
    /// Bundled glyph id to subset glyph id (= CID, `CIDToGIDMap` Identity).
    remapper: GlyphRemapper,
    /// Subset glyph id to the character it extracts as.
    unicode: BTreeMap<u16, char>,
    /// Advance per subset glyph id, in 1/1000 em.
    widths: Vec<f32>,
}

/// The glyphs a document uses, per font; written out as the embedded font
/// subsets. The number of glyphs is bounded by the bundled fonts.
pub struct Subsets {
    fonts: [Subset; 4],
}

const IDENTITY: SystemInfo = SystemInfo {
    registry: Str(b"Adobe"),
    ordering: Str(b"Identity"),
    supplement: 0,
};

impl Default for Subsets {
    fn default() -> Self {
        let fonts = Font::ALL.map(|f| {
            let notdef = f.loaded().face.as_ref().map_or(0.0, |face| face.advance(0));
            Subset {
                remapper: GlyphRemapper::new(),
                unicode: BTreeMap::new(),
                widths: vec![notdef * f.loaded().scale],
            }
        });
        Subsets { fonts }
    }
}

impl Subsets {
    /// `text` as a string of two-byte CIDs for `font` (`Identity-H`),
    /// recording the glyphs it uses.
    pub fn encode(&mut self, font: Font, text: &str) -> Vec<u8> {
        let subset = &mut self.fonts[font.index()];
        let mut out = Vec::with_capacity(text.len() * 2);
        for g in font.glyphs(text) {
            let cid = subset.remapper.remap(g.id);
            if usize::from(cid) == subset.widths.len() {
                subset.widths.push(g.advance);
            }
            subset.unicode.entry(cid).or_insert(g.ch);
            out.extend(cid.to_be_bytes());
        }
        out
    }

    /// The fonts that printed at least one glyph.
    pub fn used(&self) -> Vec<Font> {
        Font::ALL
            .into_iter()
            .filter(|f| self.fonts[f.index()].remapper.num_gids() > 1)
            .collect()
    }

    /// Writes the used fonts (`Type0` → `CIDFontType2` with the subset font
    /// file and a `ToUnicode` CMap) and returns their references.
    pub fn write(&self, pdf: &mut Pdf, alloc: &mut Ref) -> Result<Vec<(Font, Ref)>> {
        let mut refs = Vec::new();
        for font in self.used() {
            let subset = &self.fonts[font.index()];
            let face = font
                .loaded()
                .face
                .as_ref()
                .ok_or_else(|| anyhow!("the bundled font {} is unreadable", font.base()))?;
            let scale = font.loaded().scale;
            let data = subsetter::subset(font.data(), 0, &subset.remapper)
                .map_err(|e| anyhow!("could not subset the bundled font {}: {e:?}", font.base()))?;
            let [type0, cid, descriptor, file, cmap] = [(); 5].map(|_| alloc.bump());
            let name = format!("{}+{}", tag(&subset.remapper), font.base());
            let name = Name(name.as_bytes());

            pdf.type0_font(type0)
                .base_font(name)
                .encoding_predefined(Name(b"Identity-H"))
                .descendant_font(cid)
                .to_unicode(cmap);
            let mut c = pdf.cid_font(cid);
            c.subtype(CidFontType::Type2)
                .base_font(name)
                .system_info(IDENTITY)
                .font_descriptor(descriptor)
                .default_width(0.0)
                .cid_to_gid_map_predefined(Name(b"Identity"));
            c.widths().consecutive(0, subset.widths.iter().copied());
            c.finish();

            let mut flags = FontFlags::NON_SYMBOLIC;
            if font == Font::Mono {
                flags |= FontFlags::FIXED_PITCH;
            }
            if font == Font::Italic {
                flags |= FontFlags::ITALIC;
            }
            let m = &face.metrics;
            let bbox = m.bounds.map_or(Rect::new(0.0, m.descent, 1000.0 / scale, m.ascent), |b| {
                Rect::new(b.x_min, b.y_min, b.x_max, b.y_max)
            });
            let s = |v: f32| v * scale;
            pdf.font_descriptor(descriptor)
                .name(name)
                .flags(flags)
                .bbox(Rect::new(s(bbox.x1), s(bbox.y1), s(bbox.x2), s(bbox.y2)))
                .italic_angle(m.italic_angle)
                .ascent(s(m.ascent))
                .descent(s(m.descent))
                .cap_height(s(m.cap_height.unwrap_or(m.ascent)))
                .stem_v(10.0 + 0.244 * (face.weight - 50.0))
                .font_file2(file);
            let len = i32::try_from(data.len()).unwrap_or(i32::MAX);
            pdf.stream(file, &compress(&data))
                .filter(Filter::FlateDecode)
                .pair(Name(b"Length1"), len);

            let mut to_unicode = UnicodeCmap::new(Name(b"Custom"), IDENTITY);
            for (&gid, &ch) in &subset.unicode {
                to_unicode.pair(gid, ch);
            }
            pdf.cmap(cmap, &compress(&to_unicode.finish())).filter(Filter::FlateDecode);
            refs.push((font, type0));
        }
        Ok(refs)
    }
}

/// The six-letter subset tag, derived from the glyph set so equal subsets
/// get equal names.
fn tag(remapper: &GlyphRemapper) -> String {
    let mut h: u32 = 0x811C_9DC5;
    for g in remapper.remapped_gids() {
        for b in g.to_be_bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    }
    (0..6)
        .map(|_| {
            let c = char::from(b'A' + (h % 26) as u8);
            h /= 26;
            c
        })
        .collect()
}

pub fn compress(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_font_parses_and_covers_its_scripts() {
        for f in Font::ALL {
            assert!(f.loaded().face.is_some(), "{}", f.base());
            for c in "Az09éŁΩЖ€—…•✓\u{FFFD}".chars() {
                let g = f.glyph(c).unwrap();
                assert_eq!(g.ch, c, "{} lacks {c}", f.base());
                assert!(g.id != 0 && g.advance > 0.0);
            }
            assert!(!f.resource().is_empty());
        }
    }

    #[test]
    fn tabs_controls_missing_and_invisible_characters() {
        let f = Font::Regular;
        assert_eq!(f.glyph('\t'), f.glyph(' '));
        assert_eq!(f.glyph('\u{1}'), None);
        assert_eq!(f.glyph('\n'), None);
        for c in ['日', '😀', '\u{10FFFF}', '\u{E000}'] {
            assert_eq!(f.glyph(c).unwrap().ch, REPLACEMENT, "{c:?}");
        }
        for c in ['\u{200D}', '\u{FE0F}', '\u{1F3FD}', '\u{E0041}'] {
            assert_eq!(f.glyph(c), None, "{c:?}");
        }
        // A family with a thumbs-up, a skin tone and a joiner: two replacements.
        let text: String = f.glyphs("👍🏽\u{200D}👍").map(|g| g.ch).collect();
        assert_eq!(text, "\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn widths() {
        let mono = Font::Mono.width("abc", 10.0);
        assert!((mono - 3.0 * Font::Mono.width("i", 10.0)).abs() < 0.001 && mono > 0.0);
        assert!(Font::Bold.width("Hello", 10.0) > Font::Regular.width("Hello", 10.0));
        assert_eq!(Font::Regular.width("\u{1}\u{200B}", 10.0), 0.0);
        assert_eq!(Font::Regular.width("日", 10.0), Font::Regular.width("\u{FFFD}", 10.0));
        assert!((Font::Regular.width("\t", 10.0) - Font::Regular.width(" ", 10.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn an_unreadable_font_degrades_to_placeholders() {
        let broken = load(b"not a font");
        assert!(broken.face.is_none());
        assert_eq!(broken.scale, 1.0);
    }

    #[test]
    fn encodes_cids_and_records_widths_and_text() {
        let mut s = Subsets::default();
        assert!(s.used().is_empty());
        let a = s.encode(Font::Regular, "αβα");
        assert_eq!(a, [0, 1, 0, 2, 0, 1]);
        assert_eq!(s.encode(Font::Mono, "日本"), [0, 1, 0, 1]);
        assert_eq!(s.used(), [Font::Regular, Font::Mono]);
        let r = &s.fonts[0];
        assert_eq!(r.unicode.values().collect::<String>(), "αβ");
        assert_eq!(r.widths.len(), 3);
        assert_eq!(s.fonts[3].unicode.get(&1), Some(&REPLACEMENT));
    }

    #[test]
    fn subset_tags_are_deterministic_letters() {
        let a = tag(&GlyphRemapper::new_from_glyphs(&[3, 4]));
        assert_eq!(a, tag(&GlyphRemapper::new_from_glyphs(&[3, 4])));
        assert_ne!(a, tag(&GlyphRemapper::new_from_glyphs(&[4, 3])));
        assert!(a.len() == 6 && a.chars().all(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn writes_small_subsets() {
        let mut s = Subsets::default();
        s.encode(Font::Italic, "Ωmega");
        let mut pdf = Pdf::new();
        let mut alloc = Ref::new(1);
        let refs = s.write(&mut pdf, &mut alloc).unwrap();
        assert_eq!(refs, [(Font::Italic, Ref::new(1))]);
        let bytes = pdf.finish();
        // Five objects; the embedded file is far smaller than the bundled font.
        assert_eq!(alloc, Ref::new(6));
        assert!(bytes.len() < Font::Italic.data().len() / 4, "{}", bytes.len());
    }
}
