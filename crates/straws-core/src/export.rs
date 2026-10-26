//! Exports: CSV for spreadsheets and Octave/MATLAB, Touchstone for circuit
//! simulators, DXF for CAD, and a report as a web page (print it to PDF
//! from any browser).

use std::fmt::Write;

use num_complex::Complex64;

use crate::analysis::feed;
use crate::analysis::pattern::{Component, Cut, PatternGrid};
use crate::analysis::sweep::{FreqPoint, SweepResults};
use crate::deck::edit::Wire;
use crate::output::Segment;

/// The sweep as CSV: frequency, impedance, SWR, return loss, reflection
/// coefficient and (where there's a pattern) peak gain.
pub fn sweep_csv(sweep: &SweepResults, z0: f64, feed: usize) -> String {
    let mut out = format!(
        "MHz,R (ohm),X (ohm),SWR ({z0} ohm),return loss (dB),|gamma|,gamma angle (deg),peak gain (dBi),efficiency (%)\n"
    );
    for p in sweep.points() {
        let Some(i) = p.feed(feed) else { continue };
        let z = i.impedance;
        let g = (z - z0) / (z + z0);
        let peak = p.pattern.as_ref().map_or(String::new(), |grid| {
            format!("{:.2}", grid.max(Component::Total).0)
        });
        let eff = p
            .power
            .map_or(String::new(), |b| format!("{:.2}", b.efficiency_percent));
        let _ = writeln!(
            out,
            "{:.6},{:.4},{:.4},{:.4},{:.3},{:.5},{:.2},{peak},{eff}",
            p.mhz,
            z.re,
            z.im,
            feed::swr(z, z0),
            feed::return_loss_db(z, z0),
            g.norm(),
            g.arg().to_degrees()
        );
    }
    out
}

/// One frequency's pattern as CSV: every direction with every component.
pub fn pattern_csv(grid: &PatternGrid, mhz: f64) -> String {
    let mut out = format!("# pattern at {mhz} MHz; theta from zenith, phi from +X; gains in dBi\n");
    out.push_str("theta (deg),phi (deg),elevation (deg),total,vertical,horizontal,RHCP,LHCP\n");
    for (it, t) in grid.thetas.iter().enumerate() {
        for (ip, p) in grid.phis.iter().enumerate() {
            let g = |c| grid.gain(c, it, ip).max(-999.99);
            let _ = writeln!(
                out,
                "{t},{p},{},{:.2},{:.2},{:.2},{:.2},{:.2}",
                90.0 - t,
                g(Component::Total),
                g(Component::Vertical),
                g(Component::Horizontal),
                g(Component::Rhcp),
                g(Component::Lhcp)
            );
        }
    }
    out
}

/// Segment currents at one frequency as CSV (RMS, for `watts` input).
pub fn currents_csv(point: &FreqPoint, segments: &[Segment], watts: f64) -> String {
    let scale = point.current_scale(watts) / std::f64::consts::SQRT_2;
    let mut out = format!(
        "# currents at {} MHz, A RMS for {watts} W input\n",
        point.mhz
    );
    out.push_str("segment,wire,x (m),y (m),z (m),|I| (A),phase (deg)\n");
    for c in &point.currents {
        let Some(s) = segments.iter().find(|s| s.number == c.segment) else {
            continue;
        };
        let _ = writeln!(
            out,
            "{},{},{:.4},{:.4},{:.4},{:.5},{:.1}",
            c.segment,
            c.tag,
            s.centre[0],
            s.centre[1],
            s.centre[2],
            c.current.norm() * scale,
            c.current.arg().to_degrees()
        );
    }
    out
}

/// One-port Touchstone (.s1p): S11 across the sweep, real/imaginary.
pub fn touchstone(sweep: &SweepResults, z0: f64, name: &str, feed: usize) -> String {
    let mut out = format!("! {name}: feed-point reflection from STRAWS\n# MHZ S RI R {z0}\n");
    for p in sweep.points() {
        let Some(i) = p.feed(feed) else { continue };
        let g = (i.impedance - z0) / (i.impedance + z0);
        let _ = writeln!(out, "{:.6} {:.8} {:.8}", p.mhz, g.re, g.im);
    }
    out
}

