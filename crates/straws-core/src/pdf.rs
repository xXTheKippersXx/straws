//! A small PDF writer: A4 pages of text in the standard fonts (no fonts
//! embedded), lines, shapes and filled areas. Enough for reports.

use std::fmt::Write;

/// A4 in points.
pub const WIDTH: f64 = 595.28;
pub const HEIGHT: f64 = 841.89;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Regular,
    Bold,
    Mono,
}

/// An RGB colour, each 0 to 1.
pub type Rgb = [f64; 3];

pub const BLACK: Rgb = [0.13, 0.13, 0.13];
pub const GREY: Rgb = [0.45, 0.45, 0.45];
pub const LIGHT: Rgb = [0.85, 0.85, 0.85];

/// One page's drawing, in points from the bottom left.
#[derive(Default)]
pub struct Page {
    ops: String,
}

/// A document of pages.
#[derive(Default)]
pub struct Pdf {
    pages: Vec<Page>,
}

impl Pdf {
    pub fn new() -> Pdf {
        Pdf::default()
    }

    /// A new page to draw on.
    pub fn page(&mut self) -> &mut Page {
        self.pages.push(Page::default());
        self.pages.last_mut().expect("just pushed")
    }

    /// The page being drawn on (a new one if there's none yet).
    pub fn page_last(&mut self) -> &mut Page {
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        self.pages.last_mut().expect("there's a page")
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Draws on every page after the fact (for footers): `f(page, n)`.
    pub fn each_page(&mut self, mut f: impl FnMut(&mut Page, usize)) {
        for (k, p) in self.pages.iter_mut().enumerate() {
            f(p, k + 1);
        }
    }

    /// The finished file.
    pub fn finish(self, title: &str, created: &str) -> Vec<u8> {
        let n = self.pages.len().max(1);
        // Objects: 1 catalog, 2 pages, 3-6 fonts, 7 info, then a page and
        // its content stream for each page.
        let mut objects: Vec<String> = Vec::new();
        let kids: Vec<String> = (0..n).map(|k| format!("{} 0 R", 8 + 2 * k)).collect();
        objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        objects.push(format!(
            "<< /Type /Pages /Kids [{}] /Count {n} >>",
            kids.join(" ")
        ));
        for base in ["Helvetica", "Helvetica-Bold", "Courier"] {
            objects.push(format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>"
            ));
        }
        objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>".into());
        objects.push(format!(
            "<< /Title {} /Producer (STRAWS {}) /CreationDate (D:{created}) >>",
            pdf_string(title),
            env!("CARGO_PKG_VERSION")
        ));
        let empty = Page::default();
        for k in 0..n {
            let page = self.pages.get(k).unwrap_or(&empty);
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {WIDTH} {HEIGHT}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R >> >> \
                 /Contents {} 0 R >>",
                9 + 2 * k
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                page.ops.len(),
                page.ops
            ));
        }
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (k, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", k + 1).as_bytes());
            out.extend(encode_latin(o));
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
        for o in &offsets {
            let _ = writeln!(tail, "{o:010} 00000 n ");
        }
        let _ = write!(
            tail,
            "trailer\n<< /Size {} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        );
        out.extend(tail.as_bytes());
        out
    }
}

impl Page {
    fn colour(&mut self, c: Rgb, stroke: bool) {
        let _ = writeln!(
            self.ops,
            "{} {} {} {}",
            num(c[0]),
            num(c[1]),
            num(c[2]),
            if stroke { "RG" } else { "rg" }
        );
    }

    /// Text with its left end at (x, y) (the baseline).
    pub fn text(&mut self, x: f64, y: f64, size: f64, font: Font, colour: Rgb, s: &str) {
        self.colour(colour, false);
        let mut at = x;
        for (symbol, run) in runs(s) {
            let name = match (symbol, font) {
                (true, _) => "F4",
                (false, Font::Regular) => "F1",
                (false, Font::Bold) => "F2",
                (false, Font::Mono) => "F3",
            };
            let bytes: Vec<u8> = run.iter().map(|c| c.1).collect();
            let _ = writeln!(
                self.ops,
                "BT /{name} {} Tf {} {} Td {} Tj ET",
                num(size),
                num(at),
                num(y),
                pdf_bytes(&bytes)
            );
            at += run
                .iter()
                .map(|c| char_width(c.0, if symbol { None } else { Some(font) }))
                .sum::<f64>()
                * size
                / 1000.0;
        }
    }

    /// Text ending at x.
    pub fn text_right(&mut self, x: f64, y: f64, size: f64, font: Font, colour: Rgb, s: &str) {
        self.text(x - width(s, size, font), y, size, font, colour, s);
    }

