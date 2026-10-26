//! Live geometry checks: things NEC accepts but that give wrong answers,
//! explained in plain language with a suggested fix.
//!
//! The rules follow the NEC-2 user's guide (part III): segments shorter
//! than about a tenth of a wavelength, at least eight times longer than the
//! wire radius (two with the extended kernel), wires joined only at segment
//! ends, and similar segment lengths either side of a junction.

use crate::deck::NumericDeck;
use crate::output::{Patch, Segment};
use crate::units::wavelength;

use super::pattern::{Component, PatternGrid};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    /// The wires (tags) involved, to highlight.
    pub tags: Vec<u32>,
    /// What's wrong.
    pub message: String,
    /// How to fix it.
    pub fix: String,
}

/// Most issues of one kind listed before they're summarised.
const MAX_PER_KIND: usize = 12;

/// Checks the engine's segments (from a geometry run) and the deck's
/// references to them, at the highest frequency that will be run.
pub fn check(
    segments: &[Segment],
    deck: &NumericDeck,
    max_mhz: f64,
    over_ground: bool,
) -> Vec<Issue> {
    check_in(segments, deck, max_mhz, over_ground, &|m| {
        format!("{} m", short_number(m))
    })
}

/// Like [`check`], with distances in the messages written by `length`
/// (metres in, text with its unit out).
pub fn check_in(
    segments: &[Segment],
    deck: &NumericDeck,
    max_mhz: f64,
    over_ground: bool,
    length: &dyn Fn(f64) -> String,
) -> Vec<Issue> {
    let mut out = Vec::new();
    references(segments, deck, &mut out);
    if segments.is_empty() {
        return out;
    }
    let extended = deck.cards.iter().any(|c| c.is("EK") && c.ints[0] != -1);
    // Tiny wires that only carry a transmission line or network end are
    // fine short.
    let stubs: Vec<u32> = deck.cards[deck.control_start()..]
        .iter()
        .filter(|c| c.is("TL") || c.is("NT"))
        .flat_map(|c| [c.ints[0], c.ints[2]])
        .chain(deck.helper_tags.iter().copied())
        .filter_map(|t| u32::try_from(t).ok())
        .collect();
    per_wire(
        segments,
        max_mhz,
        extended,
        over_ground,
        &stubs,
        length,
        &mut out,
    );
    pairs(segments, length, &mut out);
    out.sort_by_key(|a| std::cmp::Reverse(a.severity));
    out
}

/// Checks surface patches at the highest frequency that will be run: each
/// under about 0.04 λ² (the NEC-2 user's guide asks for at least 25 patches
/// per square wavelength), and together closed surfaces, since NEC-2's
/// patch model (the magnetic-field integral equation) is only valid for
/// closed surfaces. On a closed surface the area-weighted normals add up to
/// zero; a flat sheet's don't. Over ground, a surface standing on it may
/// be open at the bottom (its image closes it, as in the NEC-2 manual's
/// box on perfect ground).
pub fn patch_checks(patches: &[Patch], max_mhz: f64, over_ground: bool) -> Vec<Issue> {
    let mut out = Vec::new();
    if patches.is_empty() {
        return out;
    }
    let lambda = wavelength(max_mhz.max(1e-6));
    let largest = patches.iter().map(|p| p.area).fold(0.0, f64::max);
    let limit = 0.04 * lambda * lambda;
    if largest > limit {
        let big = patches.iter().filter(|p| p.area > limit).count();
        out.push(Issue {
            severity: Severity::Warning,
            tags: vec![],
            message: format!(
                "{big} surface patch{} {} larger than 0.04 λ² at {max_mhz:.3} MHz (largest {:.3} λ²).",
                if big == 1 { "" } else { "es" },
                if big == 1 { "is" } else { "are" },
                largest / (lambda * lambda)
            ),
            fix: "Use more, smaller patches: at least 25 per square wavelength.".into(),
        });
    }
    let total: f64 = patches.iter().map(|p| p.area).sum();
    let mut sum = [0, 1, 2].map(|k| patches.iter().map(|p| p.normal[k] * p.area).sum::<f64>());
    let on_ground = patches
        .iter()
        .any(|p| p.centre[2] <= 0.75 * p.area.sqrt() + 1e-9);
    if over_ground && on_ground {
        sum[2] = 0.0;
    }
    if dot(sum, sum).sqrt() > 0.05 * total {
        out.push(Issue {
            severity: Severity::Warning,
            tags: vec![],
            message: "The surface patches don't form a closed surface. NEC-2's patches only give right answers on closed surfaces (a box, a cylinder with ends)."
                .into(),
            fix: "Close the surface, or model a flat sheet as a wire grid instead (Draw > Plate).".into(),
        });
    }
    out
}

