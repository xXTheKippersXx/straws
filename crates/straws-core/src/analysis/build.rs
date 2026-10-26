//! The build sheet: what to cut and buy to make the model, worked out from
//! its segments (so arcs and helices have their real length too). Lengths
//! are the modelled conductors, end to end: overlaps at tube joints,
//! clamps, insulators and the feed's own hardware aren't in a NEC model and
//! are left to the builder.

use crate::deck::edit::wires;
use crate::deck::material::material_of;
use crate::deck::{Deck, NumericDeck};
use crate::output::Segment;

type V3 = [f64; 3];

/// One conductor: every segment of one tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub tag: i64,
    /// From the wire's comment, its curve's name, or "wire N".
    pub name: String,
    /// Conductor length along the wire (m).
    pub length: f64,
    pub diameter: f64,
    /// The material's name ("Perfect conductor" if none is set).
    pub material: String,
    /// Tube wall thickness (m), `None` for solid wire.
    pub wall: Option<f64>,
    /// All its segments point the same way.
    pub straight: bool,
    /// Middle of its ends (straight pieces) or of its segments.
    pub centre: V3,
    /// Unit direction (straight pieces).
    pub direction: V3,
}

/// Identical pieces, cut alike.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    pub count: usize,
    pub length: f64,
    pub diameter: f64,
    pub material: String,
    pub wall: Option<f64>,
    pub straight: bool,
    pub names: Vec<String>,
}

/// Total length of one material and size, to buy.
#[derive(Debug, Clone, PartialEq)]
pub struct Stock {
    pub material: String,
    pub diameter: f64,
    pub wall: Option<f64>,
    pub total: f64,
}

/// Where parallel elements sit along the line through their middles (a
/// boom), first at 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Positions {
    /// (names, position along the line (m), total length of those pieces).
    pub rows: Vec<(String, f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub pieces: Vec<Piece>,
    pub cuts: Vec<Cut>,
    pub stock: Vec<Stock>,
    pub positions: Option<Positions>,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// A material's name for buying: a listed one by name; any other
/// conductivity with the nearest listed metal (within a factor of 1.3).
fn material_name(sigma: Option<f64>, name: &str) -> String {
    let Some(sigma) = sigma else {
        return "not set (a perfect conductor in the model)".to_string();
    };
    if crate::deck::material::MATERIALS
        .iter()
        .any(|(n, _)| *n == name)
    {
        return name.to_string();
    }
    crate::deck::material::MATERIALS
        .iter()
        .map(|(n, s)| (n, (sigma / s).ln().abs()))
        .filter(|(_, d)| *d < 1.3f64.ln())
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or_else(|| name.to_string(), |(n, _)| format!("{name} (about {n})"))
}

/// The build sheet for `segments` (helper wires and `skip_tags`, e.g. a
/// modelled feedline shield, left out).
pub fn build_sheet(
    segments: &[Segment],
    deck: &Deck,
    numeric: &NumericDeck,
    skip_tags: &[i64],
) -> Sheet {
    // Names: the GW line's comment, or the curve's name.
    let gw = wires(deck, numeric);
    let curves = crate::deck::curves::curves(deck);
    let name_of = |tag: i64| -> String {
        gw.iter()
            .find(|w| w.tag == tag)
            .and_then(|w| deck.lines.get(w.line)?.comment.clone())
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && !c.starts_with("straws:"))
            .or_else(|| curves.iter().find(|c| c.tag == tag).map(|c| c.name.clone()))
            .unwrap_or_else(|| format!("wire {tag}"))
    };
    let mut tags: Vec<i64> = segments.iter().map(|s| i64::from(s.tag)).collect();
    tags.sort_unstable();
    tags.dedup();
    let pieces: Vec<Piece> = tags
        .into_iter()
        .filter(|t| !numeric.helper_tags.contains(t) && !skip_tags.contains(t))
        .map(|tag| {
            let segs: Vec<&Segment> = segments
                .iter()
                .filter(|s| i64::from(s.tag) == tag)
                .collect();
            // NEC prints segment lengths to five figures (0.8 mm short of
            // exact on a 10 m element), so GW wires take theirs from the
            // deck's coordinates; only curves are summed from segments.
            let cards: Vec<_> = gw.iter().filter(|w| w.tag == tag).collect();
            let seg_length: f64 = segs.iter().map(|s| s.length).sum();
            let length = if cards.is_empty() {
                seg_length
            } else {
                cards.iter().map(|w| w.length()).sum()
            };
            let d0 = segs[0].direction();
            let straight = segs.iter().all(|s| dot(s.direction(), d0).abs() > 0.9999);
            let (centre, direction) = match cards.as_slice() {
                [w] if !w.moved && w.length() > 0.0 => (
                    [0, 1, 2].map(|k| (w.a[k] + w.b[k]) / 2.0),
                    scale(sub(w.b, w.a), 1.0 / w.length()),
                ),
                _ => {
                    let total = seg_length.max(1e-300);
                    let mut c = [0.0; 3];
                    for s in &segs {
                        for (k, v) in c.iter_mut().enumerate() {
                            *v += s.centre[k] * s.length / total;
                        }
                    }
                    (c, d0)
                }
            };
            let material = material_of(numeric, tag);
            Piece {
                tag,
                name: name_of(tag),
                length,
                diameter: 2.0 * segs[0].radius,
                material: material_name(material.conductivity, &material.name()),
                wall: material.wall,
                straight,
                centre,
                direction,
            }
        })
        .collect();

    // Identical pieces together (to half a millimetre and 10 µm across).
    let mut cuts: Vec<Cut> = Vec::new();
    for p in &pieces {
        let same = |c: &Cut| {
            (c.length - p.length).abs() < 0.0005
                && (c.diameter - p.diameter).abs() < 1e-5
                && c.material == p.material
                && c.wall == p.wall
                && c.straight == p.straight
        };
        match cuts.iter_mut().find(|c| same(c)) {
            Some(c) => {
                c.count += 1;
                c.names.push(p.name.clone());
            }
            None => cuts.push(Cut {
                count: 1,
                length: p.length,
                diameter: p.diameter,
                material: p.material.clone(),
                wall: p.wall,
                straight: p.straight,
                names: vec![p.name.clone()],
            }),
        }
    }
    cuts.sort_by(|a, b| b.length.total_cmp(&a.length));

    let mut stock: Vec<Stock> = Vec::new();
    for p in &pieces {
        match stock.iter_mut().find(|s| {
            s.material == p.material && (s.diameter - p.diameter).abs() < 1e-5 && s.wall == p.wall
        }) {
            Some(s) => s.total += p.length,
            None => stock.push(Stock {
                material: p.material.clone(),
                diameter: p.diameter,
                wall: p.wall,
                total: p.length,
            }),
        }
    }
    stock.sort_by(|a, b| b.total.total_cmp(&a.total));

    let positions = positions(&pieces);
    Sheet {
        pieces,
        cuts,
        stock,
        positions,
    }
}