    /// Text centred on x.
    pub fn text_centre(&mut self, x: f64, y: f64, size: f64, font: Font, colour: Rgb, s: &str) {
        self.text(x - width(s, size, font) / 2.0, y, size, font, colour, s);
    }

    /// Text turned a quarter anticlockwise, centred on (x, y).
    pub fn text_up(&mut self, x: f64, y: f64, size: f64, font: Font, colour: Rgb, s: &str) {
        let w = width(s, size, font);
        let _ = writeln!(self.ops, "q 0 1 -1 0 {} {} cm", num(x), num(y - w / 2.0));
        self.text(0.0, 0.0, size, font, colour, s);
        self.ops.push_str("Q\n");
    }

    pub fn line(&mut self, a: [f64; 2], b: [f64; 2], width: f64, colour: Rgb) {
        self.polyline(&[a, b], width, colour);
    }

    pub fn polyline(&mut self, points: &[[f64; 2]], width: f64, colour: Rgb) {
        if points.len() < 2 {
            return;
        }
        self.colour(colour, true);
        let _ = writeln!(self.ops, "{} w 1 J 1 j", num(width));
        for (k, p) in points.iter().enumerate() {
            let _ = writeln!(
                self.ops,
                "{} {} {}",
                num(p[0]),
                num(p[1]),
                if k == 0 { "m" } else { "l" }
            );
        }
        self.ops.push_str("S\n");
    }

    /// A rectangle, outlined and/or filled.
    pub fn rect(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        stroke: Option<(f64, Rgb)>,
        fill: Option<Rgb>,
    ) {
        if let Some(c) = fill {
            self.colour(c, false);
        }
        if let Some((lw, c)) = stroke {
            self.colour(c, true);
            let _ = writeln!(self.ops, "{} w", num(lw));
        }
        let op = match (stroke.is_some(), fill.is_some()) {
            (true, true) => "B",
            (true, false) => "S",
            (false, true) => "f",
            (false, false) => return,
        };
        let _ = writeln!(
            self.ops,
            "{} {} {} {} re {op}",
            num(x),
            num(y),
            num(w),
            num(h)
        );
    }

    /// A circle outline (four Bézier quarters).
    pub fn circle(&mut self, c: [f64; 2], r: f64, width: f64, colour: Rgb) {
        let k = 0.552_284_75 * r;
        self.colour(colour, true);
        let (x, y) = (c[0], c[1]);
        let _ = writeln!(
            self.ops,
            "{} w {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c S",
            num(width),
            num(x + r),
            num(y),
            num(x + r),
            num(y + k),
            num(x + k),
            num(y + r),
            num(x),
            num(y + r),
            num(x - k),
            num(y + r),
            num(x - r),
            num(y + k),
            num(x - r),
            num(y),
            num(x - r),
            num(y - k),
            num(x - k),
            num(y - r),
            num(x),
            num(y - r),
            num(x + k),
            num(y - r),
            num(x + r),
            num(y - k),
            num(x + r),
            num(y)
        );
    }
}

/// How wide text is in points.
pub fn width(s: &str, size: f64, font: Font) -> f64 {
    runs(s)
        .iter()
        .map(|(symbol, run)| {
            run.iter()
                .map(|c| char_width(c.0, if *symbol { None } else { Some(font) }))
                .sum::<f64>()
        })
        .sum::<f64>()
        * size
        / 1000.0
}

/// Text cut into lines no wider than `max` points (at spaces where it can).
pub fn wrap(s: &str, size: f64, font: Font, max: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let tried = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if width(&tried, size, font) <= max || line.is_empty() {
                line = tried;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            // A word too long for a line on its own is cut.
            while width(&line, size, font) > max && line.chars().count() > 1 {
                let mut cut = line.clone();
                while width(&cut, size, font) > max && cut.chars().count() > 1 {
                    cut.pop();
                }
                let rest = line[cut.len()..].to_string();
                lines.push(cut);
                line = rest;
            }
        }
        lines.push(line);
    }
    lines
}

/// The text in runs of the main font and of Symbol (Greek letters and a
/// few signs), each character with its byte.
fn runs(s: &str) -> Vec<(bool, Vec<(char, u8)>)> {
    let mut out: Vec<(bool, Vec<(char, u8)>)> = Vec::new();
    for c in s.chars() {
        let (symbol, byte) = match symbol_byte(c) {
            Some(b) => (true, b),
            None => (false, win_ansi(c)),
        };
        match out.last_mut() {
            Some((s, run)) if *s == symbol => run.push((c, byte)),
            _ => out.push((symbol, vec![(c, byte)])),
        }
    }
    out
}