/// The wires as a 3D DXF drawing (one LINE per segment, layer per wire),
/// in metres. FreeCAD, LibreCAD and most CAD programs read it.
pub fn dxf(segments: &[Segment]) -> String {
    let mut out = String::from(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n6\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
    );
    for s in segments {
        let (a, b) = s.ends();
        let _ = write!(
            out,
            "0\nLINE\n8\nWIRE_{}\n10\n{}\n20\n{}\n30\n{}\n11\n{}\n21\n{}\n31\n{}\n",
            s.tag, a[0], a[1], a[2], b[0], b[1], b[2]
        );
    }
    out.push_str("0\nENDSEC\n0\nEOF\n");
    out
}

/// The wires as solid tubes (12-sided, closed at the ends) in an ASCII STL
/// file, in millimetres as CAD programs expect. FreeCAD can turn the mesh
/// into a solid (Part > Create shape from mesh) and save it as STEP.
pub fn stl(segments: &[Segment], name: &str) -> String {
    const SIDES: usize = 12;
    let mut out = format!(
        "solid {}
",
        name.replace(char::is_whitespace, "_")
    );
    let mut facet = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-30);
        let _ = write!(
            out,
            "facet normal {} {} {}
outer loop
vertex {} {} {}
vertex {} {} {}
vertex {} {} {}
endloop
endfacet
",
            n[0] / len,
            n[1] / len,
            n[2] / len,
            a[0],
            a[1],
            a[2],
            b[0],
            b[1],
            b[2],
            c[0],
            c[1],
            c[2]
        );
    };
    for s in segments {
        let (a, b) = s.ends();
        let (a, b) = (a.map(|v| v * 1000.0), b.map(|v| v * 1000.0));
        let r = s.radius * 1000.0;
        let d = s.direction();
        // Two directions square to the segment.
        let helper = if d[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let cross = |p: [f64; 3], q: [f64; 3]| {
            [
                p[1] * q[2] - p[2] * q[1],
                p[2] * q[0] - p[0] * q[2],
                p[0] * q[1] - p[1] * q[0],
            ]
        };
        let norm = |p: [f64; 3]| {
            let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-30);
            p.map(|v| v / l)
        };
        let e1 = norm(cross(d, helper));
        let e2 = cross(d, e1);
        let ring = |c: [f64; 3], k: usize| {
            let t = 2.0 * std::f64::consts::PI * k as f64 / SIDES as f64;
            [0, 1, 2].map(|i| c[i] + r * (t.cos() * e1[i] + t.sin() * e2[i]))
        };
        for k in 0..SIDES {
            let (p0, p1) = (ring(a, k), ring(a, k + 1));
            let (q0, q1) = (ring(b, k), ring(b, k + 1));
            facet(p0, p1, q1);
            facet(p0, q1, q0);
            facet(a, p1, p0);
            facet(b, q0, q1);
        }
    }
    let _ = writeln!(out, "endsolid {}", name.replace(char::is_whitespace, "_"));
    out
}

/// Wraps drawings in a standalone SVG file with a white background, so it
/// reads the same in any program.
fn standalone(w: f64, h: f64, inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\
         <rect width=\"{w}\" height=\"{h}\" fill=\"white\"/>{inner}</svg>\n"
    )
}

/// SWR across the sweep as an SVG image.
pub fn swr_svg(sweep: &SweepResults, z0: f64, feed: usize) -> String {
    let swr: Vec<(f64, f64)> = sweep
        .points()
        .filter_map(|p| {
            p.feed(feed)
                .map(|i| (p.mhz, feed::swr(i.impedance, z0).min(20.0)))
        })
        .collect();
    standalone(
        640.0,
        260.0,
        &svg_chart(&swr, "MHz", &format!("SWR ({z0} Ω)"), Some(1.0)),
    )
}

/// The elevation and azimuth cuts side by side as an SVG image.
pub fn cuts_svg(
    elevation: &Cut,
    el_title: &str,
    azimuth: &Cut,
    az_title: &str,
    upper_half: bool,
) -> String {
    let inner = format!(
        "{}<g transform=\"translate(300,0)\">{}</g>",
        svg_polar(elevation, el_title, upper_half),
        svg_polar(azimuth, az_title, false)
    );
    standalone(600.0, 300.0, &inner)
}