fn push_limited(out: &mut Vec<Issue>, kind: &mut Vec<Issue>, what: &str) {
    let extra = kind.len().saturating_sub(MAX_PER_KIND);
    let severity = kind.iter().map(|i| i.severity).max();
    let mut tags = Vec::new();
    for i in kind.iter().skip(MAX_PER_KIND) {
        tags.extend(&i.tags);
    }
    kind.truncate(MAX_PER_KIND);
    out.append(kind);
    if extra > 0 {
        out.push(Issue {
            severity: severity.unwrap_or(Severity::Warning),
            tags,
            message: format!("…and {extra} more {what}."),
            fix: String::new(),
        });
    }
}

/// Segment length against wavelength and radius, wires under the ground.
fn per_wire(
    segments: &[Segment],
    mhz: f64,
    extended: bool,
    over_ground: bool,
    stubs: &[u32],
    length: &dyn Fn(f64) -> String,
    out: &mut Vec<Issue>,
) {
    let lambda = wavelength(mhz);
    let mut tags: Vec<u32> = segments.iter().map(|s| s.tag).collect();
    tags.sort_unstable();
    tags.dedup();
    let (mut long, mut short, mut thick, mut under) = (vec![], vec![], vec![], vec![]);
    for tag in tags {
        let on: Vec<&Segment> = segments.iter().filter(|s| s.tag == tag).collect();
        let longest = on.iter().map(|s| s.length).fold(0.0, f64::max);
        let shortest = on.iter().map(|s| s.length).fold(f64::MAX, f64::min);
        let total: f64 = on.iter().map(|s| s.length).sum();
        let ratio = on
            .iter()
            .map(|s| s.length / s.radius.max(1e-30))
            .fold(f64::MAX, f64::min);
        if longest > 0.1 * lambda {
            let need = (total / (0.1 * lambda)).ceil() as usize;
            long.push(Issue {
                severity: if longest > 0.25 * lambda {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                tags: vec![tag],
                message: format!(
                    "Wire {tag}: segments are {:.3} λ long at {mhz:.3} MHz, too long for accurate results.",
                    longest / lambda
                ),
                fix: format!(
                    "Keep segments under 0.1 λ: give it at least {need} segments."
                ),
            });
        }
        if shortest < 0.001 * lambda && !stubs.contains(&tag) {
            short.push(Issue {
                severity: Severity::Warning,
                tags: vec![tag],
                message: format!(
                    "Wire {tag}: segments are only {:.5} λ long, which can make results inaccurate.",
                    shortest / lambda
                ),
                fix: "Use fewer segments on this wire (at least 0.001 λ each).".into(),
            });
        }
        let limit = if extended { 2.0 } else { 8.0 };
        if ratio < limit {
            thick.push(Issue {
                severity: if ratio < 2.0 {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                tags: vec![tag],
                message: format!(
                    "Wire {tag}: segments are only {ratio:.1}× as long as the wire radius; NEC's thin-wire maths needs {limit}× or more."
                ),
                fix: if extended {
                    "Use fewer segments or a thinner wire.".into()
                } else {
                    "Use fewer segments or a thinner wire, or add an EK card (extended kernel, 2× is enough).".into()
                },
            });
        }
        if over_ground {
            let lowest = on
                .iter()
                .flat_map(|s| {
                    let (a, b) = s.ends();
                    [a[2], b[2]]
                })
                .fold(f64::MAX, f64::min);
            // Segment ends come from the engine's printout (5 figures), so
            // a wire standing on the ground can seem a hair below it (they
            // are printed to 10 µm).
            let shortest = on.iter().map(|s| s.length).fold(f64::MAX, f64::min);
            if lowest < -(1e-3 * shortest).max(1e-4) {
                under.push(Issue {
                    severity: Severity::Error,
                    tags: vec![tag],
                    message: format!(
                        "Wire {tag} goes {} below the ground; NEC can't model buried wires this way.",
                        length(-lowest)
                    ),
                    fix: "Raise the wire to z = 0 or above.".into(),
                });
            }
        }
    }
    push_limited(out, &mut under, "wires below ground");
    push_limited(out, &mut long, "wires with long segments");
    push_limited(out, &mut thick, "wires with short, thick segments");
    push_limited(out, &mut short, "wires with very short segments");
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(sub(a, b), sub(a, b)).sqrt()
}

/// Closest approach of segments p0-p1 and q0-q1: (distance, s, t) with s
/// and t the positions along each, 0 at the start and 1 at the end.
fn closest(p0: [f64; 3], p1: [f64; 3], q0: [f64; 3], q1: [f64; 3]) -> (f64, f64, f64) {
    let (d1, d2, r) = (sub(p1, p0), sub(q1, q0), sub(p0, q0));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (c, b) = (dot(d1, r), dot(d1, d2));
    let denom = a * e - b * b;
    let mut s = if denom > 1e-18 * a * e {
        ((b * f - c * e) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if e > 0.0 { (b * s + f) / e } else { 0.0 };
    if t < 0.0 {
        t = 0.0;
        s = if a > 0.0 {
            (-c / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    } else if t > 1.0 {
        t = 1.0;
        s = if a > 0.0 {
            ((b - c) / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    let p = [0, 1, 2].map(|k| p0[k] + d1[k] * s);
    let q = [0, 1, 2].map(|k| q0[k] + d2[k] * t);
    (dist(p, q), s, t)
}

/// Crossing, touching, nearly-joined and duplicate wires, and junctions
/// between very different segment lengths.
fn pairs(segments: &[Segment], length: &dyn Fn(f64) -> String, out: &mut Vec<Issue>) {
    let mut p = pair_issues(segments, length);
    push_limited(out, &mut p.dupes, "overlapping wires");
    push_limited(out, &mut p.crossing, "crossing wires");
    push_limited(out, &mut p.near, "nearly joined wires");
    push_limited(out, &mut p.close, "closely spaced wires");
    push_limited(out, &mut p.junctions, "uneven junctions");
}

/// Problems between pairs of segments, by kind.
struct Pairs {
    dupes: Vec<Issue>,
    crossing: Vec<Issue>,
    near: Vec<Issue>,
    close: Vec<Issue>,
    junctions: Vec<Issue>,
}

/// Designs that can't be built: wires lying on top of each other, crossing
/// or touching without a joint, or going below the ground. (The optimiser
/// skips designs with more of these than the design it started from.)
pub fn clashes(segments: &[Segment], deck: &NumericDeck, over_ground: bool) -> Vec<Issue> {
    let mut p = pair_issues(segments, &|m| format!("{} m", short_number(m)));
    let mut out = std::mem::take(&mut p.dupes);
    out.append(&mut p.crossing);
    if over_ground {
        for s in segments {
            if deck.helper_tags.contains(&i64::from(s.tag)) {
                continue;
            }
            let (a, b) = s.ends();
            if a[2].min(b[2]) < -(1e-3 * s.length).max(1e-4) {
                out.push(Issue {
                    severity: Severity::Error,
                    tags: vec![s.tag],
                    message: format!("Wire {} goes below the ground.", s.tag),
                    fix: String::new(),
                });
                break;
            }
        }
    }
    out
}

fn pair_issues(segments: &[Segment], length: &dyn Fn(f64) -> String) -> Pairs {
    let ends: Vec<([f64; 3], [f64; 3])> = segments.iter().map(Segment::ends).collect();
    // Sweep along the axis the structure spreads most in, so only
    // segments whose extents overlap are compared.
    let spread = |k: usize| {
        let (lo, hi) = ends.iter().fold((f64::MAX, f64::MIN), |(lo, hi), (a, b)| {
            (lo.min(a[k]).min(b[k]), hi.max(a[k]).max(b[k]))
        });
        hi - lo
    };
    let axis = (0..3)
        .max_by(|&i, &j| spread(i).total_cmp(&spread(j)))
        .unwrap_or(0);
    let margin = |i: usize| 4.0 * segments[i].radius + 0.1 * segments[i].length;
    let range = |i: usize| {
        let (a, b) = ends[i];
        (
            a[axis].min(b[axis]) - margin(i),
            a[axis].max(b[axis]) + margin(i),
        )
    };
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&i, &j| range(i).0.total_cmp(&range(j).0));
    let reach = segments
        .iter()
        .map(|s| s.length + 8.0 * s.radius)
        .fold(0.0, f64::max);

    let groups = Junctions::new(segments, &ends);
    let free = |i: usize, e: usize| segments[i].connections[e] == 0;
    let (mut crossing, mut near, mut close, mut dupes, mut junctions) =
        (vec![], vec![], vec![], vec![], vec![]);
    let mut seen = std::collections::HashSet::new();
    for (n, &i) in order.iter().enumerate() {
        let (_, hi) = range(i);
        for &j in &order[n + 1..] {
            let (jlo, _) = range(j);
            if jlo > hi {
                break;
            }
            let (si, sj) = (&segments[i], &segments[j]);
            let (pi, pj) = (ends[i], ends[j]);
            if dist(si.centre, sj.centre) > reach * 2.0 {
                continue;
            }
            // Joined at a junction, by NEC's own connection table (the
            // printed coordinates are too coarse to decide it ourselves).
            let joined = (0..2).any(|e| (0..2).any(|f| groups.same(2 * i + e, 2 * j + f)));
            let tol = 1e-3 * si.length.min(sj.length);
            let key = (si.tag.min(sj.tag), si.tag.max(sj.tag));
            if joined {
                let ratio = si.length.max(sj.length) / si.length.min(sj.length).max(1e-30);
                if ratio > 5.0 && si.tag != sj.tag && seen.insert((key, 0)) {
                    junctions.push(Issue {
                        severity: Severity::Warning,
                        tags: vec![key.0, key.1],
                        message: format!(
                            "Wires {} and {} meet with segments {ratio:.0}× different in length.",
                            key.0, key.1
                        ),
                        fix: "Change the segment counts so segments either side of a junction are similar (within about 2×).".into(),
                    });
                }
                // A sharp bend: the two segments fold back towards each
                // other (the angle between them, at the joint, is small).
                for e in 0..2 {
                    for f in 0..2 {
                        if !groups.same(2 * i + e, 2 * j + f) {
                            continue;
                        }
                        let out = |p: ([f64; 3], [f64; 3]), end: usize| {
                            let (from, to) = if end == 0 { (p.0, p.1) } else { (p.1, p.0) };
                            [0, 1, 2].map(|k| to[k] - from[k])
                        };
                        let (u, v) = (out(pi, e), out(pj, f));
                        let dot: f64 = (0..3).map(|k| u[k] * v[k]).sum();
                        let norm = (0..3).map(|k| u[k] * u[k]).sum::<f64>().sqrt()
                            * (0..3).map(|k| v[k] * v[k]).sum::<f64>().sqrt();
                        let angle = (dot / norm.max(1e-300))
                            .clamp(-1.0, 1.0)
                            .acos()
                            .to_degrees();
                        // Near the joint they run closer than twice their
                        // radii for (ri + rj) / sin(angle / 2): a problem only
                        // when that's a real part of a segment (a fan of thin
                        // wires, such as a discone, is fine).
                        let close_for =
                            (si.radius + sj.radius) / (angle.to_radians() / 2.0).sin().max(1e-9);
                        let sharp = angle < 30.0 && close_for > 0.1 * si.length.min(sj.length);
                        if sharp && seen.insert((key, 2)) {
                            junctions.push(Issue {
                                severity: Severity::Warning,
                                tags: vec![key.0, key.1],
                                message: format!(
                                    "Wire{} {} bend{} sharply: two segments meet at {angle:.0}°.",
                                    if key.0 == key.1 { "" } else { "s" },
                                    if key.0 == key.1 { key.0.to_string() } else { format!("{} and {}", key.0, key.1) },
                                    if key.0 == key.1 { "s" } else { "" },
                                ),
                                fix: "NEC is inaccurate where wires fold back on themselves: open the bend out, or use more segments on a curve so each step turns less.".into(),
                            });
                        }
                    }
                }
                continue;
            }
            let (d, s, t) = closest(pi.0, pi.1, pj.0, pj.1);
            let radii = si.radius + sj.radius;
            let at_end = |x: f64| !(1e-6..=1.0 - 1e-6).contains(&x);
            let same = (dist(pi.0, pj.0) <= tol && dist(pi.1, pj.1) <= tol)
                || (dist(pi.0, pj.1) <= tol && dist(pi.1, pj.0) <= tol);
            let gap_limit = 0.1 * si.length.min(sj.length);
            // Side by side: parallel and overlapping along most of their
            // length (their nearest points then fall on their ends, which
            // isn't a near miss between ends).
            let side_by_side = {
                let (u, v) = (sub(pi.1, pi.0), sub(pj.1, pj.0));
                let (lu, lv) = (dot(u, u).sqrt(), dot(v, v).sqrt());
                let parallel = lu > 0.0 && lv > 0.0 && (dot(u, v) / (lu * lv)).abs() > 0.995;
                // How much of the shorter one lies alongside the other.
                let along = |p: [f64; 3]| dot(sub(p, pi.0), u) / (lu * lu);
                let (a, b) = (along(pj.0), along(pj.1));
                let overlap = (a.max(b).min(1.0) - a.min(b).max(0.0)) * lu;
                parallel && overlap > 0.5 * lu.min(lv)
            };
            if same {
                if seen.insert((key, 1)) {
                    dupes.push(Issue {
                        severity: Severity::Error,
                        tags: vec![key.0, key.1],
                        message: if key.0 == key.1 {
                            format!("Wire {} has segments on top of each other.", key.0)
                        } else {
                            format!("Wires {} and {} lie on top of each other.", key.0, key.1)
                        },
                        fix: "Delete one of them.".into(),
                    });
                }
            } else if side_by_side && d < 2.0 * radii {
                if seen.insert((key, 4)) {
                    let touching = d < radii;
                    let issue = Issue {
                        severity: if touching {
                            Severity::Error
                        } else {
                            Severity::Warning
                        },
                        tags: vec![key.0, key.1],
                        message: if touching {
                            format!(
                                "Wires {} and {} touch along their length ({} apart).",
                                key.0,
                                key.1,
                                length(d)
                            )
                        } else {
                            format!(
                                "Wires {} and {} run side by side only {} apart, close for their thickness.",
                                key.0,
                                key.1,
                                length(d)
                            )
                        },
                        fix: "Keep parallel wires several radii apart (NEC's thin-wire maths assumes it), \
                              or check the result against a finer model."
                            .into(),
                    };
                    if touching {
                        crossing.push(issue);
                    } else {
                        close.push(issue);
                    }
                }
            } else if at_end(s)
                && at_end(t)
                && free(i, usize::from(s > 0.5))
                && free(j, usize::from(t > 0.5))
                && d < gap_limit.max(radii)
            {
                if seen.insert((key, 2)) {
                    near.push(Issue {
                        severity: Severity::Error,
                        tags: vec![key.0, key.1],
                        message: format!(
                            "Wires {} and {} nearly meet ({} gap) but aren't joined.",
                            key.0,
                            key.1,
                            length(d)
                        ),
                        fix: "Make the two end points exactly the same (drag one end onto the other, with snapping on).".into(),
                    });
                }
            } else if d < radii {
                // A joined end resting on another wire is a proper junction.
                let end_free = (at_end(s) && free(i, usize::from(s > 0.5)))
                    || (at_end(t) && free(j, usize::from(t > 0.5)));
                let interior = !at_end(s) && !at_end(t);
                if (end_free || interior) && seen.insert((key, 3)) {
                    let (msg, fix) = if !interior {
                        (
                            format!(
                                "A wire end touches the middle of a segment (wires {} and {}); NEC only joins wires at segment ends.",
                                key.0, key.1
                            ),
                            "Split the wire where they meet, so a segment ends there.".to_string(),
                        )
                    } else {
                        (
                            format!(
                                "Wires {} and {} cross or touch without being joined.",
                                key.0, key.1
                            ),
                            "Move one apart, or split both wires so they meet at a shared end point.".to_string(),
                        )
                    };
                    crossing.push(Issue {
                        severity: Severity::Error,
                        tags: vec![key.0, key.1],
                        message: msg,
                        fix,
                    });
                }
            } else if d < 2.0 * radii && !at_end(s) && !at_end(t) && seen.insert((key, 4)) {
                close.push(Issue {
                    severity: Severity::Warning,
                    tags: vec![key.0, key.1],
                    message: format!(
                        "Wires {} and {} are only {} apart, close for their thickness.",
                        key.0,
                        key.1,
                        length(d)
                    ),
                    fix:
                        "Keep wires several radii apart, or check the result against a finer model."
                            .into(),
                });
            }
        }
    }
    Pairs {
        dupes,
        crossing,
        near,
        close,
        junctions,
    }
}

/// Segment ends that NEC joined, grouped (union-find over its I-/I+
/// connection data; ends are numbered 2·segment + end).
struct Junctions {
    parent: Vec<usize>,
}

impl Junctions {
    fn new(segments: &[Segment], ends: &[([f64; 3], [f64; 3])]) -> Junctions {
        let mut g = Junctions {
            parent: (0..2 * segments.len()).collect(),
        };
        let index: std::collections::HashMap<u32, usize> = segments
            .iter()
            .enumerate()
            .map(|(k, s)| (s.number, k))
            .collect();
        for (i, s) in segments.iter().enumerate() {
            for e in 0..2 {
                let c = s.connections[e].unsigned_abs() as u32;
                if c == 0 || c == s.number {
                    continue;
                }
                let Some(&j) = index.get(&c) else { continue };
                let p = if e == 0 { ends[i].0 } else { ends[i].1 };
                let f = usize::from(dist(p, ends[j].1) < dist(p, ends[j].0));
                g.union(2 * i + e, 2 * j + f);
            }
        }
        g
    }

    fn find(&self, mut x: usize) -> usize {
        while self.parent[x] != x {
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.parent[a] = b;
    }

    fn same(&self, a: usize, b: usize) -> bool {
        self.find(a) == self.find(b)
    }
}

fn short_number(v: f64) -> String {
    format!("{:.3}", v)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// Sources, loads and lines that point at wires or segments that don't
/// exist, and a deck with nothing feeding it.
fn references(segments: &[Segment], deck: &NumericDeck, out: &mut Vec<Issue>) {
    let control = &deck.cards[deck.control_start()..];
    let count = |tag: i64| segments.iter().filter(|s| i64::from(s.tag) == tag).count() as i64;
    let total = segments.len() as i64;
    let mut bad = |what: &str, tag: i64, seg: i64| {
        let problem = if tag == 0 {
            (seg < 1 || seg > total).then(|| format!("segment {seg}, but the model has {total}"))
        } else {
            match count(tag) {
                0 => Some(format!("wire {tag}, which doesn't exist")),
                n if seg < 1 || seg > n => {
                    Some(format!("segment {seg} of wire {tag}, which has {n}"))
                }
                _ => None,
            }
        };
        if let Some(p) = problem
            && !segments.is_empty()
        {
            out.push(Issue {
                severity: Severity::Error,
                tags: if tag > 0 { vec![tag as u32] } else { vec![] },
                message: format!("The {what} is on {p}."),
                fix: format!("Point the {what} at a wire and segment that exist."),
            });
        }
    };
    let mut sources = 0;
    for c in control {
        match c.mnemonic.as_str() {
            "EX" if matches!(c.ints[0], 0 | 5) => {
                sources += 1;
                bad("source (EX card)", c.ints[1], c.ints[2]);
            }
            "EX" => sources += 1,
            "LD" if c.ints[0] != -1 && !(c.ints[2] == 0 && c.ints[3] == 0) => {
                let (tag, m, n) = (c.ints[1], c.ints[2], c.ints[3]);
                bad("load (LD card)", tag, m);
                if n != 0 && n != m {
                    bad("load (LD card)", tag, n);
                }
            }
            "LD" if c.ints[0] != -1 && c.ints[1] != 0 && count(c.ints[1]) == 0 => {
                bad("load (LD card)", c.ints[1], 1);
            }
            "NT" | "TL" => {
                let what = if c.is("NT") {
                    "network (NT card)"
                } else {
                    "transmission line (TL card)"
                };
                bad(what, c.ints[0], c.ints[1]);
                bad(what, c.ints[2], c.ints[3]);
            }
            _ => {}
        }
    }
    if sources == 0 {
        out.push(Issue {
            severity: Severity::Error,
            tags: vec![],
            message: "Nothing feeds the antenna: the deck has no source (EX card).".into(),
            fix: "Select a wire and click \"Feed this wire\" in the Wires view.".into(),
        });
    }
}

/// The average gain test: the radiated power the pattern adds up to, as a
/// share of the input power (the pattern's power gain integrated over the
/// directions it covers, divided by 4π). A lossless antenna gives 1 (0 dB)
/// in free space and over perfect ground; far from that means the model
/// isn't trustworthy. Over real ground some power is lost in the ground,
/// so less is normal. Returns `None` if the grid doesn't cover the whole
/// sphere (or the upper half, over ground).
pub fn average_gain(grid: &PatternGrid) -> Option<f64> {
    let (t, p) = (&grid.thetas, &grid.phis);
    if t.len() < 3 || p.len() < 3 {
        return None;
    }
    let top = t[0].abs() < 1e-6;
    let bottom = (t[t.len() - 1] - 180.0).abs() < 1e-6 || (t[t.len() - 1] - 90.0).abs() < 1e-6;
    let step = p[1] - p[0];
    let full_circle = (p[p.len() - 1] - p[0] + step - 360.0).abs() < 1e-6
        || (p[p.len() - 1] - p[0] - 360.0).abs() < 1e-6;
    if !(top && bottom && full_circle) {
        return None;
    }
    // Drop a repeated 360° column.
    let np = if (p[p.len() - 1] - p[0] - 360.0).abs() < 1e-6 {
        p.len() - 1
    } else {
        p.len()
    };
    let dphi = (360.0 / np as f64).to_radians();
    let mut sum = 0.0;
    for it in 0..t.len() {
        // Trapezoid weights in theta.
        let lo = if it == 0 {
            t[0]
        } else {
            (t[it - 1] + t[it]) / 2.0
        };
        let hi = if it + 1 == t.len() {
            t[it]
        } else {
            (t[it] + t[it + 1]) / 2.0
        };
        let band = lo.to_radians().cos() - hi.to_radians().cos();
        let row: f64 = (0..np)
            .map(|ip| {
                let g = grid.gain(Component::Total, it, ip);
                if g <= -999.0 {
                    0.0
                } else {
                    10f64.powf(g / 10.0)
                }
            })
            .sum::<f64>();
        sum += row * dphi * band;
    }
    Some(sum / (4.0 * std::f64::consts::PI))
}

/// The receiving directivity factor (dB): the peak gain over the average
/// gain across all the directions the pattern covers (the upper half over
/// ground, where the noise comes from; the whole sphere in free space).
/// The figure of merit for receive antennas (Beverages, K9AY loops, flags):
/// how much more they hear from the wanted direction than from everywhere.
/// Losses don't change it. `None` if the grid doesn't cover the whole
/// sphere or upper half.
pub fn rdf(grid: &PatternGrid) -> Option<f64> {
    let average = average_gain(grid)?;
    let half = (grid.thetas.last()? - 90.0).abs() < 1e-6;
    // average_gain divides by 4π; the mean over a half sphere is twice that.
    let mean = if half { 2.0 * average } else { average };
    if mean <= 0.0 {
        return None;
    }
    let (peak, _, _) = grid.max(Component::Total);
    Some(peak - 10.0 * mean.log10())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Segment;

    fn seg(number: u32, tag: u32, a: [f64; 3], b: [f64; 3], radius: f64) -> Segment {
        let d = sub(b, a);
        let length = dot(d, d).sqrt();
        let alpha = (d[2] / length).asin().to_degrees();
        let beta = d[1].atan2(d[0]).to_degrees();
        Segment {
            number,
            tag,
            centre: [0, 1, 2].map(|k| (a[k] + b[k]) / 2.0),
            length,
            alpha,
            beta,
            radius,
            connections: [0, 0],
        }
    }

    fn deck() -> NumericDeck {
        crate::deck::Deck::parse("CE\nGW 1 1 0 0 0 0 0 1 .001\nGE\nEX 0 1 1 0 1\nFR 0 1 0 0 14\n")
            .expand()
            .unwrap()
    }

    #[test]
    fn finds_crossing_near_and_long() {
        let s = vec![
            seg(1, 1, [0.0, -1.0, 5.0], [0.0, 1.0, 5.0], 0.001),
            // Crosses wire 1 in the middle.
            seg(2, 2, [-1.0, 0.0, 5.0], [1.0, 0.0, 5.0], 0.001),
            // Ends 5 mm short of wire 1's end.
            seg(3, 3, [0.0, 1.005, 5.0], [0.0, 1.5, 5.0], 0.001),
        ];
        let issues = check(&s, &deck(), 30.0, false);
        let text: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("cross")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("nearly meet")), "{text:?}");
        // 2 m segments at 30 MHz (λ = 10 m) are 0.2 λ.
        assert!(text.iter().any(|t| t.contains("too long")), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("Nothing feeds")));
    }

    #[test]
    fn a_clean_dipole_passes() {
        let s: Vec<Segment> = (0..11)
            .map(|k| {
                let y = -5.0 + k as f64 * 10.0 / 11.0;
                let mut s = seg(
                    k + 1,
                    1,
                    [0.0, y, 10.0],
                    [0.0, y + 10.0 / 11.0, 10.0],
                    0.001,
                );
                // Joined in a line, as NEC reports it.
                let n = i64::from(k) + 1;
                s.connections = [
                    if k > 0 { n - 1 } else { 0 },
                    if k < 10 { n + 1 } else { 0 },
                ];
                s
            })
            .collect();
        let deck = crate::deck::Deck::parse(
            "CE\nGW 1 11 0 -5 10 0 5 10 .001\nGE\nEX 0 1 6 0 1\nFR 0 1 0 0 14\n",
        )
        .expand()
        .unwrap();
        assert_eq!(check(&s, &deck, 14.2, true), vec![]);
        let deck = crate::deck::Deck::parse(
            "CE\nGW 1 11 0 -5 10 0 5 10 .001\nGE\nEX 0 4 6 0 1\nFR 0 1 0 0 14\n",
        )
        .expand()
        .unwrap();
        let issues = check(&s, &deck, 14.2, true);
        assert!(
            issues[0].message.contains("wire 4, which doesn't exist"),
            "{issues:?}"
        );
    }
}
