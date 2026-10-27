//! The design report as a PDF: key figures, a drawing of the antenna,
//! pattern cuts, SWR and impedance across the sweep, the wires and the
//! deck.

use crate::analysis::feed;
use crate::analysis::pattern::Cut;
use crate::export::{ReportInput, key_figures};
use crate::output::Segment;
use crate::pdf::{self, BLACK, Font, GREY, HEIGHT, LIGHT, Page, Pdf, WIDTH};

const MARGIN: f64 = 50.0;
const ACCENT: pdf::Rgb = [0.78, 0.35, 0.08];
const WIRE: pdf::Rgb = [0.2, 0.3, 0.45];

/// The report as a PDF file. `segments` are the antenna's segments (for
/// its drawing).
pub fn report_pdf(r: &ReportInput<'_>, segments: &[Segment]) -> Vec<u8> {
    let stamp = crate::step::timestamp();
    let date = stamp.get(..10).unwrap_or("").to_string();
    let mut doc = Pdf::new();
    let right = WIDTH - MARGIN;

    // Page 1: the antenna and its figures.
    {
        let p = doc.page();
        let mut y = HEIGHT - MARGIN - 14.0;
        p.text(MARGIN, y, 20.0, Font::Bold, BLACK, r.name);
        y -= 18.0;
        p.text(
            MARGIN,
            y,
            9.0,
            Font::Regular,
            GREY,
            &format!(
                "Antenna report from STRAWS {}, {date}",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let top = y - 28.0;
        // Key figures, left.
        let mut ty = top;
        if r.point.is_some() {
            p.text(
                MARGIN,
                ty,
                11.0,
                Font::Bold,
                BLACK,
                "At the design frequency",
            );
            ty -= 18.0;
            for (k, v) in key_figures(r) {
                p.text(MARGIN, ty, 9.0, Font::Regular, GREY, &k);
                p.text(MARGIN + 130.0, ty, 9.0, Font::Bold, BLACK, &v);
                ty -= 14.5;
            }
        } else {
            p.text(
                MARGIN,
                ty,
                9.0,
                Font::Regular,
                GREY,
                "No results yet: run a sweep first.",
            );
            ty -= 14.5;
        }
        // The antenna, right.
        let (bx, bw) = (330.0, right - 330.0);
        let bh = 210.0;
        p.text(bx, top, 11.0, Font::Bold, BLACK, "The antenna");
        model_drawing(p, bx, top - 14.0 - bh, bw, bh, segments);
        // Pattern cuts underneath.
        if let Some((el, az)) = &r.cuts {
            let cy = ty.min(top - bh - 30.0) - 150.0;
            let radius = 105.0;
            polar(
                p,
                [MARGIN + 120.0, cy],
                radius,
                el,
                "Elevation cut through the peak",
            );
            polar(
                p,
                [right - 120.0, cy],
                radius,
                az,
                "Azimuth cut at the peak's elevation",
            );
            p.text(
                MARGIN,
                cy - radius - 36.0,
                8.0,
                Font::Regular,
                GREY,
                "Pattern cuts: rings every 10 dB below the peak; angles in degrees (elevation from the horizon, azimuth from +x towards +y).",
            );
        }
    }

    // Page 2: across the sweep.
    if let Some(s) = r.sweep {
        let pick = |f: &dyn Fn(num_complex::Complex64) -> f64| -> Vec<(f64, f64)> {
            s.points()
                .filter_map(|q| q.feed(r.feed).map(|i| (q.mhz, f(i.impedance))))
                .collect()
        };
        let swr = pick(&|z| feed::swr(z, r.z0).min(20.0));
        if swr.len() > 1 {
            let p = doc.page();
            let mut y = HEIGHT - MARGIN - 14.0;
            p.text(MARGIN, y, 14.0, Font::Bold, BLACK, "Across the sweep");
            y -= 16.0;
            let h = 172.0;
            let w = right - MARGIN;
            for (title, points, label, floor) in [
                (
                    format!("SWR on {} Ω", r.z0),
                    swr,
                    "SWR".to_string(),
                    Some(1.0),
                ),
                (
                    "Resistance".to_string(),
                    pick(&|z| z.re),
                    "R (Ω)".to_string(),
                    None,
                ),
                (
                    "Reactance".to_string(),
                    pick(&|z| z.im),
                    "X (Ω)".to_string(),
                    None,
                ),
            ] {
                y -= 24.0;
                p.text(MARGIN, y, 10.5, Font::Bold, BLACK, &title);
                y -= 8.0;
                chart(
                    p,
                    MARGIN + 34.0,
                    y - h,
                    w - 34.0,
                    h,
                    &points,
                    "MHz",
                    &label,
                    floor,
                );
                y -= h + 26.0;
            }
        }
    }

    // Then the wires and the deck, over as many pages as they take.
    let mut writer = Flow::new(&mut doc);
    if !r.wires.is_empty() {
        writer.heading("Wires");
        let cols: [(&str, f64); 10] = [
            ("Wire", 30.0),
            ("Segs", 30.0),
            ("X1", 50.0),
            ("Y1", 50.0),
            ("Z1", 50.0),
            ("X2", 50.0),
            ("Y2", 50.0),
            ("Z2", 50.0),
            ("Dia. mm", 55.0),
            ("Length", 55.0),
        ];
        let header = |p: &mut Page, y: f64| {
            let mut x = MARGIN;
            for (name, w) in cols {
                x += w;
                p.text_right(x, y, 8.0, Font::Bold, BLACK, name);
            }
            p.line([MARGIN, y - 3.0], [x, y - 3.0], 0.5, LIGHT);
        };
        writer.need(14.0);
        let y = writer.y;
        header(writer.page(), y);
        writer.y -= 13.0;
        let lines: Vec<&str> = r.deck_text.lines().collect();
        let mut summarised: Vec<(String, String)> = Vec::new();
        for wire in r.wires {
            // Generated sets (a dish's mesh, radials, a curve) get one row.
            if let Some(key) = lines.get(wire.line).and_then(|l| set_key(l)) {
                if summarised.contains(&key) {
                    continue;
                }
                let count = r
                    .wires
                    .iter()
                    .filter(|w| lines.get(w.line).and_then(|l| set_key(l)).as_ref() == Some(&key))
                    .count();
                writer.need(11.0);
                let y = writer.y;
                writer.page().text(
                    MARGIN + 8.0,
                    y,
                    8.0,
                    Font::Regular,
                    GREY,
                    &format!(
                        "{} ({}): {count} wire{}, all in the deck file",
                        key.1,
                        key.0,
                        if count == 1 { "" } else { "s" }
                    ),
                );
                writer.y -= 11.0;
                summarised.push(key);
                continue;
            }
            if writer.need(11.0) {
                let y = writer.y;
                header(writer.page(), y);
                writer.y -= 13.0;
            }
            let cells = [
                wire.tag.to_string(),
                wire.segments.to_string(),
                format!("{:.4}", wire.a[0]),
                format!("{:.4}", wire.a[1]),
                format!("{:.4}", wire.a[2]),
                format!("{:.4}", wire.b[0]),
                format!("{:.4}", wire.b[1]),
                format!("{:.4}", wire.b[2]),
                format!("{:.3}", wire.radius * 2000.0),
                format!("{:.4}", wire.length()),
            ];
            let y = writer.y;
            let p = writer.page();
            let mut x = MARGIN;
            for ((_, w), c) in cols.iter().zip(&cells) {
                x += w;
                p.text_right(x, y, 8.0, Font::Regular, BLACK, c);
            }
            writer.y -= 11.0;
        }
        writer.y -= 4.0;
        writer.para(
            "Positions and lengths in metres.",
            8.0,
            Font::Regular,
            GREY,
            11.0,
        );
        writer.y -= 10.0;
    }
    writer.heading("Deck");
    let deck = deck_summary(&r.deck_text.replace('\r', ""));
    for line in pdf::wrap(deck.trim_end(), 7.5, Font::Mono, right - MARGIN) {
        writer.need(9.5);
        let y = writer.y;
        writer.page().text(MARGIN, y, 7.5, Font::Mono, BLACK, &line);
        writer.y -= 9.5;
    }

    // Footers.
    let n = doc.page_count();
    let name = r.name.to_string();
    doc.each_page(|p, k| {
        p.line([MARGIN, 40.0], [right, 40.0], 0.5, LIGHT);
        p.text(MARGIN, 28.0, 8.0, Font::Regular, GREY, &name);
        p.text_right(
            right,
            28.0,
            8.0,
            Font::Regular,
            GREY,
            &format!("page {k} of {n}"),
        );
    });
    let created: String = stamp.chars().filter(char::is_ascii_digit).collect();
    doc.finish(r.name, &format!("{created}Z"))
}

/// What generated set a deck line belongs to, from its marker comment:
/// (kind, name), e.g. ("reflector", "dish1").
fn set_key(line: &str) -> Option<(String, String)> {
    let at = line.find("straws: ")?;
    let mut words = line[at + 8..].split_whitespace();
    let kind = words.next()?;
    let name = match kind {
        "reflector" | "plate" | "corner" | "grid" | "radials" | "ring" | "curve" => words.next()?,
        "buried" => "on or under the ground",
        _ => return None,
    };
    Some((kind.to_string(), name.to_string()))
}

/// The deck with each generated set's cards (three or more lines) shown
/// as one line saying what they are, long runs of one kind of card cut
/// short, and tabs as spaces.
fn deck_summary(deck: &str) -> String {
    let expanded: Vec<String> = deck.lines().map(untab).collect();
    let lines: Vec<&str> = expanded.iter().map(String::as_str).collect();
    let mut out = String::new();
    let mut k = 0;
    while k < lines.len() {
        let key = set_key(lines[k]);
        let mut end = k + 1;
        if key.is_some() {
            while end < lines.len() && set_key(lines[end]) == key {
                end += 1;
            }
        }
        match key {
            Some((kind, name)) if end - k >= 3 => {
                out.push_str(&format!(
                    "' {name} ({kind}): {} cards, all in the deck file\n",
                    end - k
                ));
            }
            _ => {
                // A long run of one card (LD for every wire, say): the first
                // few and the last.
                let card = |l: &str| l.split_whitespace().next().map(str::to_ascii_uppercase);
                let kind = card(lines[k]).filter(|c| c.len() == 2 && !c.starts_with('\''));
                let mut run = end;
                if kind.is_some() && set_key(lines[k]).is_none() {
                    while run < lines.len()
                        && card(lines[run]) == kind
                        && set_key(lines[run]).is_none()
                    {
                        run += 1;
                    }
                }
                if run - k > 40 {
                    for l in &lines[k..k + 3] {
                        out.push_str(l);
                        out.push('\n');
                    }
                    out.push_str(&format!(
                        "' ... {} more {} cards, all in the deck file ...\n",
                        run - k - 4,
                        kind.unwrap_or_default()
                    ));
                    out.push_str(lines[run - 1]);
                    out.push('\n');
                    end = run;
                } else {
                    for l in &lines[k..end] {
                        out.push_str(l);
                        out.push('\n');
                    }
                }
            }
        }
        k = end;
    }
    out
}

/// Tabs as spaces, to the next multiple of eight columns.
fn untab(line: &str) -> String {
    let mut out = String::new();
    for c in line.chars() {
        if c == '\t' {
            out.push(' ');
            while !out.chars().count().is_multiple_of(8) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Text flowing down pages, starting a new page when one is full.
struct Flow<'a> {
    doc: &'a mut Pdf,
    y: f64,
    started: bool,
}

impl<'a> Flow<'a> {
    fn new(doc: &'a mut Pdf) -> Flow<'a> {
        Flow {
            doc,
            y: 0.0,
            started: false,
        }
    }

    /// Makes room for `h` points; true when that took a new page.
    fn need(&mut self, h: f64) -> bool {
        if !self.started || self.y - h < MARGIN + 10.0 {
            self.doc.page();
            self.y = HEIGHT - MARGIN - 10.0;
            self.started = true;
            return true;
        }
        false
    }

    fn page(&mut self) -> &mut Page {
        self.doc.page_last()
    }

    fn heading(&mut self, s: &str) {
        self.need(40.0);
        let y = self.y;
        self.page().text(MARGIN, y, 14.0, Font::Bold, BLACK, s);
        self.y -= 22.0;
    }

    fn para(&mut self, s: &str, size: f64, font: Font, colour: pdf::Rgb, step: f64) {
        for line in pdf::wrap(s, size, font, WIDTH - 2.0 * MARGIN) {
            self.need(step);
            let y = self.y;
            self.page().text(MARGIN, y, size, font, colour, &line);
            self.y -= step;
        }
    }
}

/// A line chart in the box (x, y) to (x + w, y + h).
#[allow(clippy::too_many_arguments)]
fn chart(
    p: &mut Page,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    points: &[(f64, f64)],
    x_label: &str,
    y_label: &str,
    y_floor: Option<f64>,
) {
    if points.len() < 2 {
        return;
    }
    let (x0, x1) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q.0), a.1.max(q.0)));
    let (mut y0, mut y1) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q.1), a.1.max(q.1)));
    if let Some(f) = y_floor {
        y0 = y0.min(f);
    }
    if y1 - y0 < 1e-9 {
        y1 = y0 + 1.0;
    }
    let sx = |v: f64| x + (v - x0) / (x1 - x0).max(1e-12) * w;
    let sy = |v: f64| y + (v - y0) / (y1 - y0) * h;
    let (ystep, xstep) = (nice_step((y1 - y0) / 5.0), nice_step((x1 - x0) / 5.0));
    for fy in ticks(y0, y1, ystep) {
        p.line([x, sy(fy)], [x + w, sy(fy)], 0.4, LIGHT);
        p.text_right(
            x - 4.0,
            sy(fy) - 3.0,
            8.0,
            Font::Regular,
            GREY,
            &tick(fy, ystep),
        );
    }
    for fx in ticks(x0, x1, xstep) {
        p.line([sx(fx), y], [sx(fx), y + h], 0.4, LIGHT);
        p.text_centre(sx(fx), y - 12.0, 8.0, Font::Regular, GREY, &tick(fx, xstep));
    }
    p.rect(x, y, w, h, Some((0.6, GREY)), None);
    let pts: Vec<[f64; 2]> = points.iter().map(|q| [sx(q.0), sy(q.1)]).collect();
    p.polyline(&pts, 1.4, ACCENT);
    p.text_centre(x + w / 2.0, y - 24.0, 8.5, Font::Regular, GREY, x_label);
    p.text_up(x - 30.0, y + h / 2.0, 8.5, Font::Regular, GREY, y_label);
}