/// The Smith chart of the sweep's feed impedance as an SVG image, with the
/// frequency `mark` (if any) highlighted.
pub fn smith_svg(sweep: &SweepResults, z0: f64, mark: Option<f64>, feed: usize) -> String {
    let (size, r) = (460.0, 200.0);
    let (cx, cy) = (size / 2.0, size / 2.0);
    let at = |z: Complex64| {
        let g = (z - 1.0) / (z + 1.0);
        (cx + g.re * r, cy - g.im * r)
    };
    let poly = |pts: Vec<(f64, f64)>, style: &str| {
        let p: Vec<String> = pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        format!(
            "<polyline fill=\"none\" {style} points=\"{}\"/>",
            p.join(" ")
        )
    };
    let mut svg = String::new();
    let grid = "stroke=\"#ccc\" stroke-width=\"1\"";
    for rr in [0.0, 0.2, 0.5, 1.0, 2.0, 5.0] {
        let pts = (0..=200)
            .map(|k| {
                let t = -std::f64::consts::FRAC_PI_2 * 0.999
                    + std::f64::consts::PI * 0.999 * k as f64 / 200.0;
                at(Complex64::new(rr, t.tan()))
            })
            .collect();
        svg.push_str(&poly(pts, grid));
    }
    for x in [0.2, 0.5, 1.0, 2.0, 5.0] {
        for sign in [1.0, -1.0] {
            let pts = (0..=160)
                .map(|k| {
                    let t = 0.995 * k as f64 / 160.0;
                    at(Complex64::new(t / (1.0 - t), sign * x))
                })
                .collect();
            svg.push_str(&poly(pts, grid));
        }
    }
    let _ = write!(
        svg,
        "<line x1=\"{}\" y1=\"{cy}\" x2=\"{}\" y2=\"{cy}\" {grid}/>",
        cx - r,
        cx + r
    );
    let trace: Vec<(f64, f64)> = sweep
        .points()
        .filter_map(|p| p.feed(feed).map(|i| at(i.impedance / z0)))
        .collect();
    svg.push_str(&poly(trace, "stroke=\"#c85a14\" stroke-width=\"2\""));
    if let Some(m) = mark
        && let Some(p) = sweep.nearest(m)
        && let Some(i) = p.feed(feed)
    {
        let (x, y) = at(i.impedance / z0);
        let _ = write!(
            svg,
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"5\" fill=\"#c85a14\"/>\
             <text x=\"10\" y=\"20\" font-family=\"sans-serif\" font-size=\"12\">{:.4} MHz: Z = {:.2} {} j{:.2} Ω, SWR {:.2}</text>",
            p.mhz,
            i.impedance.re,
            if i.impedance.im < 0.0 { "−" } else { "+" },
            i.impedance.im.abs(),
            feed::swr(i.impedance, z0)
        );
    }
    let _ = write!(
        svg,
        "<text x=\"10\" y=\"{}\" font-family=\"sans-serif\" font-size=\"11\" fill=\"#666\">Normalised to Z0 = {z0} Ω</text>",
        size - 10.0
    );
    standalone(size, size, &svg)
}

/// What goes into a report.
pub struct ReportInput<'a> {
    pub name: &'a str,
    pub deck_text: &'a str,
    pub sweep: Option<&'a SweepResults>,
    /// The result at the design frequency (with a pattern).
    pub point: Option<&'a FreqPoint>,
    pub wires: &'a [Wire],
    pub z0: f64,
    /// Cuts through the pattern's peak: elevation, then azimuth.
    pub cuts: Option<(Cut, Cut)>,
    pub over_ground: bool,
    /// Which feed point (source, in the deck's order) the figures are for.
    pub feed: usize,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// An SVG line chart.
