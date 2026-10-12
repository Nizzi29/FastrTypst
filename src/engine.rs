//! Embedded Typst compiler: compiles single math formulas, renders them to
//! RGBA images and decides whether two formulas typeset identically.

use std::sync::LazyLock;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;
use typst_render::RenderOptions;

/// Font size used for all formulas, in pt.
pub const FONT_SIZE: f64 = 20.0;

/// Resolution used when comparing two formulas.
const COMPARE_PX_PER_PT: f32 = 3.0;

/// Shadows DIN-style constructs with plain ones so that `d x` == `dif x`,
/// `j` == `upright(j)` and so on. Used for lenient comparisons only.
const LENIENT_PREAMBLE: &str = "#let upright(body) = body\n#let dif = $d$\n#let Dif = $D$\n";

struct Shared {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_id: FileId,
}

static SHARED: LazyLock<Shared> = LazyLock::new(|| {
    let fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    Shared {
        library: LazyHash::new(Library::default()),
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
        main_id: RootedPath::new(VirtualRoot::Project, VirtualPath::new("/main.typ").unwrap())
            .intern(),
    }
});

/// Forces font loading and library construction (call once at startup).
pub fn warm_up() {
    LazyLock::force(&SHARED);
    let _ = render("x", &Style::default());
}

struct FormulaWorld {
    source: Source,
}

impl World for FormulaWorld {
    fn library(&self) -> &LazyHash<Library> {
        &SHARED.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &SHARED.book
    }

    fn main(&self) -> FileId {
        SHARED.main_id
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == SHARED.main_id {
            Ok(self.source.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == SHARED.main_id {
            Ok(Bytes::from_string(self.source.clone()))
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        SHARED.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        Datetime::from_ymd(2026, 1, 1)
    }
}

/// How a formula should be typeset.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// Text color as `[r, g, b]`.
    pub color: [u8; 3],
    pub px_per_pt: f32,
    pub lenient: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self { color: [0, 0, 0], px_per_pt: 1.0, lenient: false }
    }
}

/// A rendered formula with premultiplied RGBA pixels.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn document_source(formula: &str, style: &Style) -> String {
    let [r, g, b] = style.color;
    format!(
        "#set page(width: auto, height: auto, margin: 4pt, fill: none)\n\
         #set text(size: {FONT_SIZE}pt, fill: rgb({r}, {g}, {b}))\n\
         {}\
         $ {formula}\n$\n",
        if style.lenient { LENIENT_PREAMBLE } else { "" },
    )
}

fn compile(formula: &str, style: &Style) -> Result<PagedDocument, String> {
    // A stray `$` would let input escape math mode and break the template.
    if formula.contains('$') {
        return Err("`$` is not needed: you are already in math mode".into());
    }
    let world = FormulaWorld {
        source: Source::new(SHARED.main_id, document_source(formula, style)),
    };
    let result = typst::compile::<PagedDocument>(&world).output;
    comemo::evict(20);
    result.map_err(|errors| {
        errors
            .first()
            .map(|e| e.message.to_string())
            .unwrap_or_else(|| "unknown error".into())
    })
}

/// Compiles and rasterizes a math formula (the content between `$ ... $`).
pub fn render(formula: &str, style: &Style) -> Result<Image, String> {
    let doc = compile(formula, style)?;
    let page = doc.pages().first().ok_or("empty document")?;
    let pixmap = typst_render::render(
        page,
        &RenderOptions { pixel_per_pt: (style.px_per_pt as f64).into(), render_bleed: false },
    );
    Ok(Image { width: pixmap.width(), height: pixmap.height(), rgba: pixmap.data().to_vec() })
}

/// Whether `input` typesets exactly like `target`. Errors if `input` does not
/// compile.
pub fn equivalent(target: &str, input: &str, lenient: bool) -> Result<bool, String> {
    let style = Style { px_per_pt: COMPARE_PX_PER_PT, lenient, ..Style::default() };
    let a = render(target, &style).map_err(|e| format!("target does not compile: {e}"))?;
    let b = render(input, &style)?;
    Ok(a.width == b.width && a.height == b.height && a.rgba == b.rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_spellings() {
        assert!(equivalent("a/b", "frac(a, b)", false).unwrap());
        assert!(equivalent("x^(2)", "x^2", false).unwrap());
        assert!(equivalent("a+b", "a + b", false).unwrap());
        assert!(!equivalent("a/b", "b/a", false).unwrap());
    }

    #[test]
    fn lenient_ignores_upright() {
        let din = "integral f(t) dif t + upright(j) omega upright(e)^x";
        let plain = "integral f(t) d t + j omega e^x";
        assert!(!equivalent(din, plain, false).unwrap());
        assert!(equivalent(din, plain, true).unwrap());
    }

    #[test]
    fn errors_are_reported() {
        assert!(render("frac(a", &Style::default()).is_err());
        assert!(render("ab", &Style::default()).is_err());
        // A comment must not swallow the closing `$` of the template.
        assert!(equivalent("a + b", "a + b // comment", false).unwrap());
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::generator::Generator;
    #[test]
    #[ignore]
    fn timing() {
        warm_up();
        let mut generator = Generator::seeded(5);
        let formulas: Vec<String> = (0..300).map(|i| generator.any((i % 10 + 1) as f32).source).collect();
        for scale in [1.0f32, 1.6, 3.0] {
            let mut worst = 0.0f64;
            let t = std::time::Instant::now();
            for f in &formulas {
                let t1 = std::time::Instant::now();
                render(f, &Style { px_per_pt: scale, ..Style::default() }).unwrap();
                worst = worst.max(t1.elapsed().as_secs_f64());
            }
            println!("px/pt {scale}: avg {:.1} ms, worst {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / formulas.len() as f64, worst * 1000.0);
        }
    }
}