/// A round step near `raw`: 1, 2, 2.5 or 5 times a power of ten.
fn nice_step(raw: f64) -> f64 {
    if raw.is_nan() || raw <= 0.0 || raw.is_infinite() {
        return 1.0;
    }
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    let n = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 2.5 {
        2.5
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * p
}

/// The multiples of `step` from `lo` to `hi`.
fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let first = (lo / step - 1e-9).ceil() as i64;
    let last = (hi / step + 1e-9).floor() as i64;
    (first..=last).map(|k| k as f64 * step).collect()
}

/// A tick label with as many decimals as the step needs.
fn tick(v: f64, step: f64) -> String {
    let mut places = 0;
    while places < 6 && (step * 10f64.powi(places)).fract().abs() > 1e-6 {
        places += 1;
    }
    let places = places as usize;
    let s = format!("{v:.places$}");
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        format!("{:.places$}", 0.0)
    } else {
        s
    }
}

/// A polar plot of a cut, 40 dB deep, centred at c.
fn polar(p: &mut Page, c: [f64; 2], r: f64, cut: &Cut, title: &str) {
    let peak = cut.points.iter().map(|q| q.1).fold(f64::MIN, f64::max);
    let range = 40.0;
    p.text_centre(c[0], c[1] + r + 22.0, 9.5, Font::Bold, BLACK, title);
    for k in 1..=4 {
        p.circle(c, r * k as f64 / 4.0, 0.4, LIGHT);
    }
    for k in 0..12 {
        let a = (k as f64 * 30.0).to_radians();
        p.line(c, [c[0] + r * a.cos(), c[1] + r * a.sin()], 0.3, LIGHT);
        let label = format!("{}°", k * 30);
        let (lx, ly) = (
            c[0] + (r + 10.0) * a.cos(),
            c[1] + (r + 10.0) * a.sin() - 3.0,
        );
        p.text_centre(lx, ly, 7.0, Font::Regular, GREY, &label);
    }
    for k in 1..4 {
        p.text(
            c[0] + 2.0,
            c[1] + r * k as f64 / 4.0 + 2.0,
            6.5,
            Font::Regular,
            GREY,
            &format!("-{}", 10 * (4 - k)),
        );
    }
    let pts: Vec<[f64; 2]> = cut
        .points
        .iter()
        .map(|&(angle, g)| {
            let u = ((g - (peak - range)) / range).clamp(0.0, 1.0);
            let a = angle.to_radians();
            [c[0] + a.cos() * r * u, c[1] + a.sin() * r * u]
        })
        .collect();
    p.polyline(&pts, 1.4, ACCENT);
    p.text_centre(
        c[0],
        c[1] - r - 22.0,
        8.0,
        Font::Regular,
        GREY,
        &format!("peak {peak:.2} dBi"),
    );
}