fn svg_chart(points: &[(f64, f64)], x_label: &str, y_label: &str, y_floor: Option<f64>) -> String {
    let (w, h, m) = (640.0, 260.0, 46.0);
    if points.len() < 2 {
        return String::new();
    }
    let (x0, x1) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
    let (mut y0, mut y1) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
    if let Some(f) = y_floor {
        y0 = y0.min(f);
    }
    if y1 - y0 < 1e-9 {
        y1 = y0 + 1.0;
    }
    let sx = |x: f64| m + (x - x0) / (x1 - x0).max(1e-12) * (w - 2.0 * m);
    let sy = |y: f64| h - m + (y - y0) / (y1 - y0) * -(h - 2.0 * m);
    let path: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("{:.1},{:.1}", sx(*x), sy(*y)))
        .collect();
    let mut svg = format!(
        "<svg viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" xmlns=\"http://www.w3.org/2000/svg\" font-family=\"sans-serif\" font-size=\"11\">\
         <rect x=\"{m}\" y=\"{m}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"#bbb\"/>",
        w - 2.0 * m,
        h - 2.0 * m
    );
    for k in 0..=4 {
        let fy = y0 + (y1 - y0) * k as f64 / 4.0;
        let fx = x0 + (x1 - x0) * k as f64 / 4.0;
        let _ = write!(
            svg,
            "<line x1=\"{m}\" x2=\"{}\" y1=\"{:.1}\" y2=\"{:.1}\" stroke=\"#eee\"/><text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\">{fy:.2}</text>\
             <text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{fx:.3}</text>",
            w - m,
            sy(fy),
            sy(fy),
            m - 4.0,
            sy(fy) + 4.0,
            sx(fx),
            h - m + 16.0
        );
    }
    let _ = write!(
        svg,
        "<polyline fill=\"none\" stroke=\"#c85a14\" stroke-width=\"2\" points=\"{}\"/>\
         <text x=\"{}\" y=\"{}\" text-anchor=\"middle\">{x_label}</text>\
         <text x=\"12\" y=\"{}\" transform=\"rotate(-90 12 {})\" text-anchor=\"middle\">{y_label}</text></svg>",
        path.join(" "),
        w / 2.0,
        h - 8.0,
        h / 2.0,
        h / 2.0
    );
    svg
}

/// A polar plot of a pattern cut (dB below its peak, 40 dB range).
fn svg_polar(cut: &Cut, title: &str, upper_half: bool) -> String {
    let (size, r) = (300.0, 120.0);
    let (cx, cy) = (
        size / 2.0,
        if upper_half { size - 40.0 } else { size / 2.0 },
    );
    let peak = cut.points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let range = 40.0;
    let mut svg = format!(
        "<svg viewBox=\"0 0 {size} {size}\" width=\"{size}\" height=\"{size}\" xmlns=\"http://www.w3.org/2000/svg\" font-family=\"sans-serif\" font-size=\"11\">\
         <text x=\"{cx}\" y=\"14\" text-anchor=\"middle\">{}</text>",
        esc(title)
    );
    for k in 1..=4 {
        let rr = r * k as f64 / 4.0;
        let _ = write!(
            svg,
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{rr:.1}\" fill=\"none\" stroke=\"#ddd\"/>"
        );
    }
    let pts: Vec<String> = cut
        .points
        .iter()
        .map(|&(angle, g)| {
            let u = ((g - (peak - range)) / range).clamp(0.0, 1.0);
            let a = angle.to_radians();
            format!("{:.1},{:.1}", cx + a.cos() * r * u, cy - a.sin() * r * u)
        })
        .collect();
    let _ = write!(
        svg,
        "<polyline fill=\"none\" stroke=\"#c85a14\" stroke-width=\"2\" points=\"{}\"/>\
         <text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\">rings every 10 dB below {peak:.2} dBi</text></svg>",
        pts.join(" "),
        size - 8.0
    );
    svg
}