/// Characters drawn from the Symbol font.
fn symbol_byte(c: char) -> Option<u8> {
    Some(match c {
        'Ω' => b'W',
        'λ' => b'l',
        'π' => b'p',
        'Γ' => b'G',
        'θ' => b'q',
        'φ' => b'f',
        'ε' => b'e',
        'σ' => b's',
        'η' => b'h',
        '√' => 0xD6,
        '≤' => 0xA3,
        '≥' => 0xB3,
        '≈' => 0xBB,
        '−' => b'-',
        _ => return None,
    })
}

/// A character's byte in WinAnsi (Windows-1252), '?' if it has none.
fn win_ansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{A0}'..='\u{FF}' => c as u32 as u8,
        '€' => 0x80,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '™' => 0x99,
        _ => b'?',
    }
}

/// Glyph widths in thousandths of the size (Adobe's metrics for the
/// standard fonts).
fn char_width(c: char, font: Option<Font>) -> f64 {
    let Some(font) = font else {
        return match c {
            'Ω' => 768.0,
            'λ' | 'π' | 'θ' | 'φ' => 549.0,
            'Γ' => 603.0,
            'ε' => 439.0,
            'σ' => 603.0,
            'η' => 603.0,
            '√' => 549.0,
            '≤' | '≥' | '≈' => 549.0,
            '−' => 549.0,
            _ => 500.0,
        };
    };
    if font == Font::Mono {
        return 600.0;
    }
    const REGULAR: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
        722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
        556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
        500, 334, 260, 334, 584,
    ];
    const BOLD: [u16; 95] = [
        278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722,
        722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611,
        611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556,
        500, 389, 280, 389, 584,
    ];
    let table = if font == Font::Bold { &BOLD } else { &REGULAR };
    match c {
        ' '..='~' => f64::from(table[c as usize - 32]),
        '°' => 400.0,
        '×' | '±' => 584.0,
        'µ' => 556.0,
        '–' => 556.0,
        '—' | '…' => 1000.0,
        '•' => 350.0,
        '‘' | '’' => 222.0,
        '“' | '”' => 333.0,
        'é' | 'è' | 'ê' | 'ë' | 'á' | 'à' | 'â' | 'ä' | 'ó' | 'ö' | 'ü' | 'ú' => 556.0,
        _ => 556.0,
    }
}

/// A number for the page description, short and exact enough.
fn num(v: f64) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// A PDF literal string of these bytes.
fn pdf_bytes(bytes: &[u8]) -> String {
    let mut out = String::from("(");
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(')');
    out
}

/// A PDF text string (for the document's title).
fn pdf_string(s: &str) -> String {
    pdf_bytes(&s.chars().map(win_ansi).collect::<Vec<u8>>())
}

/// The file's text as bytes (everything written is ASCII by now).
fn encode_latin(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_has_its_pages_and_a_good_index() {
        let mut pdf = Pdf::new();
        let p = pdf.page();
        p.text(
            50.0,
            800.0,
            12.0,
            Font::Bold,
            BLACK,
            "Feed (50 Ω) at 1/4 λ: 30° (test)",
        );
        p.polyline(&[[50.0, 50.0], [100.0, 80.0], [150.0, 60.0]], 1.0, BLACK);
        p.circle([300.0, 400.0], 50.0, 0.5, GREY);
        pdf.page()
            .text_right(545.0, 40.0, 9.0, Font::Mono, GREY, "page 2");
        let bytes = pdf.finish("A (test)", "20261027120000");
        // One character a byte, so offsets stay byte offsets.
        let text: String = bytes
            .iter()
            .map(|&b| if b.is_ascii() { b as char } else { '?' })
            .collect();
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert!(text.contains("(W)"), "Omega from Symbol");
        assert!(text.contains("\\260"), "degree sign in WinAnsi");
        assert!(text.contains("\\(test\\)"));
        // Every object starts where the index says.
        let xref = text.rfind("startxref\n").unwrap();
        let at: usize = text[xref + 10..].lines().next().unwrap().parse().unwrap();
        assert!(text[at..].starts_with("xref"));
        for (k, line) in text[at..].lines().skip(3).take(10).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(
                text[off..].starts_with(&format!("{} 0 obj", k + 1)),
                "object {}",
                k + 1
            );
        }
    }

    #[test]
    fn widths_and_wrapping() {
        // "Hello" in Helvetica 10 pt: 722+556+222+222+556 = 2278 -> 22.78.
        assert!((width("Hello", 10.0, Font::Regular) - 22.78).abs() < 1e-9);
        assert!((width("abc", 10.0, Font::Mono) - 18.0).abs() < 1e-9);
        let lines = wrap("one two three four five six", 10.0, Font::Regular, 60.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| width(l, 10.0, Font::Regular) <= 60.0));
        let long = wrap(&"x".repeat(200), 8.0, Font::Mono, 100.0);
        assert!(long.iter().all(|l| width(l, 8.0, Font::Mono) <= 100.0));
        assert_eq!(long.concat().len(), 200);
    }
}