/// The wires seen from above one corner (isometric), fitted into the box.
fn model_drawing(p: &mut Page, x: f64, y: f64, w: f64, h: f64, segments: &[Segment]) {
    p.rect(x, y, w, h, Some((0.5, LIGHT)), None);
    if segments.is_empty() {
        p.text_centre(
            x + w / 2.0,
            y + h / 2.0,
            9.0,
            Font::Regular,
            GREY,
            "No wires yet.",
        );
        return;
    }
    let (c30, s30) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let project = |q: [f64; 3]| [(q[0] - q[1]) * c30, q[2] + (q[0] + q[1]) * s30];
    let lines: Vec<([f64; 2], [f64; 2])> = segments
        .iter()
        .map(|s| {
            let (a, b) = s.ends();
            (project(a), project(b))
        })
        .collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (a, b) in &lines {
        for q in [a, b] {
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
    }
    let pad = 14.0;
    let span = [(hi[0] - lo[0]).max(1e-9), (hi[1] - lo[1]).max(1e-9)];
    let k = ((w - 2.0 * pad) / span[0]).min((h - 2.0 * pad) / span[1]);
    let ox = x + w / 2.0 - k * (lo[0] + hi[0]) / 2.0;
    let oy = y + h / 2.0 - k * (lo[1] + hi[1]) / 2.0;
    let place = |q: [f64; 2]| [ox + k * q[0], oy + k * q[1]];
    for (a, b) in &lines {
        p.line(place(*a), place(*b), 0.7, WIRE);
    }
    // Which way is which.
    let o = [x + 18.0, y + 14.0];
    for (axis, name) in [
        ([1.0, 0.0, 0.0], "x"),
        ([0.0, 1.0, 0.0], "y"),
        ([0.0, 0.0, 1.0], "z"),
    ] {
        let d = project(axis);
        let tip = [o[0] + 12.0 * d[0], o[1] + 12.0 * d[1]];
        p.line(o, tip, 0.6, GREY);
        p.text(tip[0] + 1.5, tip[1] + 1.0, 6.5, Font::Regular, GREY, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::plan::FrequencyPlan;
    use crate::analysis::sweep::{FreqPoint, SweepResults};
    use crate::output::InputParams;
    use num_complex::Complex64;

    #[test]
    fn generated_sets_are_summed_up() {
        let deck = "CE\nGW 1 5 0 0 0 0 0 1 .001 ' driven\n\
                    GW 2 1 a b c d e f g ' straws: reflector dish1 rect dish 3x3\n\
                    GW 2 1 a b c d e f g ' straws: reflector dish1 rect dish 3x3\n\
                    GW 3 1 a b c d e f g ' straws: reflector dish1 rect dish 3x3\n\
                    GW 4 1 a b c d e f g ' straws: radials radials1\nGE 0\nEN\n";
        let out = deck_summary(deck);
        assert!(
            out.contains("' dish1 (reflector): 3 cards, all in the deck file"),
            "{out}"
        );
        assert!(
            out.contains("GW 1 5") && out.contains("radials radials1"),
            "{out}"
        );
        assert_eq!(out.lines().count(), 6, "{out}");
        let loads: String = (1..=60)
            .map(|k| format!("LD\t5\t{k}\t0\t0\t58000000\n"))
            .collect();
        let out = deck_summary(&format!("CE\n{loads}EN\n"));
        assert!(
            out.contains("' ... 56 more LD cards, all in the deck file ..."),
            "{out}"
        );
        assert!(
            out.contains("LD      5       60") && !out.contains('\t'),
            "{out}"
        );
        assert_eq!(out.lines().count(), 7, "{out}");
    }

    #[test]
    fn round_ticks() {
        assert_eq!(nice_step(0.53), 1.0);
        assert_eq!(nice_step(0.13), 0.2);
        assert_eq!(nice_step(2.2), 2.5);
        assert_eq!(ticks(1.0, 3.66, 0.5), vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5]);
        assert_eq!(tick(2.5, 0.5), "2.5");
        assert_eq!(tick(14.05, 0.05), "14.05");
        assert_eq!(tick(-0.0, 1.0), "0");
    }

    #[test]
    fn a_report_with_everything() {
        let mut s = SweepResults::new(FrequencyPlan {
            start_mhz: 14.0,
            stop_mhz: 14.4,
            points: 5,
        });
        for k in 0..5 {
            let mhz = 14.0 + 0.1 * k as f64;
            s.insert(
                k,
                FreqPoint {
                    mhz,
                    inputs: vec![InputParams {
                        tag: 1,
                        segment: 6,
                        voltage: Complex64::new(1.0, 0.0),
                        current: Complex64::new(0.02, 0.0),
                        impedance: Complex64::new(60.0 + 10.0 * k as f64, -20.0 + 10.0 * k as f64),
                        admittance: Complex64::new(0.02, 0.0),
                        power: 0.01,
                    }],
                    network: Vec::new(),
                    power: None,
                    pattern: None,
                    pattern_step: None,
                    currents: Vec::new(),
                    estimated: false,
                },
            );
        }
        let cut = Cut {
            points: (0..360)
                .map(|a| (a as f64, -10.0 * (1.0 - (a as f64).to_radians().cos())))
                .collect(),
        };
        let deck: String = (0..200)
            .map(|k| format!("GW {k} 21 0 0 {k} 1 0 {k} .001\n"))
            .collect();
        let seg = Segment {
            number: 1,
            tag: 1,
            centre: [0.0, 0.0, 1.0],
            length: 0.5,
            alpha: 90.0,
            beta: 0.0,
            radius: 0.001,
            connections: [0, 0],
        };
        let bytes = report_pdf(
            &ReportInput {
                name: "20 m dipole (test)",
                deck_text: &deck,
                sweep: Some(&s),
                point: s.nearest(14.2),
                wires: &[],
                z0: 50.0,
                cuts: Some((cut.clone(), cut)),
                over_ground: false,
                feed: 0,
            },
            &[seg],
        );
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        // The front page, the sweep, and the deck with its 200 wires
        // summed up.
        assert!(
            text.contains("/Count 3"),
            "{}",
            text.matches("/Type /Page ").count()
        );
        assert!(text.contains("(Feed impedance)"));
        assert!(text.contains("more GW cards"));
        assert!(text.contains("page 3 of 3"));
    }

    /// Writes a sample report to the temp folder, to look at.
    #[test]
    #[ignore]
    fn sample_pdf() {
        let deck =
            "CM test\nCE\nGW 1 21 0 -5 10 0 5 10 .001\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n";
        let bytes = report_pdf(
            &ReportInput {
                name: "sample",
                deck_text: deck,
                sweep: None,
                point: None,
                wires: &[],
                z0: 50.0,
                cuts: None,
                over_ground: false,
                feed: 0,
            },
            &[],
        );
        std::fs::write(std::env::temp_dir().join("straws-sample.pdf"), bytes).unwrap();
    }
}