/// The report as a self-contained web page.
pub fn report_html(r: &ReportInput<'_>) -> String {
    let feed = r.feed;
    let mut h = format!(
        "<!doctype html><html lang=\"en-AU\"><head><meta charset=\"utf-8\"><title>{0}</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:900px;margin:2em auto;padding:0 1em;color:#222}}\
         table{{border-collapse:collapse;margin:1em 0}}td,th{{border:1px solid #ccc;padding:3px 8px;text-align:right}}\
         th{{background:#f4f4f4}}td:first-child,th:first-child{{text-align:left}}pre{{background:#f7f7f7;padding:1em;overflow:auto}}\
         h2{{margin-top:1.6em}}@media print{{h2{{break-before:auto}}}}</style></head><body>\
         <h1>{0}</h1><p>Report from STRAWS. Print this page to save it as a PDF.</p>",
        esc(r.name)
    );
    if let Some(p) = r.point {
        h.push_str("<h2>At the design frequency</h2><table>");
        let row = |h: &mut String, k: &str, v: String| {
            let _ = write!(h, "<tr><td>{k}</td><td>{}</td></tr>", esc(&v));
        };
        row(&mut h, "Frequency", format!("{:.4} MHz", p.mhz));
        if let Some(i) = p.feed(feed) {
            let z: Complex64 = i.impedance;
            row(
                &mut h,
                "Feed impedance",
                format!(
                    "{:.2} {} j{:.2} Ω",
                    z.re,
                    if z.im < 0.0 { "−" } else { "+" },
                    z.im.abs()
                ),
            );
            row(
                &mut h,
                &format!("SWR ({} Ω)", r.z0),
                format!("{:.2}", feed::swr(z, r.z0)),
            );
            row(
                &mut h,
                "Return loss",
                format!("{:.1} dB", feed::return_loss_db(z, r.z0)),
            );
        }
        if let Some(b) = p.power {
            row(
                &mut h,
                "Efficiency",
                format!("{:.1} %", b.efficiency_percent),
            );
        }
        if let Some(g) = &p.pattern {
            let (max, t, ph) = g.max(Component::Total);
            row(&mut h, "Peak gain", format!("{max:.2} dBi"));
            row(
                &mut h,
                "Direction of peak",
                format!("elevation {:.0}°, azimuth {ph:.0}°", 90.0 - t),
            );
        }
        if let Some((el, az)) = &r.cuts {
            if let Some(f) = az.figures() {
                if let Some(bw) = f.beamwidth {
                    row(&mut h, "Azimuth beamwidth (−3 dB)", format!("{bw:.0}°"));
                }
                if let Some(fb) = f.front_to_back {
                    row(&mut h, "Front-to-back", format!("{fb:.1} dB"));
                }
            }
            if let Some(f) = el.figures()
                && let Some(bw) = f.beamwidth
            {
                row(&mut h, "Elevation beamwidth (−3 dB)", format!("{bw:.0}°"));
            }
        }
        row(
            &mut h,
            "Ground",
            if r.over_ground {
                "over ground".into()
            } else {
                "free space".into()
            },
        );
        h.push_str("</table>");
        if let Some((el, az)) = &r.cuts {
            h.push_str("<div>");
            h.push_str(&svg_polar(
                el,
                "Elevation cut through the peak",
                r.over_ground,
            ));
            h.push_str(&svg_polar(az, "Azimuth cut at the peak's elevation", false));
            h.push_str("</div>");
        }
    }
    if let Some(s) = r.sweep {
        let swr: Vec<(f64, f64)> = s
            .points()
            .filter_map(|p| {
                p.feed(feed)
                    .map(|i| (p.mhz, feed::swr(i.impedance, r.z0).min(20.0)))
            })
            .collect();
        if swr.len() > 1 {
            let _ = write!(
                h,
                "<h2>SWR across the sweep</h2>{}",
                svg_chart(&swr, "MHz", &format!("SWR ({} Ω)", r.z0), Some(1.0))
            );
            let rs: Vec<(f64, f64)> = s
                .points()
                .filter_map(|p| p.feed(feed).map(|i| (p.mhz, i.impedance.re)))
                .collect();
            let xs: Vec<(f64, f64)> = s
                .points()
                .filter_map(|p| p.feed(feed).map(|i| (p.mhz, i.impedance.im)))
                .collect();
            let _ = write!(
                h,
                "<h2>Resistance and reactance</h2>{}{}",
                svg_chart(&rs, "MHz", "R (Ω)", None),
                svg_chart(&xs, "MHz", "X (Ω)", None)
            );
        }
    }
    if !r.wires.is_empty() {
        h.push_str("<h2>Wires</h2><table><tr><th>Tag</th><th>Segments</th><th>X1</th><th>Y1</th><th>Z1</th><th>X2</th><th>Y2</th><th>Z2</th><th>Diameter (mm)</th><th>Length</th></tr>");
        for w in r.wires {
            let _ = write!(
                h,
                "<tr><td>{}</td><td>{}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.3}</td><td>{:.4}</td></tr>",
                w.tag,
                w.segments,
                w.a[0],
                w.a[1],
                w.a[2],
                w.b[0],
                w.b[1],
                w.b[2],
                w.radius * 2000.0,
                w.length()
            );
        }
        h.push_str("</table><p>Positions and lengths in metres.</p>");
    }
    let _ = write!(
        h,
        "<h2>Deck</h2><pre>{}</pre></body></html>",
        esc(r.deck_text)
    );
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::plan::FrequencyPlan;
    use crate::output::InputParams;

    #[test]
    fn exports_have_the_numbers() {
        let mut s = SweepResults::new(FrequencyPlan {
            start_mhz: 14.0,
            stop_mhz: 14.2,
            points: 2,
        });
        for (k, mhz) in [14.0, 14.2].into_iter().enumerate() {
            s.insert(
                k,
                FreqPoint {
                    mhz,
                    inputs: vec![InputParams {
                        tag: 1,
                        segment: 6,
                        voltage: Complex64::new(1.0, 0.0),
                        current: Complex64::new(0.02, 0.0),
                        impedance: Complex64::new(50.0, 0.0),
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
        let csv = sweep_csv(&s, 50.0, 0);
        // A feed point past the last falls back to the first.
        assert_eq!(sweep_csv(&s, 50.0, 3), csv);
        assert!(
            csv.lines()
                .nth(1)
                .unwrap()
                .starts_with("14.000000,50.0000,0.0000,1.0000"),
            "{csv}"
        );
        let s1p = touchstone(&s, 50.0, "test", 0);
        assert!(s1p.contains("# MHZ S RI R 50"));
        assert!(s1p.contains("14.200000 0.00000000 0.00000000"));
        let html = report_html(&ReportInput {
            name: "a <test>",
            deck_text: "CE",
            sweep: Some(&s),
            point: s.nearest(14.2),
            wires: &[],
            z0: 50.0,
            cuts: None,
            over_ground: false,
            feed: 0,
        });
        assert!(html.contains("a &lt;test&gt;") && html.contains("<svg"));
    }
}

#[cfg(test)]
mod stl_tests {
    #[test]
    fn a_segment_is_a_closed_tube() {
        let s = crate::output::Segment {
            number: 1,
            tag: 1,
            centre: [0.0, 0.0, 1.0],
            length: 0.5,
            alpha: 90.0,
            beta: 0.0,
            radius: 0.01,
            connections: [0, 0],
        };
        let text = super::stl(&[s], "a test");
        assert!(text.starts_with("solid a_test"));
        // 12 sides: 2 triangles each on the side, 1 each on both ends.
        assert_eq!(text.matches("facet normal").count(), 48);
        // The bottom end is at z = 0.75 m = 750 mm.
        let lowest = text
            .lines()
            .filter_map(|l| l.strip_prefix("vertex "))
            .filter_map(|v| v.split_whitespace().nth(2)?.parse::<f64>().ok())
            .fold(f64::MAX, f64::min);
        assert!((lowest - 750.0).abs() < 1e-6, "{lowest}");
    }

    /// Exported tubes read back as the same wires: an inverted L of three
    /// segments (two up the vertical, one across), 2 mm radius.
    #[test]
    fn exported_tubes_import_again() {
        let seg = |number, centre, length, alpha: f64, beta| crate::output::Segment {
            number,
            tag: 1,
            centre,
            length,
            alpha,
            beta,
            radius: 0.002,
            connections: [0, 0],
        };
        let segments = [
            seg(1, [0.0, 0.0, 0.25], 0.5, 90.0, 0.0),
            seg(2, [0.0, 0.0, 0.75], 0.5, 90.0, 0.0),
            seg(3, [0.3, 0.0, 1.0], 0.6, 0.0, 0.0),
        ];
        let mesh = crate::stl::read_stl(super::stl(&segments, "l").as_bytes()).unwrap();
        let parts = crate::stl::tubes(&mesh);
        // The vertical's two segments share their rings: one tube.
        assert_eq!((parts.tubes.len(), parts.other), (2, 0));
        let wires = crate::stl::wires(&parts.tubes, 0.001, 0.0005);
        let has = |a: [f64; 3], b: [f64; 3]| {
            let near = |p: [f64; 3], q: [f64; 3]| (0..3).all(|i| (p[i] - q[i]).abs() < 1e-6);
            wires
                .iter()
                .any(|w| (near(w.a, a) && near(w.b, b)) || (near(w.a, b) && near(w.b, a)))
        };
        assert!(has([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]), "{wires:?}");
        assert!(has([0.0, 0.0, 1.0], [0.6, 0.0, 1.0]), "{wires:?}");
        assert!(
            wires
                .iter()
                .all(|w| (w.radius.unwrap() - 0.002).abs() < 1e-9)
        );
    }
}

/// The build sheet as a printable web page. Lengths in `unit` (millimetres
/// if that's wavelengths: nobody cuts in wavelengths), diameters and walls
/// in millimetres.
pub fn build_sheet_html(
    name: &str,
    sheet: &crate::analysis::build::Sheet,
    unit: crate::units::Length,
    mhz: f64,
) -> String {
    use crate::units::Length;
    let unit = if unit == Length::Wavelengths {
        Length::Millimetres
    } else {
        unit
    };
    let len = |m: f64| {
        format!(
            "{:.*} {}",
            unit.decimals(),
            unit.from_metres(m, mhz),
            unit.symbol()
        )
    };
    let size = |d: f64, wall: Option<f64>| match wall {
        Some(t) => format!("{:.2} mm tube, {:.2} mm wall", d * 1e3, t * 1e3),
        None => format!("{:.2} mm", d * 1e3),
    };
    let mut h = format!(
        "<!doctype html><html lang=\"en-AU\"><head><meta charset=\"utf-8\"><title>{0} build sheet</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:900px;margin:2em auto;padding:0 1em;color:#222}}\
         table{{border-collapse:collapse;margin:1em 0}}td,th{{border:1px solid #ccc;padding:3px 8px;text-align:left}}\
         th{{background:#f4f4f4}}td.n{{text-align:right}}.note{{color:#555}}</style></head><body>\
         <h1>{0}: build sheet</h1><p class=\"note\">From STRAWS. Print this page to save it as a PDF. \
         Lengths are the conductors as modelled, end to end (where wires join, to the joint). Add what the \
         model doesn't have: overlaps where tubes slide together, clamps, insulators and the feed's \
         hardware. Check the model's results before cutting.</p>",
        esc(name)
    );
    h.push_str("<h2>Cut list</h2><table><tr><th>Qty</th><th>Length</th><th>Size</th><th>Material</th><th>Shape</th><th>Pieces</th></tr>");
    for c in &sheet.cuts {
        let _ = write!(
            h,
            "<tr><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            c.count,
            len(c.length),
            size(c.diameter, c.wall),
            esc(&c.material),
            if c.straight {
                "straight"
            } else {
                "bent (along the curve)"
            },
            esc(&c.names.join(", "))
        );
    }
    h.push_str("</table><h2>Stock to buy</h2><table><tr><th>Material</th><th>Size</th><th>Total length</th></tr>");
    for s in &sheet.stock {
        let _ = write!(
            h,
            "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td></tr>",
            esc(&s.material),
            size(s.diameter, s.wall),
            len(s.total)
        );
    }
    h.push_str("</table><p class=\"note\">Totals without offcuts or overlaps: allow extra.</p>");
    if let Some(p) = &sheet.positions {
        h.push_str(
            "<h2>Element positions</h2><p class=\"note\">Where the parallel elements sit along the line \
             through their middles (a boom), measured from the first; a split element's halves share a \
             row. Mark these centre to centre.</p><table><tr><th>Element</th><th>Position</th>\
             <th>Gap from the one before</th><th>Element length</th></tr>",
        );
        let mut before: Option<f64> = None;
        for (names, at, length) in &p.rows {
            let _ = write!(
                h,
                "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                esc(names),
                len(*at),
                before.map_or("".to_string(), |b| len(at - b)),
                len(*length)
            );
            before = Some(*at);
        }
        h.push_str("</table>");
    }
    h.push_str("<h2>Every piece</h2><table><tr><th>Piece</th><th>Length</th><th>Size</th><th>Material</th></tr>");
    for p in &sheet.pieces {
        let _ = write!(
            h,
            "<tr><td>{}</td><td class=\"n\">{}</td><td>{}</td><td>{}</td></tr>",
            esc(&p.name),
            len(p.length),
            size(p.diameter, p.wall),
            esc(&p.material)
        );
    }
    h.push_str("</table></body></html>");
    h
}