/// Element positions: the straight pieces that share the commonest
/// direction, their middles taken across that direction (so the two halves
/// of a split driven element count as one place), when there are at least
/// three places and they lie on a line (within 5% of its length) across the
/// elements.
fn positions(pieces: &[Piece]) -> Option<Positions> {
    let straight: Vec<&Piece> = pieces.iter().filter(|p| p.straight).collect();
    let parallel_to = |d: V3| -> Vec<&Piece> {
        straight
            .iter()
            .copied()
            .filter(|p| dot(p.direction, d).abs() > 0.9998)
            .collect()
    };
    let group = straight
        .iter()
        .map(|p| parallel_to(p.direction))
        .max_by_key(|g| g.len())?;
    let d = group[0].direction;
    // Middles across the elements, gathered where they coincide (1 mm).
    let mut places: Vec<(V3, Vec<&Piece>)> = Vec::new();
    for p in group {
        let across = sub(p.centre, scale(d, dot(p.centre, d)));
        match places
            .iter_mut()
            .find(|(c, _)| norm(sub(*c, across)) < 0.001)
        {
            Some((_, ps)) => ps.push(p),
            None => places.push((across, vec![p])),
        }
    }
    if places.len() < 3 {
        return None;
    }
    // The line through them: from the first to the farthest from it.
    let first = places[0].0;
    let far = places
        .iter()
        .map(|(c, _)| *c)
        .max_by(|a, b| norm(sub(*a, first)).total_cmp(&norm(sub(*b, first))))?;
    let span = norm(sub(far, first));
    if span < 1e-6 {
        return None;
    }
    let axis = scale(sub(far, first), 1.0 / span);
    let off_line = places.iter().map(|(c, _)| {
        let v = sub(*c, first);
        norm(sub(v, scale(axis, dot(v, axis))))
    });
    if off_line.fold(0.0, f64::max) > 0.05 * span + 0.001 {
        return None;
    }
    let mut rows: Vec<(String, f64, f64)> = places
        .iter()
        .map(|(c, ps)| {
            let names: Vec<&str> = ps.iter().map(|p| p.name.as_str()).collect();
            (
                names.join(" + "),
                dot(sub(*c, first), axis),
                ps.iter().map(|p| p.length).sum(),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    let start = rows[0].1;
    for r in &mut rows {
        r.1 -= start;
    }
    Some(Positions { rows })
}
