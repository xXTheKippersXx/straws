//! STEP files (ISO 10303-21, AP214): the wires written out as solid tubes
//! for CAD, and the straight round tubes of a solid model read back in as
//! wires (their axis and outer radius).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::build::Sheet;
use crate::output::Segment;
use crate::stl::{Parts, Tube};

type P3 = [f64; 3];

// ---------------------------------------------------------------- writing

/// A straight solid rod, or a hollow tube, to write (metres).
#[derive(Debug, Clone, PartialEq)]
pub struct Rod {
    pub name: String,
    pub a: P3,
    pub b: P3,
    pub radius: f64,
    /// The inside radius of a hollow tube.
    pub inner: Option<f64>,
}

/// One body of the STEP file: wires joined to each other (a driven
/// element, a reflector), kept apart from the others in CAD.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub name: String,
    pub rods: Vec<Rod>,
    /// Wire meshes written as solid sheets.
    pub sheets: Vec<Mesh>,
}

/// A wire mesh (a plate, corner reflector, dish or grid) to write as a
/// solid sheet of this thickness, its middle where the wires are.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub name: String,
    /// Its wires' ends (metres).
    pub wires: Vec<(P3, P3)>,
    /// Metres.
    pub thickness: f64,
    /// The wires' radius (metres), for any that stick out past the
    /// mesh's last whole cell: they're written as tubes.
    pub radius: f64,
}

/// A build sheet's pieces as bodies, one for each set of wires joined to
/// each other (NEC's own junctions decide). A straight wire is one rod end
/// to end (from the deck's exact coordinates where it has them), a curve
/// one rod a segment. Everything stays where it is in the model.
pub fn bodies(sheet: &Sheet, segments: &[Segment]) -> Vec<Body> {
    // Tags joined through a segment junction are one body.
    let tags: Vec<i64> = sheet.pieces.iter().map(|p| p.tag).collect();
    let slot: HashMap<i64, usize> = tags.iter().enumerate().map(|(k, t)| (*t, k)).collect();
    let tag_of: HashMap<u32, i64> = segments
        .iter()
        .map(|s| (s.number, i64::from(s.tag)))
        .collect();
    let mut parent: Vec<usize> = (0..tags.len()).collect();
    for s in segments {
        let Some(&a) = slot.get(&i64::from(s.tag)) else {
            continue;
        };
        for c in s.connections {
            let other = c.unsigned_abs() as u32;
            if other == 0 || other == s.number {
                continue; // a free end, or the ground
            }
            if let Some(&b) = tag_of.get(&other).and_then(|t| slot.get(t)) {
                union(&mut parent, a, b);
            }
        }
    }
    let rods = rods(sheet, segments);
    let mut out: Vec<(usize, Vec<String>, Vec<Rod>)> = Vec::new();
    for (k, p) in sheet.pieces.iter().enumerate() {
        let root = find(&mut parent, k);
        let mine: Vec<Rod> = rods
            .iter()
            .filter(|r| r.0 == p.tag)
            .map(|r| r.1.clone())
            .collect();
        match out.iter_mut().find(|b| b.0 == root) {
            Some(b) => {
                b.1.push(p.name.clone());
                b.2.extend(mine);
            }
            None => out.push((root, vec![p.name.clone()], mine)),
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(k, (_, names, rods))| {
            // Only names the user gave count ("wire 7" says nothing).
            let given: Vec<&String> = names
                .iter()
                .filter(|n| {
                    !n.strip_prefix("wire ")
                        .is_some_and(|t| t.parse::<i64>().is_ok())
                })
                .collect();
            let part = k + 1;
            let count = names.len();
            Body {
                name: match (names.as_slice(), given.as_slice()) {
                    ([one], _) => one.clone(),
                    (_, []) => format!("part {part} ({count} wires)"),
                    (_, [a, b]) if count == 2 => format!("part {part}: {a}, {b}"),
                    (_, [a, ..]) => format!("part {part}: {a} and {} more wires", count - 1),
                },
                rods,
                sheets: Vec::new(),
            }
        })
        .collect()
}

/// A build sheet's pieces as (tag, rod).
fn rods(sheet: &Sheet, segments: &[Segment]) -> Vec<(i64, Rod)> {
    let mut out = Vec::new();
    for p in &sheet.pieces {
        let radius = p.diameter / 2.0;
        let inner = p.wall.map(|w| radius - w).filter(|r| *r > 0.0);
        if p.straight {
            let half = p.length / 2.0;
            out.push((
                p.tag,
                Rod {
                    name: p.name.clone(),
                    a: [0, 1, 2].map(|i| p.centre[i] - half * p.direction[i]),
                    b: [0, 1, 2].map(|i| p.centre[i] + half * p.direction[i]),
                    radius,
                    inner,
                },
            ));
            continue;
        }
        let segs = segments.iter().filter(|s| i64::from(s.tag) == p.tag);
        for (k, s) in segs.enumerate() {
            let (a, b) = s.ends();
            out.push((
                p.tag,
                Rod {
                    name: format!("{} piece {}", p.name, k + 1),
                    a,
                    b,
                    radius: s.radius,
                    inner: p.wall.map(|w| s.radius - w).filter(|r| *r > 0.0),
                },
            ));
        }
    }
    out
}

/// The bodies as one STEP file in millimetres, as CAD programs expect:
/// one body alone is a part; several are an assembly of parts, each in
/// place (no part is moved). Each rod is a solid of its body; rods with no
/// length or thickness are left out.
pub fn write(bodies: &[Body], name: &str) -> String {
    let mut w = Writer {
        out: String::new(),
        next: 1,
    };
    let title = text(name);
    let app = w.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
    w.add(format!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app})"
    ));
    let pc = w.add(format!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
    let pdc = w.add(format!(
        "PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"
    ));
    let product = |w: &mut Writer, title: &str| {
        let product = w.add(format!("PRODUCT({title},{title},'',(#{pc}))"));
        w.add(format!(
            "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{product}))"
        ));
        let formation = w.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{product})"));
        let pd = w.add(format!(
            "PRODUCT_DEFINITION('design','',#{formation},#{pdc})"
        ));
        let pds = w.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{pd})"));
        (pd, pds)
    };
    let mm = w.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))");
    let rad = w.add("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))");
    let sr = w.add("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())");
    let accuracy = w.add(format!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#{mm},'distance_accuracy_value','confusion accuracy')"
    ));
    let context = w.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{accuracy})) \
         GLOBAL_UNIT_ASSIGNED_CONTEXT((#{mm},#{rad},#{sr})) REPRESENTATION_CONTEXT('','3D'))"
    ));
    // A body's shape: its solids, about the same origin as the assembly.
    let shape = |w: &mut Writer, body: &Body, title: &str| {
        let origin = w.placement([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let mut items = vec![origin];
        for rod in &body.rods {
            if let Some(solid) = w.rod(rod) {
                items.push(solid);
            }
        }
        for mesh in &body.sheets {
            if let Ok(solid) = sheet_solid(mesh) {
                items.push(w.polyhedron(&solid, &mesh.name));
                for (n, &k) in solid.loose.iter().enumerate() {
                    let (a, b) = mesh.wires[k];
                    let rod = Rod {
                        name: format!("{} wire {}", mesh.name, n + 1),
                        a,
                        b,
                        radius: mesh.radius,
                        inner: None,
                    };
                    if let Some(solid) = w.rod(&rod) {
                        items.push(solid);
                    }
                }
            }
        }
        let list = items
            .iter()
            .map(|i| format!("#{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let rep = w.add(format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION({title},({list}),#{context})"
        ));
        (rep, origin)
    };
    let bodies: Vec<&Body> = bodies
        .iter()
        .filter(|b| !b.rods.is_empty() || b.sheets.iter().any(|m| sheet_solid(m).is_ok()))
        .collect();
    if let [body] = bodies.as_slice() {
        let (_, pds) = product(&mut w, &title);
        let (rep, _) = shape(&mut w, body, &title);
        w.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{rep})"));
    } else {
        let (top, top_pds) = product(&mut w, &title);
        let top_origin = w.placement([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let top_rep = w.add(format!(
            "SHAPE_REPRESENTATION({title},(#{top_origin}),#{context})"
        ));
        w.add(format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{top_pds},#{top_rep})"
        ));
        for (k, body) in bodies.iter().enumerate() {
            let body_title = text(&body.name);
            let (pd, pds) = product(&mut w, &body_title);
            let (rep, origin) = shape(&mut w, body, &body_title);
            w.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{rep})"));
            // Placed with its origin on the assembly's: exactly where it is.
            let usage = w.add(format!(
                "NEXT_ASSEMBLY_USAGE_OCCURRENCE('{}',{body_title},'',#{top},#{pd},$)",
                k + 1
            ));
            let usage_shape = w.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{usage})"));
            let moved = w.add(format!(
                "ITEM_DEFINED_TRANSFORMATION('','',#{origin},#{top_origin})"
            ));
            let rr = w.add(format!(
                "(REPRESENTATION_RELATIONSHIP('','',#{rep},#{top_rep}) \
                 REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{moved}) \
                 SHAPE_REPRESENTATION_RELATIONSHIP())"
            ));
            w.add(format!(
                "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{rr},#{usage_shape})"
            ));
        }
    }

    let mut file = String::from("ISO-10303-21;\nHEADER;\n");
    let _ = writeln!(
        file,
        "FILE_DESCRIPTION(('Antenna wires as solid tubes'),'2;1');\n\
         FILE_NAME({title},'{}',(''),(''),'STRAWS {}','STRAWS','');\n\
         FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\n\
         ENDSEC;\nDATA;",
        timestamp(),
        env!("CARGO_PKG_VERSION")
    );
    file.push_str(&w.out);
    file.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    file
}

struct Writer {
    out: String,
    next: u64,
}

/// One cylinder's topology: its surface, the circles at each end and the
/// seam line joining them.
struct Wall {
    surface: u64,
    bottom: u64,
    top: u64,
    seam: u64,
}

impl Writer {
    fn add(&mut self, entity: impl AsRef<str>) -> u64 {
        let id = self.next;
        self.next += 1;
        let _ = writeln!(self.out, "#{id}={};", entity.as_ref());
        id
    }

    fn point(&mut self, p: P3) -> u64 {
        self.add(format!(
            "CARTESIAN_POINT('',({},{},{}))",
            real(p[0], 9),
            real(p[1], 9),
            real(p[2], 9)
        ))
    }

    fn direction(&mut self, d: P3) -> u64 {
        self.add(format!(
            "DIRECTION('',({},{},{}))",
            real(d[0], 15),
            real(d[1], 15),
            real(d[2], 15)
        ))
    }

    fn placement(&mut self, at: P3, axis: P3, x: P3) -> u64 {
        let p = self.point(at);
        let z = self.direction(axis);
        let x = self.direction(x);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{z},#{x})"))
    }

    fn edge(&mut self, edge: u64, forward: bool) -> String {
        let id = self.add(format!(
            "ORIENTED_EDGE('',*,*,#{edge},{})",
            if forward { ".T." } else { ".F." }
        ));
        format!("#{id}")
    }

    fn bound(&mut self, edges: &[String], outer: bool) -> u64 {
        let lp = self.add(format!("EDGE_LOOP('',({}))", edges.join(",")));
        self.add(format!(
            "{}('',#{lp},.T.)",
            if outer {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            }
        ))
    }

    /// The cylinder of radius r (mm) from a to b along d (unit, length len).
    fn wall(&mut self, a: P3, b: P3, d: P3, x: P3, r: f64, len: f64) -> Wall {
        let at_a = self.placement(a, d, x);
        let surface = self.add(format!("CYLINDRICAL_SURFACE('',#{at_a},{})", real(r, 6)));
        let pa = self.point([0, 1, 2].map(|i| a[i] + r * x[i]));
        let pb = self.point([0, 1, 2].map(|i| b[i] + r * x[i]));
        let va = self.add(format!("VERTEX_POINT('',#{pa})"));
        let vb = self.add(format!("VERTEX_POINT('',#{pb})"));
        let ca = self.add(format!("CIRCLE('',#{at_a},{})", real(r, 6)));
        let at_b = self.placement(b, d, x);
        let cb = self.add(format!("CIRCLE('',#{at_b},{})", real(r, 6)));
        let bottom = self.add(format!("EDGE_CURVE('',#{va},#{va},#{ca},.T.)"));
        let top = self.add(format!("EDGE_CURVE('',#{vb},#{vb},#{cb},.T.)"));
        let dir = self.direction(d);
        let vector = self.add(format!("VECTOR('',#{dir},{})", real(len, 6)));
        let line = self.add(format!("LINE('',#{pa},#{vector})"));
        let seam = self.add(format!("EDGE_CURVE('',#{va},#{vb},#{line},.T.)"));
        Wall {
            surface,
            bottom,
            top,
            seam,
        }
    }

    /// A closed solid of flat faces: shared corners and straight edges.
    fn polyhedron(&mut self, solid: &Polyhedron, name: &str) -> u64 {
        let mut vertices: HashMap<usize, u64> = HashMap::new();
        let mut edges: HashMap<(usize, usize), u64> = HashMap::new();
        let mut faces = Vec::new();
        for face in &solid.faces {
            let mut bounds = Vec::new();
            for (k, ring) in face.loops.iter().enumerate() {
                let mut oriented = Vec::new();
                for (i, &u) in ring.iter().enumerate() {
                    let v = ring[(i + 1) % ring.len()];
                    let key = (u.min(v), u.max(v));
                    let edge = match edges.get(&key) {
                        Some(e) => *e,
                        None => {
                            let mut vertex = |w: &mut Writer, i: usize| {
                                *vertices.entry(i).or_insert_with(|| {
                                    let pt = w.point(solid.points[i]);
                                    w.add(format!("VERTEX_POINT('',#{pt})"))
                                })
                            };
                            let (va, vb) = (vertex(self, key.0), vertex(self, key.1));
                            let (pa, pb) = (solid.points[key.0], solid.points[key.1]);
                            let ab = sub(pb, pa);
                            let len = length(ab);
                            let start = self.point(pa);
                            let dir = self.direction(scale(ab, 1.0 / len.max(1e-300)));
                            let vector = self.add(format!("VECTOR('',#{dir},{})", real(len, 9)));
                            let line = self.add(format!("LINE('',#{start},#{vector})"));
                            let e = self.add(format!("EDGE_CURVE('',#{va},#{vb},#{line},.T.)"));
                            edges.insert(key, e);
                            e
                        }
                    };
                    oriented.push(self.edge(edge, u == key.0));
                }
                bounds.push(self.bound(&oriented, k == 0));
            }
            let first = solid.points[face.loops[0][0]];
            let next = solid.points[face.loops[0][1]];
            let x = sub(next, first);
            let x = unit(sub(x, scale(face.normal, dot(x, face.normal))));
            let at = self.placement(first, face.normal, x);
            let plane = self.add(format!("PLANE('',#{at})"));
            let list = bounds
                .iter()
                .map(|b| format!("#{b}"))
                .collect::<Vec<_>>()
                .join(",");
            faces.push(self.add(format!("ADVANCED_FACE('',({list}),#{plane},.T.)")));
        }
        let list = faces
            .iter()
            .map(|f| format!("#{f}"))
            .collect::<Vec<_>>()
            .join(",");
        let shell = self.add(format!("CLOSED_SHELL('',({list}))"));
        self.add(format!("MANIFOLD_SOLID_BREP({},#{shell})", text(name)))
    }

    /// One rod as a closed solid: its side (and inside) walls and two end
    /// caps, which are rings for a hollow tube.
    fn rod(&mut self, rod: &Rod) -> Option<u64> {
        let a = rod.a.map(|v| v * 1000.0);
        let b = rod.b.map(|v| v * 1000.0);
        let r = rod.radius * 1000.0;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = length(ab);
        if !(len > 0.0 && r > 0.0 && len.is_finite() && r.is_finite()) {
            return None;
        }
        let d = ab.map(|v| v / len);
        let x = square_to(d);
        let outer = self.wall(a, b, d, x, r, len);
        let inner = rod
            .inner
            .map(|ri| ri * 1000.0)
            .filter(|ri| *ri > 0.0 && *ri < r)
            .map(|ri| self.wall(a, b, d, x, ri, len));
        let mut faces = Vec::new();
        // Outside: round the bottom, up the seam, back round the top, down
        // the seam (anticlockwise seen from outside).
        let edges = [
            self.edge(outer.bottom, true),
            self.edge(outer.seam, true),
            self.edge(outer.top, false),
            self.edge(outer.seam, false),
        ];
        let fb = self.bound(&edges, true);
        faces.push(self.add(format!("ADVANCED_FACE('',(#{fb}),#{},.T.)", outer.surface)));
        if let Some(inner) = &inner {
            // Inside, facing the axis: the same loop the other way.
            let edges = [
                self.edge(inner.seam, true),
                self.edge(inner.top, true),
                self.edge(inner.seam, false),
                self.edge(inner.bottom, false),
            ];
            let fb = self.bound(&edges, true);
            faces.push(self.add(format!("ADVANCED_FACE('',(#{fb}),#{},.F.)", inner.surface)));
        }
        for (at, rim, hole, up) in [
            (a, outer.bottom, inner.as_ref().map(|w| w.bottom), false),
            (b, outer.top, inner.as_ref().map(|w| w.top), true),
        ] {
            let pl = self.placement(at, d, x);
            let plane = self.add(format!("PLANE('',#{pl})"));
            let e = self.edge(rim, up);
            let mut bounds = vec![self.bound(&[e], true)];
            if let Some(hole) = hole {
                let e = self.edge(hole, !up);
                bounds.push(self.bound(&[e], false));
            }
            let list = bounds
                .iter()
                .map(|b| format!("#{b}"))
                .collect::<Vec<_>>()
                .join(",");
            faces.push(self.add(format!(
                "ADVANCED_FACE('',({list}),#{plane},{})",
                if up { ".T." } else { ".F." }
            )));
        }
        let list = faces
            .iter()
            .map(|f| format!("#{f}"))
            .collect::<Vec<_>>()
            .join(",");
        let shell = self.add(format!("CLOSED_SHELL('',({list}))"));
        Some(self.add(format!("MANIFOLD_SOLID_BREP({},#{shell})", text(&rod.name))))
    }
}

/// A closed solid of flat faces (millimetres): each face an outer loop of
/// corner indices, anticlockwise about its outward normal, then any holes.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyhedron {
    pub points: Vec<P3>,
    pub faces: Vec<PolyFace>,
    /// The mesh's wires that aren't a side of any cell (by index).
    pub loose: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolyFace {
    pub loops: Vec<Vec<usize>>,
    pub normal: P3,
}

/// A wire mesh as a solid sheet (millimetres), or why it can't be one.
/// The mesh's cells are its four-sided loops of wires; the sheet is the
/// surface they make, thickened evenly on both sides (more at folds, so
/// it stays the full thickness). Neighbouring cells in one plane become
/// one face, so a flat plate is a plain slab.
pub fn sheet_solid(mesh: &Mesh) -> Result<Polyhedron, String> {
    let ends: Vec<P3> = mesh
        .wires
        .iter()
        .flat_map(|(a, b)| [*a, *b])
        .map(|p| p.map(|v| v * 1000.0))
        .collect();
    if ends.is_empty() {
        return Err("it has no wires".into());
    }
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &ends {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let size = length(sub(hi, lo)).max(1e-9);
    let t = mesh.thickness * 1000.0;
    if !(t > 0.0 && t.is_finite()) {
        return Err("the thickness must be more than zero".into());
    }
    let id = crate::dxf::merge_points(&ends, 1e-7 * size);
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut points: Vec<P3> = Vec::new();
    let mut node = |k: usize| {
        *index.entry(id[k]).or_insert_with(|| {
            points.push(ends[k]);
            points.len() - 1
        })
    };
    let mut links: HashSet<(usize, usize)> = HashSet::new();
    let mut wire_nodes = Vec::with_capacity(mesh.wires.len());
    for k in 0..mesh.wires.len() {
        let (a, b) = (node(2 * k), node(2 * k + 1));
        wire_nodes.push((a.min(b), a.max(b)));
        if a != b {
            links.insert((a.min(b), a.max(b)));
        }
    }
    let n = points.len();
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &links {
        near[a].push(b);
        near[b].push(a);
    }
    for v in &mut near {
        v.sort_unstable();
    }
    let linked = |a: usize, b: usize| links.contains(&(a.min(b), a.max(b)));
    // Cells: loops a-b-c-d of four wires with no wire across them.
    let mut seen = HashSet::new();
    let mut cells: Vec<[usize; 4]> = Vec::new();
    for a in 0..n {
        for (i, &b) in near[a].iter().enumerate() {
            for &d in &near[a][i + 1..] {
                for &c in &near[b] {
                    if c == a || c == d || !linked(c, d) || linked(a, c) || linked(b, d) {
                        continue;
                    }
                    let mut key = [a, b, c, d];
                    key.sort_unstable();
                    if seen.insert(key) {
                        cells.push([a, b, c, d]);
                    }
                }
            }
        }
    }
    if cells.is_empty() {
        return Err("its wires don't make any four-sided cells".into());
    }
    // Every cell turned the same way round, as neighbours across a shared
    // wire must be.
    let mut users: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (k, c) in cells.iter().enumerate() {
        for i in 0..4 {
            let (u, v) = (c[i], c[(i + 1) % 4]);
            users.entry((u.min(v), u.max(v))).or_default().push(k);
        }
    }
    if users.values().any(|u| u.len() > 2) {
        return Err("more than two of its cells meet along one wire".into());
    }
    let runs =
        |c: &[usize; 4], u: usize, v: usize| (0..4).any(|i| c[i] == u && c[(i + 1) % 4] == v);
    let mut placed = vec![false; cells.len()];
    for start in 0..cells.len() {
        if placed[start] {
            continue;
        }
        placed[start] = true;
        let mut queue = vec![start];
        while let Some(k) = queue.pop() {
            for i in 0..4 {
                let (u, v) = (cells[k][i], cells[k][(i + 1) % 4]);
                for &j in &users[&(u.min(v), u.max(v))] {
                    if j == k {
                        continue;
                    }
                    let same_way = runs(&cells[j], u, v);
                    if placed[j] {
                        if same_way {
                            return Err("it twists like a Möbius strip".into());
                        }
                        continue;
                    }
                    if same_way {
                        cells[j].reverse();
                    }
                    placed[j] = true;
                    queue.push(j);
                }
            }
        }
    }
    // How far each corner moves out: along the mean of its cells' normals,
    // further at a fold (a mitre).
    let normals: Vec<P3> = cells
        .iter()
        .map(|c| unit(newell(&c.map(|i| points[i]))))
        .collect();
    let mut sum = vec![[0.0; 3]; n];
    for (c, nm) in cells.iter().zip(&normals) {
        for &i in c {
            sum[i] = add(sum[i], *nm);
        }
    }
    let mut out_by = vec![[0.0; 3]; n];
    for (i, s) in sum.iter().enumerate() {
        if length(*s) == 0.0 {
            continue;
        }
        let m = unit(*s);
        let least = cells
            .iter()
            .zip(&normals)
            .filter(|(c, _)| c.contains(&i))
            .map(|(_, nm)| dot(*nm, m))
            .fold(1.0_f64, f64::min)
            .max(0.25);
        out_by[i] = scale(m, t / 2.0 / least);
    }
    let mut corners: Vec<P3> = (0..n).map(|i| add(points[i], out_by[i])).collect();
    corners.extend((0..n).map(|i| sub(points[i], out_by[i])));
    // The faces: the cells on top, the same underneath the other way
    // round, and a wall along each edge of the sheet.
    let mut polys: Vec<Vec<usize>> = Vec::new();
    for c in &cells {
        polys.push(c.to_vec());
        polys.push(c.iter().rev().map(|i| i + n).collect());
        for i in 0..4 {
            let (u, v) = (c[i], c[(i + 1) % 4]);
            if users[&(u.min(v), u.max(v))].len() == 1 {
                polys.push(vec![v, u, u + n, v + n]);
            }
        }
    }
    let tol = 1e-9 * size + 1e-7;
    // Four corners not in one plane: two triangles.
    let mut flat: Vec<Vec<usize>> = Vec::new();
    for poly in polys {
        if is_planar(&poly, &corners, tol) {
            flat.push(poly);
        } else {
            flat.push(vec![poly[0], poly[1], poly[2]]);
            flat.push(vec![poly[0], poly[2], poly[3]]);
        }
    }
    let loose = wire_nodes
        .iter()
        .enumerate()
        .filter(|(_, e)| e.0 != e.1 && !users.contains_key(e))
        .map(|(k, _)| k)
        .collect();
    Ok(Polyhedron {
        faces: merge_coplanar(&flat, &corners, tol),
        points: corners,
        loose,
    })
}

/// A polygon's area-weighted normal (Newell's method).
fn newell(p: &[P3]) -> P3 {
    let mut nm = [0.0; 3];
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        nm[0] += (a[1] - b[1]) * (a[2] + b[2]);
        nm[1] += (a[2] - b[2]) * (a[0] + b[0]);
        nm[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    nm
}

fn is_planar(poly: &[usize], corners: &[P3], tol: f64) -> bool {
    let pts: Vec<P3> = poly.iter().map(|&i| corners[i]).collect();
    let nm = newell(&pts);
    if length(nm) == 0.0 {
        return false;
    }
    let nm = unit(nm);
    pts.iter().all(|p| dot(sub(*p, pts[0]), nm).abs() <= tol)
}

/// Neighbouring polygons in one plane joined into faces (outer loop, then
/// holes). A face whose outline touches itself at a corner stays apart.
fn merge_coplanar(polys: &[Vec<usize>], corners: &[P3], tol: f64) -> Vec<PolyFace> {
    let normal_of =
        |p: &Vec<usize>| unit(newell(&p.iter().map(|&i| corners[i]).collect::<Vec<_>>()));
    let normals: Vec<P3> = polys.iter().map(normal_of).collect();
    let mut parent: Vec<usize> = (0..polys.len()).collect();
    let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (k, p) in polys.iter().enumerate() {
        for i in 0..p.len() {
            let (u, v) = (p[i], p[(i + 1) % p.len()]);
            by_edge.entry((u.min(v), u.max(v))).or_default().push(k);
        }
    }
    for sharing in by_edge.values() {
        for (i, &a) in sharing.iter().enumerate() {
            for &b in &sharing[i + 1..] {
                let same_plane = dot(normals[a], normals[b]) > 1.0 - 1e-12
                    && dot(sub(corners[polys[b][0]], corners[polys[a][0]]), normals[a]).abs()
                        <= tol;
                if same_plane {
                    union(&mut parent, a, b);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for k in 0..polys.len() {
        let r = find(&mut parent, k);
        groups.entry(r).or_default().push(k);
    }
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort_unstable();
    let mut out = Vec::new();
    for r in roots {
        let members = &groups[&r];
        let alone = || {
            members
                .iter()
                .map(|&k| PolyFace {
                    loops: vec![polys[k].clone()],
                    normal: normals[k],
                })
                .collect::<Vec<_>>()
        };
        if members.len() == 1 {
            out.extend(alone());
            continue;
        }
        // The group's outline: its edges not shared inside it, chained.
        let mut directed: HashSet<(usize, usize)> = HashSet::new();
        for &k in members {
            let p = &polys[k];
            for i in 0..p.len() {
                directed.insert((p[i], p[(i + 1) % p.len()]));
            }
        }
        let mut next: HashMap<usize, usize> = HashMap::new();
        let mut pinched = false;
        for &(u, v) in &directed {
            if !directed.contains(&(v, u)) && next.insert(u, v).is_some() {
                pinched = true;
            }
        }
        if pinched || next.is_empty() {
            out.extend(alone());
            continue;
        }
        let mut loops: Vec<Vec<usize>> = Vec::new();
        let mut starts: Vec<usize> = next.keys().copied().collect();
        starts.sort_unstable();
        let mut used = HashSet::new();
        let mut broken = false;
        for s0 in starts {
            if used.contains(&s0) {
                continue;
            }
            let mut ring = vec![s0];
            used.insert(s0);
            let mut at = s0;
            loop {
                let Some(&v) = next.get(&at) else {
                    broken = true;
                    break;
                };
                if v == s0 {
                    break;
                }
                if !used.insert(v) {
                    broken = true;
                    break;
                }
                ring.push(v);
                at = v;
            }
            loops.push(ring);
        }
        if broken {
            out.extend(alone());
            continue;
        }
        let nm = normals[members[0]];
        let area = |l: &Vec<usize>| {
            dot(
                newell(&l.iter().map(|&i| corners[i]).collect::<Vec<_>>()),
                nm,
            )
        };
        // The outer loop goes round anticlockwise (positive area).
        loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
        if area(&loops[0]) <= 0.0 {
            out.extend(alone());
            continue;
        }
        out.push(PolyFace { loops, normal: nm });
    }
    out
}

/// A number as a STEP real (always with a decimal point), to `places`
/// decimals without trailing zeros.
fn real(v: f64, places: usize) -> String {
    let mut s = format!("{v:.places$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
    } else {
        s.push('.');
    }
    if s == "-0." {
        s = "0.".into();
    }
    s
}

/// A STEP string: quotes doubled, backslashes doubled, other characters
/// than printable ASCII as \X2\ UTF-16 hex.
fn text(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                out.push_str("\\X2\\");
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(out, "{u:04X}");
                }
                out.push_str("\\X0\\");
            }
        }
    }
    out.push('\'');
    out
}

/// Now in UTC as an ISO 8601 date and time.
fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

// ---------------------------------------------------------------- reading

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Ref(u64),
    Num(f64),
    Str(String),
    Enum(String),
    List(Vec<Value>),
    /// A typed value such as LENGTH_MEASURE(25.4).
    Typed(String, Vec<Value>),
    Null,
}

impl Value {
    fn as_ref(&self) -> Option<u64> {
        match self {
            Value::Ref(r) => Some(*r),
            _ => None,
        }
    }
    fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Typed(_, v) => v.first()?.as_num(),
            _ => None,
        }
    }
    fn as_list(&self) -> &[Value] {
        match self {
            Value::List(l) => l,
            _ => &[],
        }
    }
    fn refs(&self) -> impl Iterator<Item = u64> + '_ {
        self.as_list().iter().filter_map(Value::as_ref)
    }
}

/// One entity: a simple one has one part, a complex one several, e.g.
/// (LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.)).
#[derive(Debug, Clone, PartialEq)]
struct Entity {
    parts: Vec<(String, Vec<Value>)>,
}

impl Entity {
    /// The parameters of the part of this type.
    fn part(&self, kind: &str) -> Option<&[Value]> {
        self.parts
            .iter()
            .find(|p| p.0 == kind)
            .map(|p| p.1.as_slice())
    }
    /// A simple entity's type and parameters.
    fn simple(&self) -> Option<(&str, &[Value])> {
        match self.parts.as_slice() {
            [(k, v)] => Some((k.as_str(), v.as_slice())),
            _ => None,
        }
    }
}

type Entities = HashMap<u64, Entity>;

/// Reads the DATA section of a STEP file.
fn parse(text: &str) -> Result<Entities, String> {
    let start = find_data(text).ok_or(
        "it isn't a STEP file: there's no DATA section (a STEP file starts \"ISO-10303-21;\")",
    )?;
    let mut p = Parser {
        s: text.as_bytes(),
        at: start,
    };
    let mut out = HashMap::new();
    loop {
        p.space();
        if p.at >= p.s.len() {
            break;
        }
        if p.s[p.at] != b'#' {
            // ENDSEC (or another section starting): the data is done.
            break;
        }
        let (id, entity) = p.record().map_err(|e| {
            let line = text.as_bytes()[..p.at.min(text.len())]
                .iter()
                .filter(|c| **c == b'\n')
                .count()
                + 1;
            format!("line {line}: {e}")
        })?;
        out.insert(id, entity);
    }
    Ok(out)
}

/// Where the first DATA section's records start.
fn find_data(text: &str) -> Option<usize> {
    let upper = text.as_bytes();
    let mut k = 0;
    while let Some(off) = text[k..].find("DATA") {
        let at = k + off;
        let before_ok = at == 0 || !upper[at - 1].is_ascii_alphanumeric() && upper[at - 1] != b'_';
        let mut j = at + 4;
        while j < upper.len() && upper[j].is_ascii_whitespace() {
            j += 1;
        }
        if before_ok && j < upper.len() && (upper[j] == b';' || upper[j] == b'(') {
            // DATA; or DATA('name',(schema));
            return text[j..].find(';').map(|e| j + e + 1);
        }
        k = at + 4;
    }
    None
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        loop {
            while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
            if self.s[self.at..].starts_with(b"/*") {
                match self.s[self.at + 2..].windows(2).position(|w| w == b"*/") {
                    Some(e) => self.at += e + 4,
                    None => self.at = self.s.len(),
                }
            } else {
                return;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.space();
        self.s.get(self.at).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c as char))
        }
    }

    fn number_text(&mut self) -> &str {
        let start = self.at;
        while self.at < self.s.len()
            && matches!(
                self.s[self.at],
                b'0'..=b'9' | b'+' | b'-' | b'.' | b'E' | b'e'
            )
        {
            self.at += 1;
        }
        std::str::from_utf8(&self.s[start..self.at]).unwrap_or("")
    }

    fn keyword(&mut self) -> String {
        let start = self.at;
        while self.at < self.s.len()
            && (self.s[self.at].is_ascii_alphanumeric() || self.s[self.at] == b'_')
        {
            self.at += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.at]).to_ascii_uppercase()
    }

    fn record(&mut self) -> Result<(u64, Entity), String> {
        self.expect(b'#')?;
        let id: u64 = self
            .number_text()
            .parse()
            .map_err(|_| "an entity number after '#'".to_string())?;
        self.expect(b'=')?;
        let mut parts = Vec::new();
        if self.peek() == Some(b'(') {
            self.at += 1;
            while self.peek() != Some(b')') {
                if self.at >= self.s.len() {
                    return Err("the file ends inside an entity".into());
                }
                parts.push(self.part()?);
            }
            self.at += 1;
        } else {
            parts.push(self.part()?);
        }
        self.expect(b';')?;
        Ok((id, Entity { parts }))
    }

    fn part(&mut self) -> Result<(String, Vec<Value>), String> {
        self.space();
        let name = self.keyword();
        if name.is_empty() {
            return Err("expected an entity type".into());
        }
        let params = self.params()?;
        Ok((name, params))
    }

    fn params(&mut self) -> Result<Vec<Value>, String> {
        self.expect(b'(')?;
        let mut out = Vec::new();
        if self.peek() == Some(b')') {
            self.at += 1;
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b')') => {
                    self.at += 1;
                    return Ok(out);
                }
                _ => return Err("expected ',' or ')' between parameters".into()),
            }
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'#') => {
                self.at += 1;
                self.number_text()
                    .parse()
                    .map(Value::Ref)
                    .map_err(|_| "an entity number after '#'".to_string())
            }
            Some(b'\'') => {
                self.at += 1;
                let mut bytes = Vec::new();
                loop {
                    match self.s.get(self.at) {
                        None => return Err("the file ends inside a string".into()),
                        Some(b'\'') if self.s.get(self.at + 1) == Some(&b'\'') => {
                            bytes.push(b'\'');
                            self.at += 2;
                        }
                        Some(b'\'') => {
                            self.at += 1;
                            break;
                        }
                        Some(c) => {
                            bytes.push(*c);
                            self.at += 1;
                        }
                    }
                }
                Ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Some(b'"') => {
                // Binary: not needed here.
                self.at += 1;
                while self.at < self.s.len() && self.s[self.at] != b'"' {
                    self.at += 1;
                }
                self.at += 1;
                Ok(Value::Null)
            }
            Some(b'.') if self.s.get(self.at + 1).is_some_and(u8::is_ascii_alphabetic) => {
                self.at += 1;
                let e = self.keyword();
                self.expect(b'.')?;
                Ok(Value::Enum(e))
            }
            Some(b'$' | b'*') => {
                self.at += 1;
                Ok(Value::Null)
            }
            Some(b'(') => {
                let list = self.params()?;
                Ok(Value::List(list))
            }
            Some(b'0'..=b'9' | b'-' | b'+' | b'.') => {
                let t = self.number_text().to_string();
                // STEP writes reals like "1." and "1.E-07"; Rust wants a digit
                // after the point.
                let fixed = t.replace(".E", ".0E").replace(".e", ".0e");
                let fixed = if fixed.ends_with('.') {
                    format!("{fixed}0")
                } else {
                    fixed
                };
                fixed
                    .parse()
                    .map(Value::Num)
                    .map_err(|_| format!("\"{t}\" isn't a number"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let name = self.keyword();
                let params = self.params()?;
                Ok(Value::Typed(name, params))
            }
            _ => Err("expected a value".into()),
        }
    }
}

/// A rigid placement: the columns are x, y, z, then the origin.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame([P3; 4]);

impl Frame {
    const IDENTITY: Frame = Frame([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]]);

    fn point(&self, p: P3) -> P3 {
        let m = &self.0;
        [0, 1, 2].map(|i| m[0][i] * p[0] + m[1][i] * p[1] + m[2][i] * p[2] + m[3][i])
    }
    fn vector(&self, v: P3) -> P3 {
        let m = &self.0;
        [0, 1, 2].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
    }
    /// This after `inner`: inner's coordinates to this frame's outside.
    fn then(&self, inner: &Frame) -> Frame {
        let m = &inner.0;
        Frame([
            self.vector(m[0]),
            self.vector(m[1]),
            self.vector(m[2]),
            self.point(m[3]),
        ])
    }
    /// The inverse (the axes are orthonormal).
    fn inverse(&self) -> Frame {
        let m = &self.0;
        let t = [0, 1, 2].map(|i| [m[0][i], m[1][i], m[2][i]]);
        let o = m[3];
        let back = [0, 1, 2].map(|i| -dot(t[i], o));
        // Columns of the transpose: row i of the original's axes.
        Frame([
            [t[0][0], t[1][0], t[2][0]],
            [t[0][1], t[1][1], t[2][1]],
            [t[0][2], t[1][2], t[2][2]],
            back,
        ])
    }
}

struct Model<'a> {
    e: &'a Entities,
}

impl Model<'_> {
    fn get(&self, id: u64) -> Option<&Entity> {
        self.e.get(&id)
    }
    /// A simple entity of this type.
    fn of(&self, id: u64, kind: &str) -> Option<&[Value]> {
        self.get(id)?.part(kind)
    }

    fn point(&self, id: u64) -> Option<P3> {
        let c = self.of(id, "CARTESIAN_POINT")?.get(1)?.as_list();
        let v: Vec<f64> = c.iter().filter_map(Value::as_num).collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    }

    fn direction(&self, id: u64) -> Option<P3> {
        let c = self.of(id, "DIRECTION")?.get(1)?.as_list();
        let v: Vec<f64> = c.iter().filter_map(Value::as_num).collect();
        (v.len() == 3 && length([v[0], v[1], v[2]]) > 0.0).then(|| unit([v[0], v[1], v[2]]))
    }

    /// AXIS2_PLACEMENT_3D as a frame (axis and reference direction made
    /// square, as the standard says).
    fn frame(&self, id: u64) -> Option<Frame> {
        let p = self.of(id, "AXIS2_PLACEMENT_3D")?;
        let origin = self.point(p.get(1)?.as_ref()?)?;
        let z = p
            .get(2)
            .and_then(Value::as_ref)
            .and_then(|d| self.direction(d))
            .unwrap_or([0.0, 0.0, 1.0]);
        let x0 = p
            .get(3)
            .and_then(Value::as_ref)
            .and_then(|d| self.direction(d))
            .unwrap_or_else(|| square_to(z));
        let x = sub(x0, scale(z, dot(x0, z)));
        let x = if length(x) > 1e-12 {
            unit(x)
        } else {
            square_to(z)
        };
        Some(Frame([x, cross(z, x), z, origin]))
    }

    /// A transformation operator or placement as a frame.
    fn placement_or_operator(&self, id: u64) -> Option<Frame> {
        if let Some(f) = self.frame(id) {
            return Some(f);
        }
        let ent = self.get(id)?;
        let p = ent.part("CARTESIAN_TRANSFORMATION_OPERATOR")?;
        // (name, description, axis1, axis2, local_origin, scale) then axis3.
        let origin = self.point(p.get(4)?.as_ref()?)?;
        let x = p
            .get(2)
            .and_then(Value::as_ref)
            .and_then(|d| self.direction(d));
        let y = p
            .get(3)
            .and_then(Value::as_ref)
            .and_then(|d| self.direction(d));
        let z = ent
            .part("CARTESIAN_TRANSFORMATION_OPERATOR_3D")
            .and_then(|q| q.first())
            .and_then(Value::as_ref)
            .and_then(|d| self.direction(d));
        let z = z.unwrap_or([0.0, 0.0, 1.0]);
        let x = x.unwrap_or_else(|| square_to(z));
        let x = unit(sub(x, scale(z, dot(x, z))));
        let y = y.map_or_else(|| cross(z, x), |_| cross(z, x));
        Some(Frame([x, y, z, origin]))
    }

    /// Metres in one length unit (an SI unit with its prefix, or a unit
    /// defined from one).
    fn unit_metres(&self, id: u64, depth: usize) -> Option<f64> {
        let ent = self.get(id)?;
        ent.part("LENGTH_UNIT")?;
        if let Some(si) = ent.part("SI_UNIT") {
            let prefix = match si.first() {
                Some(Value::Enum(p)) => p.as_str(),
                _ => "",
            };
            return Some(match prefix {
                "EXA" => 1e18,
                "PETA" => 1e15,
                "TERA" => 1e12,
                "GIGA" => 1e9,
                "MEGA" => 1e6,
                "KILO" => 1e3,
                "HECTO" => 1e2,
                "DECA" => 1e1,
                "DECI" => 1e-1,
                "CENTI" => 1e-2,
                "MILLI" => 1e-3,
                "MICRO" => 1e-6,
                "NANO" => 1e-9,
                "PICO" => 1e-12,
                _ => 1.0,
            });
        }
        let conv = ent.part("CONVERSION_BASED_UNIT")?;
        let named = match conv.first() {
            Some(Value::Str(s)) => s.to_ascii_lowercase(),
            _ => String::new(),
        };
        let measured = conv.get(1).and_then(Value::as_ref).and_then(|m| {
            let p = self.get(m)?;
            let p = p
                .part("LENGTH_MEASURE_WITH_UNIT")
                .or_else(|| p.part("MEASURE_WITH_UNIT"))?;
            let v = p.first()?.as_num()?;
            let base = p.get(1)?.as_ref()?;
            (depth < 8)
                .then(|| self.unit_metres(base, depth + 1))
                .flatten()
                .map(|b| v * b)
        });
        measured.or(match named.as_str() {
            "inch" | "in" => Some(0.0254),
            "foot" | "ft" => Some(0.3048),
            "yard" => Some(0.9144),
            "mil" => Some(0.0000254),
            "mile" => Some(1609.344),
            _ => None,
        })
    }

    /// The length unit of the first context that says.
    fn length_unit(&self) -> Option<f64> {
        let mut ids: Vec<&u64> = self.e.keys().collect();
        ids.sort_unstable();
        for id in &ids {
            if let Some(units) = self.e[id].part("GLOBAL_UNIT_ASSIGNED_CONTEXT")
                && let Some(m) = units
                    .first()
                    .into_iter()
                    .flat_map(Value::refs)
                    .find_map(|u| self.unit_metres(u, 0))
            {
                return Some(m);
            }
        }
        ids.iter().find_map(|id| self.unit_metres(**id, 0))
    }
}

/// A cylindrical face in its part's own coordinates.
#[derive(Debug, Clone, Copy)]
struct CylFace {
    centre: P3,
    axis: P3,
    radius: f64,
    /// Its extent along the axis from `centre`.
    along: (f64, f64),
    /// How far round it goes, in degrees.
    span: f64,
    /// Which solid it's a face of: only faces of one solid join up.
    solid: usize,
}

/// One solid's share of a representation's faces.
struct Share {
    planes: usize,
    other: usize,
    triangles: std::ops::Range<usize>,
    points: Vec<P3>,
}

/// Faces of a solid (or a shell), sorted into kinds.
#[derive(Default)]
struct Faces {
    cylinders: Vec<CylFace>,
    planes: usize,
    other: usize,
    /// Triangles of faceted faces (POLY_LOOPs), for the mesh reader.
    triangles: Vec<[P3; 3]>,
}

impl Model<'_> {
    /// The faces of a shape representation item: a solid, a shell or a
    /// shell-based surface model.
    fn faces_of_item(&self, id: u64, out: &mut Faces) {
        let Some(ent) = self.get(id) else { return };
        let Some((kind, p)) = ent.simple() else {
            return;
        };
        match kind {
            "MANIFOLD_SOLID_BREP" | "FACETED_BREP" | "BREP_WITH_VOIDS" => {
                if let Some(shell) = p.get(1).and_then(Value::as_ref) {
                    self.faces_of_item(shell, out);
                }
            }
            "SHELL_BASED_SURFACE_MODEL" => {
                for s in p.get(1).into_iter().flat_map(Value::refs) {
                    self.faces_of_item(s, out);
                }
            }
            "CLOSED_SHELL" | "OPEN_SHELL" => {
                for f in p.get(1).into_iter().flat_map(Value::refs) {
                    self.face(f, out);
                }
            }
            _ => {}
        }
    }

    fn face(&self, id: u64, out: &mut Faces) {
        let Some((kind, p)) = self.get(id).and_then(Entity::simple) else {
            return;
        };
        if !matches!(kind, "ADVANCED_FACE" | "FACE_SURFACE" | "FACE") {
            return;
        }
        let bounds: Vec<u64> = p.get(1).into_iter().flat_map(Value::refs).collect();
        // A faceted face: its polygons, fanned into triangles.
        let polys: Vec<Vec<P3>> = bounds
            .iter()
            .filter_map(|b| {
                let lp = self.get(*b)?.simple()?.1.get(1)?.as_ref()?;
                let pts = self.of(lp, "POLY_LOOP")?.get(1)?.as_list();
                Some(pts.iter().filter_map(|v| self.point(v.as_ref()?)).collect())
            })
            .collect();
        if !polys.is_empty() {
            for poly in polys {
                for k in 1..poly.len().saturating_sub(1) {
                    out.triangles.push([poly[0], poly[k], poly[k + 1]]);
                }
            }
            return;
        }
        let Some(surface) = p.get(2).and_then(Value::as_ref) else {
            out.other += 1;
            return;
        };
        let surf = self.get(surface).and_then(Entity::simple);
        match surf {
            Some(("PLANE", _)) => out.planes += 1,
            Some(("CYLINDRICAL_SURFACE", sp)) => {
                let frame = sp
                    .get(1)
                    .and_then(Value::as_ref)
                    .and_then(|f| self.frame(f));
                let radius = sp.get(2).and_then(Value::as_num);
                match (frame, radius) {
                    (Some(f), Some(r)) if r > 0.0 => {
                        out.cylinders.push(self.cylinder(&bounds, f, r));
                    }
                    _ => out.other += 1,
                }
            }
            _ => out.other += 1,
        }
    }

    /// A cylindrical face's extent along its axis and how far round it
    /// goes, from its edges.
    fn cylinder(&self, bounds: &[u64], f: Frame, r: f64) -> CylFace {
        let (c, x, y, d) = (f.0[3], f.0[0], f.0[1], f.0[2]);
        let mut points = Vec::new();
        let mut arc_span: f64 = 0.0;
        let mut seen = HashSet::new();
        let mut stack: Vec<u64> = bounds.to_vec();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(ent) = self.get(id) else { continue };
            let Some((kind, p)) = ent.simple() else {
                continue;
            };
            match kind {
                "CARTESIAN_POINT" => {
                    if let Some(pt) = self.point(id) {
                        points.push(pt);
                    }
                    continue;
                }
                // Parameter-space curves and the surfaces they lie on
                // hold no 3D points of the edge.
                "PCURVE" | "DEFINITIONAL_REPRESENTATION" | "DIRECTION" | "VECTOR" => continue,
                k if k.ends_with("_SURFACE") || k == "PLANE" => continue,
                "EDGE_CURVE" => {
                    if let Some(span) = self.arc_span(p, d, r) {
                        arc_span = arc_span.max(span);
                    }
                }
                _ => {}
            }
            collect_refs(p, &mut stack);
        }
        let mut along = (f64::MAX, f64::MIN);
        let mut angles = Vec::new();
        for pt in &points {
            let v = sub(*pt, c);
            let t = dot(v, d);
            along = (along.0.min(t), along.1.max(t));
            let (u, w) = (dot(v, x), dot(v, y));
            if (u * u + w * w).sqrt() > 0.5 * r {
                angles.push(w.atan2(u).to_degrees());
            }
        }
        if along.0 > along.1 {
            along = (0.0, 0.0);
        }
        // Without circular edges, the points' spread round the axis.
        let span = if arc_span > 0.0 {
            arc_span
        } else {
            angles.sort_by(f64::total_cmp);
            let mut gap: f64 = 0.0;
            for k in 0..angles.len() {
                let next = if k + 1 < angles.len() {
                    angles[k + 1]
                } else {
                    angles[0] + 360.0
                };
                gap = gap.max(next - angles[k]);
            }
            if angles.is_empty() { 0.0 } else { 360.0 - gap }
        };
        CylFace {
            centre: c,
            axis: d,
            radius: r,
            along,
            span,
            solid: 0,
        }
    }

    /// How far round an edge goes, if it's an arc of a circle about the
    /// cylinder's axis with its radius.
    fn arc_span(&self, p: &[Value], axis: P3, r: f64) -> Option<f64> {
        let (v1, v2) = (p.get(1)?.as_ref()?, p.get(2)?.as_ref()?);
        let mut curve = p.get(3)?.as_ref()?;
        // SURFACE_CURVE and SEAM_CURVE wrap the 3D curve.
        for _ in 0..4 {
            match self.get(curve)?.simple()? {
                ("SURFACE_CURVE" | "SEAM_CURVE" | "BOUNDED_SURFACE_CURVE", q) => {
                    curve = q.get(1)?.as_ref()?;
                }
                ("TRIMMED_CURVE", q) => curve = q.get(1)?.as_ref()?,
                _ => break,
            }
        }
        let circle = self.of(curve, "CIRCLE")?;
        let f = self.frame(circle.get(1)?.as_ref()?)?;
        let cr = circle.get(2)?.as_num()?;
        if dot(f.0[2], axis).abs() < 0.9999 || (cr - r).abs() > 1e-6 * r.max(1e-9) {
            return None;
        }
        let vertex = |v: u64| self.point(self.of(v, "VERTEX_POINT")?.get(1)?.as_ref()?);
        let (a, b) = (vertex(v1)?, vertex(v2)?);
        if v1 == v2 || length(sub(a, b)) <= 1e-9 * r.max(1e-9) {
            return Some(360.0);
        }
        let angle = |q: P3| {
            let v = sub(q, f.0[3]);
            dot(v, f.0[1]).atan2(dot(v, f.0[0])).to_degrees()
        };
        let same_sense = !matches!(p.get(4), Some(Value::Enum(e)) if e == "F");
        let (from, to) = if same_sense {
            (angle(a), angle(b))
        } else {
            (angle(b), angle(a))
        };
        let span = (to - from).rem_euclid(360.0);
        Some(if span == 0.0 { 360.0 } else { span })
    }
}

impl Model<'_> {
    /// The faces of a solid or shell item.
    fn face_ids(&self, id: u64, out: &mut Vec<u64>, depth: usize) {
        let Some((kind, p)) = self.get(id).and_then(Entity::simple) else {
            return;
        };
        if depth > 8 {
            return;
        }
        match kind {
            "MANIFOLD_SOLID_BREP" | "FACETED_BREP" | "BREP_WITH_VOIDS" => {
                if let Some(shell) = p.get(1).and_then(Value::as_ref) {
                    self.face_ids(shell, out, depth + 1);
                }
            }
            "SHELL_BASED_SURFACE_MODEL" => {
                for sh in p.get(1).into_iter().flat_map(Value::refs) {
                    self.face_ids(sh, out, depth + 1);
                }
            }
            "CLOSED_SHELL" | "OPEN_SHELL" => out.extend(p.get(1).into_iter().flat_map(Value::refs)),
            _ => {}
        }
    }

    /// Points spread over a solid's surface, enough to see its shape: the
    /// corners, points along its circular edges, the control points of
    /// its curves and freeform surfaces, and points over spheres and
    /// surfaces of revolution.
    fn solid_points(&self, item: u64) -> Vec<P3> {
        let mut faces = Vec::new();
        self.face_ids(item, &mut faces, 0);
        let mut out = Vec::new();
        for face in faces {
            let Some((kind, p)) = self.get(face).and_then(Entity::simple) else {
                continue;
            };
            if !matches!(kind, "ADVANCED_FACE" | "FACE_SURFACE" | "FACE") {
                continue;
            }
            let bounds: Vec<u64> = p.get(1).into_iter().flat_map(Value::refs).collect();
            let rim = self.points_under(&bounds, true);
            // Points spread evenly over a curved face stand for it alone:
            // its edges' points would crowd one side (a sphere's seam).
            let inside = p
                .get(2)
                .and_then(Value::as_ref)
                .map(|surface| self.surface_points(surface, &rim))
                .unwrap_or_default();
            if inside.is_empty() {
                out.extend(rim);
            } else {
                out.extend(thinned(inside));
            }
        }
        out
    }

    /// The 3D points under these entities (placements, directions and
    /// parameter-space curves left out), with circular edges sampled.
    fn points_under(&self, start: &[u64], edges: bool) -> Vec<P3> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack: Vec<u64> = start.to_vec();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(ent) = self.get(id) else { continue };
            if let Some(curve) = self.bspline(id, false) {
                out.extend(curve.curve_points(24));
                continue;
            }
            let Some((kind, p)) = ent.simple() else {
                for (_, q) in &ent.parts {
                    collect_refs(q, &mut stack);
                }
                continue;
            };
            match kind {
                "CARTESIAN_POINT" => {
                    if let Some(pt) = self.point(id) {
                        out.push(pt);
                    }
                    continue;
                }
                "AXIS2_PLACEMENT_3D"
                | "AXIS1_PLACEMENT"
                | "AXIS2_PLACEMENT_2D"
                | "DIRECTION"
                | "VECTOR"
                | "PCURVE"
                | "DEFINITIONAL_REPRESENTATION" => continue,
                k if edges && (k.ends_with("_SURFACE") || k == "PLANE") => continue,
                "EDGE_CURVE" => out.extend(self.edge_points(p)),
                _ => {}
            }
            collect_refs(p, &mut stack);
        }
        out
    }

    /// Points along an edge: an arc of a circle sampled, a straight edge
    /// cut into tenths.
    fn edge_points(&self, p: &[Value]) -> Vec<P3> {
        let Some(v1) = p.get(1).and_then(Value::as_ref) else {
            return Vec::new();
        };
        let Some(v2) = p.get(2).and_then(Value::as_ref) else {
            return Vec::new();
        };
        let Some(mut curve) = p.get(3).and_then(Value::as_ref) else {
            return Vec::new();
        };
        for _ in 0..4 {
            match self.get(curve).and_then(Entity::simple) {
                Some((
                    "SURFACE_CURVE" | "SEAM_CURVE" | "BOUNDED_SURFACE_CURVE" | "TRIMMED_CURVE",
                    q,
                )) => match q.get(1).and_then(Value::as_ref) {
                    Some(c) => curve = c,
                    None => return Vec::new(),
                },
                _ => break,
            }
        }
        let vertex = |v: u64| self.point(self.of(v, "VERTEX_POINT")?.get(1)?.as_ref()?);
        if self.of(curve, "LINE").is_some() {
            return match (vertex(v1), vertex(v2)) {
                (Some(a), Some(b)) => (1..10)
                    .map(|k| add(a, scale(sub(b, a), k as f64 / 10.0)))
                    .collect(),
                _ => Vec::new(),
            };
        }
        let Some(circle) = self.of(curve, "CIRCLE") else {
            return Vec::new();
        };
        let (Some(f), Some(r)) = (
            circle
                .get(1)
                .and_then(Value::as_ref)
                .and_then(|f| self.frame(f)),
            circle.get(2).and_then(Value::as_num),
        ) else {
            return Vec::new();
        };
        let angle = |q: P3| {
            let v = sub(q, f.0[3]);
            dot(v, f.0[1]).atan2(dot(v, f.0[0]))
        };
        let (from, span) = match (vertex(v1), vertex(v2)) {
            (Some(a), Some(b)) if v1 != v2 && length(sub(a, b)) > 1e-9 * r.max(1e-9) => {
                let same_sense = !matches!(p.get(4), Some(Value::Enum(e)) if e == "F");
                let (s, e) = if same_sense {
                    (angle(a), angle(b))
                } else {
                    (angle(b), angle(a))
                };
                (s, (e - s).rem_euclid(2.0 * std::f64::consts::PI))
            }
            _ => (0.0, 2.0 * std::f64::consts::PI),
        };
        (0..=24)
            .map(|k| {
                let t = from + span * k as f64 / 24.0;
                f.point([r * t.cos(), r * t.sin(), 0.0])
            })
            .collect()
    }

    /// Points over a face's surface, from its rim: control points of
    /// freeform surfaces, a cap of a sphere, or a surface of revolution's
    /// curve turned round its axis.
    fn surface_points(&self, surface: u64, rim: &[P3]) -> Vec<P3> {
        let Some(ent) = self.get(surface) else {
            return Vec::new();
        };
        let name = ent
            .parts
            .iter()
            .map(|p| p.0.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(b) = self.bspline(surface, true) {
            return b.surface_points(16);
        }
        if name.contains("B_SPLINE_SURFACE") || name.contains("BEZIER_SURFACE") {
            return self.points_under(&[surface], false);
        }
        if let Some(sp) = ent.part("SPHERICAL_SURFACE")
            && let (Some(f), Some(r)) = (
                sp.get(1)
                    .and_then(Value::as_ref)
                    .and_then(|f| self.frame(f)),
                sp.get(2).and_then(Value::as_num),
            )
            && !rim.is_empty()
        {
            // The cap the rim encloses: its middle direction and width.
            let c = f.0[3];
            // The narrowest cone round the rim's directions (its seam and
            // pole count too): step towards the farthest each time.
            let dirs: Vec<P3> = rim.iter().map(|q| unit(sub(*q, c))).collect();
            let mut mid = unit(dirs.iter().fold([0.0; 3], |acc, d| add(acc, *d)));
            for k in 1..400 {
                let far = dirs
                    .iter()
                    .min_by(|a, b| dot(**a, mid).total_cmp(&dot(**b, mid)))
                    .copied()
                    .unwrap_or(mid);
                mid = unit(add(mid, scale(sub(far, mid), 1.0 / (k as f64 + 1.0))));
            }
            let widest = rim
                .iter()
                .map(|q| dot(unit(sub(*q, c)), mid).clamp(-1.0, 1.0).acos())
                .fold(0.0, f64::max);
            let e1 = square_to(mid);
            let e2 = cross(mid, e1);
            let mut out = vec![add(c, scale(mid, r))];
            for ring in 1..=4 {
                let th = widest * ring as f64 / 4.0;
                for k in 0..16 {
                    let ph = 2.0 * std::f64::consts::PI * k as f64 / 16.0;
                    let d = add(
                        scale(mid, th.cos()),
                        scale(add(scale(e1, ph.cos()), scale(e2, ph.sin())), th.sin()),
                    );
                    out.push(add(c, scale(d, r)));
                }
            }
            return out;
        }
        if let Some(sr) = ent.part("SURFACE_OF_REVOLUTION")
            && let (Some(curve), Some(axis)) = (
                sr.get(1).and_then(Value::as_ref),
                sr.get(2).and_then(Value::as_ref),
            )
            && let Some(ax) = self.of(axis, "AXIS1_PLACEMENT")
            && let Some(o) = ax
                .get(1)
                .and_then(Value::as_ref)
                .and_then(|q| self.point(q))
        {
            let d = ax
                .get(2)
                .and_then(Value::as_ref)
                .and_then(|q| self.direction(q))
                .unwrap_or([0.0, 0.0, 1.0]);
            let profile = self.points_under(&[curve], false);
            let mut out = Vec::new();
            for k in 0..24 {
                let t = 2.0 * std::f64::consts::PI * k as f64 / 24.0;
                let (cs, sn) = (t.cos(), t.sin());
                for q in &profile {
                    // Rodrigues' rotation about the axis through o.
                    let v = sub(*q, o);
                    let along = scale(d, dot(v, d));
                    let rot = add(add(scale(sub(v, along), cs), scale(cross(d, v), sn)), along);
                    out.push(add(o, rot));
                }
            }
            return out;
        }
        Vec::new()
    }
}

/// A B-spline curve or surface: degrees, control points (rows of a
/// surface's net), weights and full knot vectors.
struct BSpline {
    degree: [usize; 2],
    control: Vec<Vec<P3>>,
    weights: Vec<Vec<f64>>,
    knots: [Vec<f64>; 2],
}

impl BSpline {
    /// Points along a curve, `n` intervals over its parameter range.
    fn curve_points(&self, n: usize) -> Vec<P3> {
        let (p, k) = (self.degree[1], &self.knots[1]);
        let ctrl = &self.control[0];
        let w = &self.weights[0];
        let (a, b) = (k[p], k[ctrl.len()]);
        (0..=n)
            .map(|i| {
                let t = a + (b - a) * i as f64 / n as f64;
                crate::dxf::de_boor(t, p, k, ctrl, &|j| w[j])
            })
            .collect()
    }

    /// Points over a surface, an `n` x `n` grid of its parameters.
    fn surface_points(&self, n: usize) -> Vec<P3> {
        let [pu, pv] = self.degree;
        let [ku, kv] = &self.knots;
        let (nu, nv) = (self.control.len(), self.control[0].len());
        let range = |k: &[f64], p: usize, n: usize| (k[p], k[n]);
        let (u0, u1) = range(ku, pu, nu);
        let (v0, v1) = range(kv, pv, nv);
        let mut out = Vec::new();
        for i in 0..=n {
            let v = v0 + (v1 - v0) * i as f64 / n as f64;
            // Each row along v, kept homogeneous (x w, y w, z w as a point,
            // w apart), then the column along u.
            let mut col = Vec::with_capacity(nu);
            let mut wcol = Vec::with_capacity(nu);
            for r in 0..nu {
                let row = &self.control[r];
                let w = &self.weights[r];
                let lifted: Vec<P3> = row.iter().zip(w).map(|(q, wi)| scale(*q, *wi)).collect();
                let num = crate::dxf::de_boor(v, pv, kv, &lifted, &|_| 1.0);
                let ws: Vec<P3> = w.iter().map(|wi| [*wi, 0.0, 0.0]).collect();
                let den = crate::dxf::de_boor(v, pv, kv, &ws, &|_| 1.0)[0];
                col.push(num);
                wcol.push(den);
            }
            for j in 0..=n {
                let u = u0 + (u1 - u0) * j as f64 / n as f64;
                let num = crate::dxf::de_boor(u, pu, ku, &col, &|_| 1.0);
                let ws: Vec<P3> = wcol.iter().map(|wi| [*wi, 0.0, 0.0]).collect();
                let den = crate::dxf::de_boor(u, pu, ku, &ws, &|_| 1.0)[0];
                if den != 0.0 {
                    out.push(scale(num, 1.0 / den));
                }
            }
        }
        out
    }
}

impl Model<'_> {
    /// A B-spline curve (or surface, if `surface`), simple or rational.
    fn bspline(&self, id: u64, surface: bool) -> Option<BSpline> {
        let ent = self.get(id)?;
        let (base, knots_part, rational) = if surface {
            (
                "B_SPLINE_SURFACE",
                "B_SPLINE_SURFACE_WITH_KNOTS",
                "RATIONAL_B_SPLINE_SURFACE",
            )
        } else {
            (
                "B_SPLINE_CURVE",
                "B_SPLINE_CURVE_WITH_KNOTS",
                "RATIONAL_B_SPLINE_CURVE",
            )
        };
        // A simple entity has a name first and its knots after the shape's
        // own fields; a complex one splits them into parts.
        let (shape, knots, weights): (Vec<Value>, Vec<Value>, Option<&Value>) =
            if let Some((kind, p)) = ent.simple() {
                if kind != knots_part {
                    return None;
                }
                let own = if surface { 8 } else { 6 };
                (p.get(1..own)?.to_vec(), p.get(own..)?.to_vec(), None)
            } else {
                let shape = ent.part(base)?.to_vec();
                let knots = ent.part(knots_part)?.to_vec();
                let weights = ent.part(rational).and_then(|w| w.first());
                (shape, knots, weights)
            };
        let int = |v: &Value| v.as_num().map(|x| x as usize);
        let full = |mults: &Value, values: &Value| -> Vec<f64> {
            mults
                .as_list()
                .iter()
                .zip(values.as_list())
                .flat_map(|(m, k)| {
                    let m = m.as_num().unwrap_or(1.0).max(0.0) as usize;
                    std::iter::repeat_n(k.as_num().unwrap_or(0.0), m)
                })
                .collect()
        };
        let points = |v: &Value| -> Option<Vec<P3>> {
            v.as_list()
                .iter()
                .map(|q| self.point(q.as_ref()?))
                .collect()
        };
        if surface {
            let (du, dv) = (int(shape.first()?)?, int(shape.get(1)?)?);
            let control: Vec<Vec<P3>> = shape
                .get(2)?
                .as_list()
                .iter()
                .map(points)
                .collect::<Option<_>>()?;
            let ku = full(knots.first()?, knots.get(2)?);
            let kv = full(knots.get(1)?, knots.get(3)?);
            let (nu, nv) = (control.len(), control.first()?.len());
            if nu == 0 || nv == 0 || ku.len() != nu + du + 1 || kv.len() != nv + dv + 1 {
                return None;
            }
            let weights: Vec<Vec<f64>> = match weights {
                Some(w) => w
                    .as_list()
                    .iter()
                    .map(|row| {
                        row.as_list()
                            .iter()
                            .map(|x| x.as_num().unwrap_or(1.0))
                            .collect()
                    })
                    .collect(),
                None => vec![vec![1.0; nv]; nu],
            };
            if weights.len() != nu || weights.iter().any(|r| r.len() != nv) {
                return None;
            }
            Some(BSpline {
                degree: [du, dv],
                control,
                weights,
                knots: [ku, kv],
            })
        } else {
            let d = int(shape.first()?)?;
            let control = points(shape.get(1)?)?;
            let k = full(knots.first()?, knots.get(1)?);
            let n = control.len();
            if n == 0 || k.len() != n + d + 1 {
                return None;
            }
            let w: Vec<f64> = match weights {
                Some(w) => w
                    .as_list()
                    .iter()
                    .map(|x| x.as_num().unwrap_or(1.0))
                    .collect(),
                None => vec![1.0; n],
            };
            if w.len() != n {
                return None;
            }
            Some(BSpline {
                degree: [0, d],
                control: vec![control],
                weights: vec![w],
                knots: [Vec::new(), k],
            })
        }
    }
}

/// A face's points spaced at least a fortieth of its size apart, so a
/// narrow face (a sheet's edge) adds a few points, not as many as a broad
/// one.
fn thinned(points: Vec<P3>) -> Vec<P3> {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let gap = length(sub(hi, lo)) / 40.0;
    let mut kept: Vec<P3> = Vec::new();
    for p in points {
        if kept.iter().all(|q| length(sub(p, *q)) >= gap) {
            kept.push(p);
        }
    }
    kept
}

fn collect_refs(values: &[Value], stack: &mut Vec<u64>) {
    for v in values {
        match v {
            Value::Ref(r) => stack.push(*r),
            Value::List(l) | Value::Typed(_, l) => collect_refs(l, stack),
            _ => {}
        }
    }
}

/// What a STEP file holds: its tubes and thin sheets (in the file's
/// units) and the units.
#[derive(Debug, Clone, PartialEq)]
pub struct Solids {
    pub parts: Parts,
    /// Thin solid sheets (plates, corner reflectors, dishes) fitted as
    /// reflectors; their faces aren't counted in `parts`.
    pub sheets: Vec<crate::sheetfit::SheetFit>,
    /// Metres per unit, as the file says.
    pub metres_per_unit: Option<f64>,
}

/// Reads a STEP file's solids and finds their straight round tubes:
/// cylindrical faces about the same axis with the same radius are one
/// tube when together they go all the way round. Assemblies are followed,
/// so parts used several times give a tube each time.
pub fn read_step(text: &str) -> Result<Solids, String> {
    let entities = parse(text)?;
    let m = Model { e: &entities };
    let unit = m.length_unit();

    // Representations, joined by relationships into frames: a part's
    // geometry and the shape it belongs to share a frame; an assembly's
    // part sits in its parent's frame through a transformation.
    let mut ids: Vec<u64> = entities.keys().copied().collect();
    ids.sort_unstable();
    let is_rep = |id: u64| {
        entities[&id].parts.iter().any(|(k, p)| {
            k.ends_with("REPRESENTATION")
                && !k.starts_with("DEFINITIONAL")
                && p.len() >= 3
                && matches!(p[1], Value::List(_))
        })
    };
    let reps: Vec<u64> = ids.iter().copied().filter(|&i| is_rep(i)).collect();
    let index: HashMap<u64, usize> = reps.iter().enumerate().map(|(k, r)| (*r, k)).collect();
    let mut parent: Vec<usize> = (0..reps.len()).collect();

    // Which representation belongs to which product definition, to tell
    // a relationship's child from its parent.
    let mut reps_of_definition: HashMap<u64, Vec<u64>> = HashMap::new();
    for &id in &ids {
        if let Some(p) = entities[&id].part("SHAPE_DEFINITION_REPRESENTATION")
            && let (Some(pds), Some(rep)) = (
                p.first().and_then(Value::as_ref),
                p.get(1).and_then(Value::as_ref),
            )
            && let Some(pd) = m
                .of(pds, "PRODUCT_DEFINITION_SHAPE")
                .and_then(|q| q.get(2)?.as_ref())
        {
            reps_of_definition.entry(pd).or_default().push(rep);
        }
    }
    let mut child_of_relationship: HashMap<u64, u64> = HashMap::new();
    for &id in &ids {
        if let Some(p) = entities[&id].part("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION")
            && let (Some(rr), Some(pds)) = (
                p.first().and_then(Value::as_ref),
                p.get(1).and_then(Value::as_ref),
            )
            && let Some(nauo) = m
                .of(pds, "PRODUCT_DEFINITION_SHAPE")
                .and_then(|q| q.get(2)?.as_ref())
            && let Some(usage) = m.get(nauo).and_then(|e| {
                e.part("NEXT_ASSEMBLY_USAGE_OCCURRENCE")
                    .or_else(|| e.part("PRODUCT_DEFINITION_USAGE"))
                    .or_else(|| e.part("ASSEMBLY_COMPONENT_USAGE"))
            })
            && let Some(related) = usage.get(4).and_then(Value::as_ref)
        {
            child_of_relationship.insert(rr, related);
        }
    }

    // (child rep, parent rep, child to parent).
    let mut links: Vec<(usize, usize, Frame)> = Vec::new();
    for &id in &ids {
        let ent = &entities[&id];
        let Some(rr) = ent
            .part("REPRESENTATION_RELATIONSHIP")
            .or_else(|| ent.part("SHAPE_REPRESENTATION_RELATIONSHIP"))
        else {
            continue;
        };
        let (Some(r1), Some(r2)) = (
            rr.get(2)
                .and_then(Value::as_ref)
                .and_then(|r| index.get(&r).copied()),
            rr.get(3)
                .and_then(Value::as_ref)
                .and_then(|r| index.get(&r).copied()),
        ) else {
            continue;
        };
        let transform = ent
            .part("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
            .and_then(|p| p.first()?.as_ref());
        let Some(t) = transform else {
            union(&mut parent, r1, r2);
            continue;
        };
        let Some(idt) = m.of(t, "ITEM_DEFINED_TRANSFORMATION") else {
            continue;
        };
        let (Some(i1), Some(i2)) = (
            idt.get(2)
                .and_then(Value::as_ref)
                .and_then(|i| m.placement_or_operator(i)),
            idt.get(3)
                .and_then(Value::as_ref)
                .and_then(|i| m.placement_or_operator(i)),
        ) else {
            continue;
        };
        // The child is rep_1 unless the assembly structure says rep_2.
        let child_is_2 = child_of_relationship.get(&id).is_some_and(|pd| {
            let mine = reps_of_definition.get(pd);
            let has = |r: usize| mine.is_some_and(|v| v.contains(&reps[r]));
            has(r2) && !has(r1)
        });
        let (child, parent_rep, item_child, item_parent) = if child_is_2 {
            (r2, r1, i2, i1)
        } else {
            (r1, r2, i1, i2)
        };
        links.push((child, parent_rep, item_parent.then(&item_child.inverse())));
    }
    // Mapped items: a representation placed inside another.
    for (k, &rep) in reps.iter().enumerate() {
        for item in rep_items(&entities[&rep]) {
            let Some(mi) = m.of(item, "MAPPED_ITEM") else {
                continue;
            };
            let (Some(map), Some(target)) = (
                mi.get(1).and_then(Value::as_ref),
                mi.get(2).and_then(Value::as_ref),
            ) else {
                continue;
            };
            let Some(rm) = m.of(map, "REPRESENTATION_MAP") else {
                continue;
            };
            let (Some(origin), Some(mapped)) = (
                rm.first()
                    .and_then(Value::as_ref)
                    .and_then(|o| m.placement_or_operator(o)),
                rm.get(1)
                    .and_then(Value::as_ref)
                    .and_then(|r| index.get(&r).copied()),
            ) else {
                continue;
            };
            let target = m.placement_or_operator(target).unwrap_or(Frame::IDENTITY);
            links.push((mapped, k, target.then(&origin.inverse())));
        }
    }
    let links: Vec<(usize, usize, Frame)> = links
        .into_iter()
        .map(|(c, p, f)| (find(&mut parent, c), find(&mut parent, p), f))
        .filter(|(c, p, _)| c != p)
        .collect();

    // Every place each frame ends up in the top frame (assemblies may use
    // a part many times).
    const MAX_PLACES: usize = 20_000;
    fn places(group: usize, links: &[(usize, usize, Frame)], depth: usize, out: &mut Vec<Frame>) {
        if out.len() >= MAX_PLACES {
            return;
        }
        let ups: Vec<&(usize, usize, Frame)> = links.iter().filter(|l| l.0 == group).collect();
        if ups.is_empty() || depth > 32 {
            out.push(Frame::IDENTITY);
            return;
        }
        for (_, up, f) in ups {
            let mut above = Vec::new();
            places(*up, links, depth + 1, &mut above);
            for a in above {
                out.push(a.then(f));
            }
        }
    }

    let mut out = Parts::default();
    let mut cylinders: Vec<CylFace> = Vec::new();
    let mut triangles: Vec<[P3; 3]> = Vec::new();
    let mut done = HashSet::new();
    let mut next_solid = 0;
    let mut sheets: Vec<(Vec<P3>, crate::sheetfit::SheetFit)> = Vec::new();
    for (k, &rep) in reps.iter().enumerate() {
        let mut faces = Faces::default();
        let mut solids = 0;
        // Each solid's share of the faces, and points over its surface.
        let mut shares: Vec<Share> = Vec::new();
        for item in rep_items(&entities[&rep]) {
            if done.insert(item) {
                let before = (
                    faces.cylinders.len(),
                    faces.planes,
                    faces.other,
                    faces.triangles.len(),
                );
                m.faces_of_item(item, &mut faces);
                for c in &mut faces.cylinders[before.0..] {
                    c.solid = solids;
                }
                shares.push(Share {
                    planes: faces.planes - before.1,
                    other: faces.other - before.2,
                    triangles: before.3..faces.triangles.len(),
                    points: m.solid_points(item),
                });
                solids += 1;
            }
        }
        if faces.cylinders.is_empty()
            && faces.planes == 0
            && faces.other == 0
            && faces.triangles.is_empty()
        {
            continue;
        }
        // A solid holding a whole round tube is tubes (a T of two tubes
        // can lie flat like a sheet).
        let has_tube: Vec<bool> = (0..shares.len())
            .map(|k| {
                let mine: Vec<CylFace> = faces
                    .cylinders
                    .iter()
                    .filter(|c| c.solid == k)
                    .copied()
                    .collect();
                !join_cylinders(&mine).0.is_empty()
            })
            .collect();
        let group = find(&mut parent, k);
        let mut at = Vec::new();
        places(group, &links, 0, &mut at);
        for f in &at {
            // Thin sheets are fitted as reflectors; the rest is sorted
            // into tubes and what's left out.
            let mut sheet = vec![false; shares.len()];
            for (k, share) in shares.iter().enumerate() {
                let points: Vec<P3> = share.points.iter().map(|p| f.point(*p)).collect();
                let fit = (!has_tube[k])
                    .then(|| crate::sheetfit::fit_sheet(&points, None))
                    .flatten();
                if let Some(fit) = fit {
                    sheet[k] = true;
                    sheets.push((points, fit));
                } else {
                    out.flat += share.planes;
                    out.other += share.other;
                    for t in &faces.triangles[share.triangles.clone()] {
                        triangles.push(t.map(|p| f.point(p)));
                    }
                }
            }
            for c in &faces.cylinders {
                if sheet.get(c.solid) != Some(&true) {
                    cylinders.push(CylFace {
                        centre: f.point(c.centre),
                        axis: f.vector(c.axis),
                        solid: next_solid + c.solid,
                        ..*c
                    });
                }
            }
            next_solid += solids;
        }
    }

    let (tubes, other) = join_cylinders(&cylinders);
    out.other += other;
    if !triangles.is_empty() {
        let mesh = crate::stl::tubes(&triangles);
        out.flat += mesh.flat;
        out.other += mesh.other;
        out.tubes.extend(mesh.tubes);
    }
    out.tubes = without_inner_walls(tubes.into_iter().chain(out.tubes).collect());
    // Flat sheets face the rest of the antenna (the tubes' middle).
    let towards = (!out.tubes.is_empty()).then(|| {
        let n = out.tubes.len() as f64;
        out.tubes
            .iter()
            .fold([0.0; 3], |acc, t| add(acc, scale(add(t.a, t.b), 0.5 / n)))
    });
    let sheets = sheets
        .into_iter()
        .map(|(points, fit)| match fit.shape {
            crate::sheetfit::Shape::Flat if towards.is_some() => {
                crate::sheetfit::fit_sheet(&points, towards).unwrap_or(fit)
            }
            _ => fit,
        })
        .collect();
    Ok(Solids {
        parts: out,
        sheets,
        metres_per_unit: unit,
    })
}

/// A representation's items.
fn rep_items(e: &Entity) -> Vec<u64> {
    e.parts
        .iter()
        .filter(|(k, p)| k.ends_with("REPRESENTATION") && p.len() >= 3)
        .flat_map(|(_, p)| p[1].refs().collect::<Vec<_>>())
        .collect()
}

/// Cylindrical faces on one axis line with one radius, touching or
/// overlapping along it, are one tube when they go all the way round.
/// Returns the tubes and how many groups weren't tubes.
fn join_cylinders(faces: &[CylFace]) -> (Vec<Tube>, usize) {
    let mut order: Vec<usize> = (0..faces.len()).collect();
    order.sort_by(|a, b| faces[*a].radius.total_cmp(&faces[*b].radius));
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let ends = |f: &CylFace| {
        (
            add(f.centre, scale(f.axis, f.along.0)),
            add(f.centre, scale(f.axis, f.along.1)),
        )
    };
    for (k, &i) in order.iter().enumerate() {
        let a = &faces[i];
        let tol = 1e-6 * a.radius.max(1e-9) + 1e-9;
        for &j in &order[k + 1..] {
            let b = &faces[j];
            if b.radius - a.radius > tol {
                break;
            }
            if a.solid != b.solid || dot(a.axis, b.axis).abs() < 1.0 - 1e-9 {
                continue;
            }
            // b's axis point on a's axis line.
            let v = sub(b.centre, a.centre);
            let off = sub(v, scale(a.axis, dot(v, a.axis)));
            if length(off) > 10.0 * tol {
                continue;
            }
            let (b0, b1) = ends(b);
            let (t0, t1) = (
                dot(sub(b0, a.centre), a.axis),
                dot(sub(b1, a.centre), a.axis),
            );
            let (lo, hi) = (t0.min(t1), t0.max(t1));
            if lo <= a.along.1 + 10.0 * tol && hi >= a.along.0 - 10.0 * tol {
                union(&mut parent, i, j);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..faces.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort_unstable();
    let mut tubes = Vec::new();
    let mut other = 0;
    for r in roots {
        let members = &groups[&r];
        let first = &faces[members[0]];
        let span: f64 = members.iter().map(|&i| faces[i].span).sum();
        let mut along = (f64::MAX, f64::MIN);
        for &i in members {
            let (p, q) = ends(&faces[i]);
            for e in [p, q] {
                let t = dot(sub(e, first.centre), first.axis);
                along = (along.0.min(t), along.1.max(t));
            }
        }
        let radius = members.iter().map(|&i| faces[i].radius).fold(0.0, f64::max);
        if span < 350.0 || along.1 - along.0 < 2.0 * radius {
            other += 1;
            continue;
        }
        tubes.push(Tube {
            a: add(first.centre, scale(first.axis, along.0)),
            b: add(first.centre, scale(first.axis, along.1)),
            radius,
        });
    }
    (tubes, other)
}

/// Leaves out tubes that are another tube's inner wall: on the same axis,
/// thinner, and within it for most of their length.
fn without_inner_walls(tubes: Vec<Tube>) -> Vec<Tube> {
    let inside = |t: &Tube, o: &Tube| {
        let d = unit(sub(o.b, o.a));
        let len = length(sub(t.b, t.a));
        let off = |p: P3| {
            let v = sub(p, o.a);
            length(sub(v, scale(d, dot(v, d))))
        };
        if off(t.a) > o.radius || off(t.b) > o.radius || dot(unit(sub(t.b, t.a)), d).abs() < 0.999 {
            return false;
        }
        let (ta, tb) = (dot(sub(t.a, o.a), d), dot(sub(t.b, o.a), d));
        let ol = length(sub(o.b, o.a));
        let overlap = ta.max(tb).min(ol) - ta.min(tb).max(0.0);
        overlap > 0.5 * len
    };
    tubes
        .iter()
        .enumerate()
        .filter(|(k, t)| {
            !tubes.iter().enumerate().any(|(j, o)| {
                j != *k && (o.radius > t.radius || (o.radius == t.radius && j < *k)) && inside(t, o)
            })
        })
        .map(|(_, t)| *t)
        .collect()
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut i = i;
    while parent[i] != r {
        let next = parent[i];
        parent[i] = r;
        i = next;
    }
    r
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[a.max(b)] = a.min(b);
    }
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: P3, k: f64) -> P3 {
    a.map(|v| v * k)
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: P3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: P3) -> P3 {
    let l = length(a).max(1e-300);
    a.map(|v| v / l)
}
fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// A unit direction square to d.
fn square_to(d: P3) -> P3 {
    let helper = if d[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    unit(cross(helper, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P3, b: P3, tol: f64) -> bool {
        length(sub(a, b)) < tol
    }

    fn rod(name: &str, a: P3, b: P3, radius: f64, inner: Option<f64>) -> Rod {
        Rod {
            name: name.into(),
            a,
            b,
            radius,
            inner,
        }
    }

    #[test]
    fn written_rods_read_back() {
        let rods = [
            rod(
                "driven",
                [0.0, -5.1, 10.0],
                [0.0, 5.1, 10.0],
                0.006,
                Some(0.0045),
            ),
            rod("mast", [0.1, 0.0, 0.0], [0.1, 0.0, 10.0], 0.02, None),
            rod(
                "tilted élément",
                [1.0, 2.0, 3.0],
                [1.3, 2.4, 3.5],
                0.001,
                None,
            ),
            rod("nothing", [1.0; 3], [1.0; 3], 0.001, None),
        ];
        // As one part, and as an assembly of two.
        let one = [Body {
            name: "all".into(),
            rods: rods.to_vec(),
            sheets: Vec::new(),
        }];
        let two = [
            Body {
                name: "element".into(),
                rods: rods[..2].to_vec(),
                sheets: Vec::new(),
            },
            Body {
                name: "other".into(),
                rods: rods[2..].to_vec(),
                sheets: Vec::new(),
            },
        ];
        for (bodies, assembly) in [(&one[..], false), (&two[..], true)] {
            let text = write(bodies, "test 'yagi'");
            assert!(text.starts_with("ISO-10303-21;"));
            assert!(text.contains("'test ''yagi'''"));
            assert!(text.contains("\\X2\\00E9\\X0\\"));
            assert_eq!(text.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"), assembly);
            let read = read_step(&text).unwrap();
            assert_eq!(read.metres_per_unit, Some(0.001));
            let tubes = &read.parts.tubes;
            assert_eq!(tubes.len(), 3, "{tubes:?}");
            // The hollow tube's inner wall is left out; 2 + 2 + 2 caps.
            assert_eq!(read.parts.flat, 6);
            assert_eq!(read.parts.other, 0);
            for (t, r) in tubes.iter().zip(&rods) {
                let (a, b) = (r.a.map(|v| v * 1000.0), r.b.map(|v| v * 1000.0));
                assert!(
                    (close(t.a, a, 1e-5) && close(t.b, b, 1e-5))
                        || (close(t.a, b, 1e-5) && close(t.b, a, 1e-5)),
                    "{t:?} vs {r:?}"
                );
                assert!((t.radius - r.radius * 1000.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn reals_and_strings() {
        assert_eq!(real(1.0, 6), "1.");
        assert_eq!(real(-0.0000001, 6), "0.");
        assert_eq!(real(2.5, 6), "2.5");
        assert_eq!(real(-12.3456789, 6), "-12.345679");
        assert_eq!(text("a\\b"), "'a\\\\b'");
        let ts = timestamp();
        assert_eq!(ts.len(), 19, "{ts}");
    }

    /// A cylinder split into two half faces (as some CAD programs write
    /// it), a quarter-round fillet, inch units, and a part used twice in
    /// an assembly.
    #[test]
    fn halves_fillets_units_and_assemblies() {
        let text = "ISO-10303-21;
HEADER;
FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
ENDSEC;
DATA;
/* units: inches */
#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));
#2=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);
#3=(CONVERSION_BASED_UNIT('INCH',#2) LENGTH_UNIT() NAMED_UNIT(#4));
#4=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
#5=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#3)) REPRESENTATION_CONTEXT('',''));
#10=CARTESIAN_POINT('',(0.,0.,0.));
#11=DIRECTION('',(0.,0.,1.));
#12=DIRECTION('',(1.,0.,0.));
#13=AXIS2_PLACEMENT_3D('',#10,#11,#12);
#14=CYLINDRICAL_SURFACE('',#13,0.5);
#20=CARTESIAN_POINT('',(0.5,0.,0.));
#21=CARTESIAN_POINT('',(-0.5,0.,0.));
#22=CARTESIAN_POINT('',(0.5,0.,8.));
#23=CARTESIAN_POINT('',(-0.5,0.,8.));
#24=VERTEX_POINT('',#20);
#25=VERTEX_POINT('',#21);
#26=VERTEX_POINT('',#22);
#27=VERTEX_POINT('',#23);
#30=CIRCLE('',#13,0.5);
#31=CARTESIAN_POINT('',(0.,0.,8.));
#32=AXIS2_PLACEMENT_3D('',#31,#11,#12);
#33=CIRCLE('',#32,0.5);
#40=EDGE_CURVE('',#24,#25,#30,.T.);
#41=EDGE_CURVE('',#25,#24,#30,.T.);
#42=EDGE_CURVE('',#26,#27,#33,.T.);
#43=EDGE_CURVE('',#27,#26,#33,.T.);
#50=ORIENTED_EDGE('',*,*,#40,.T.);
#51=ORIENTED_EDGE('',*,*,#42,.F.);
#52=EDGE_LOOP('',(#50,#51));
#53=FACE_OUTER_BOUND('',#52,.T.);
#54=ADVANCED_FACE('',(#53),#14,.T.);
#55=ORIENTED_EDGE('',*,*,#41,.T.);
#56=ORIENTED_EDGE('',*,*,#43,.F.);
#57=EDGE_LOOP('',(#55,#56));
#58=FACE_OUTER_BOUND('',#57,.T.);
#59=ADVANCED_FACE('',(#58),#14,.T.);
/* a quarter-round fillet along y, radius 0.2, 4 long */
#60=CARTESIAN_POINT('',(5.,0.,0.));
#61=DIRECTION('',(0.,1.,0.));
#62=AXIS2_PLACEMENT_3D('',#60,#61,#12);
#63=CYLINDRICAL_SURFACE('',#62,0.2);
#64=CARTESIAN_POINT('',(5.2,0.,0.));
#65=CARTESIAN_POINT('',(5.,0.,-0.2));
#66=VERTEX_POINT('',#64);
#67=VERTEX_POINT('',#65);
#68=CIRCLE('',#62,0.2);
#69=EDGE_CURVE('',#66,#67,#68,.T.);
#70=ORIENTED_EDGE('',*,*,#69,.T.);
#71=EDGE_LOOP('',(#70));
#72=FACE_OUTER_BOUND('',#71,.T.);
#73=ADVANCED_FACE('',(#72),#63,.T.);
#74=CARTESIAN_POINT('',(5.2,4.,0.));
#75=VERTEX_POINT('',#74);
#80=OPEN_SHELL('',(#54,#59,#73));
#81=SHELL_BASED_SURFACE_MODEL('',(#80));
#82=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('part',(#81,#13),#5);
/* the assembly: the part at x = 0 and turned to lie along x at z = 10 */
#90=SHAPE_REPRESENTATION('assembly',(#13),#5);
#91=CARTESIAN_POINT('',(0.,3.,10.));
#92=DIRECTION('',(1.,0.,0.));
#93=DIRECTION('',(0.,0.,-1.));
#94=AXIS2_PLACEMENT_3D('',#91,#92,#93);
#95=ITEM_DEFINED_TRANSFORMATION('','',#13,#94);
#96=(REPRESENTATION_RELATIONSHIP('','',#82,#90) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#95) SHAPE_REPRESENTATION_RELATIONSHIP());
#97=ITEM_DEFINED_TRANSFORMATION('','',#13,#13);
#98=(REPRESENTATION_RELATIONSHIP('','',#82,#90) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#97) SHAPE_REPRESENTATION_RELATIONSHIP());
ENDSEC;
END-ISO-10303-21;
";
        let read = read_step(text).unwrap();
        assert!((read.metres_per_unit.unwrap() - 0.0254).abs() < 1e-12);
        let mut tubes = read.parts.tubes.clone();
        tubes.sort_by(|a, b| a.a[2].total_cmp(&b.a[2]));
        assert_eq!(tubes.len(), 2, "{tubes:?}");
        // In place: up the z axis from 0 to 8.
        let t = tubes[0];
        assert!(
            close(t.a, [0.0, 0.0, 0.0], 1e-9) && close(t.b, [0.0, 0.0, 8.0], 1e-9),
            "{t:?}"
        );
        assert!((t.radius - 0.5).abs() < 1e-12);
        // Moved: the part's z is the assembly's x, at y = 3, z = 10.
        let t = tubes[1];
        let ends = [t.a, t.b];
        assert!(
            ends.iter().any(|p| close(*p, [0.0, 3.0, 10.0], 1e-9))
                && ends.iter().any(|p| close(*p, [8.0, 3.0, 10.0], 1e-9)),
            "{t:?}"
        );
        // The fillet, twice (once a placement), isn't a tube.
        assert_eq!(read.parts.other, 2);
    }

    /// A file written by OpenCascade (FreeCAD's CAD kernel): an assembly
    /// with a rod, a pipe, two tubes fused into a T, a block, a ring and a
    /// part used twice, once turned.
    #[test]
    fn an_opencascade_assembly() {
        let read = read_step(include_str!("../tests/cad/occ_assembly.step")).unwrap();
        assert_eq!(read.metres_per_unit, Some(0.001));
        let tubes = &read.parts.tubes;
        let has = |a: P3, b: P3, r: f64, tol: f64| {
            tubes.iter().any(|t| {
                (t.radius - r).abs() < 1e-9
                    && ((close(t.a, a, tol) && close(t.b, b, tol))
                        || (close(t.a, b, tol) && close(t.b, a, tol)))
            })
        };
        assert_eq!(tubes.len(), 6, "{tubes:?}");
        assert!(has([0.0, 0.0, 0.0], [0.0, 0.0, 100.0], 5.0, 1e-6), "rod");
        assert!(has([0.0, 0.0, 50.0], [200.0, 0.0, 50.0], 6.0, 1e-6), "pipe");
        assert!(
            has([0.0, 300.0, 0.0], [80.0, 300.0, 0.0], 3.0, 1e-6),
            "T across"
        );
        // The upright of the T starts between the cross piece's axis and
        // its surface (where the wires should join).
        assert!(
            has([40.0, 300.0, 1.5], [40.0, 300.0, 60.0], 2.0, 1.6),
            "T upright: {tubes:?}"
        );
        assert!(
            has([1000.0, -40.0, 20.0], [1000.0, 40.0, 20.0], 1.5, 1e-6),
            "director"
        );
        assert!(
            has([1160.0, 0.0, 20.0], [1240.0, 0.0, 20.0], 1.5, 1e-6),
            "turned director"
        );
        assert!(read.parts.flat >= 6, "{:?}", read.parts);
        assert!(read.parts.other >= 1, "the ring");
    }

    /// Thin sheets made in OpenCascade (a revolved paraboloid dish, a
    /// thickened spherical cap, a box plate tilted 30 degrees, two boxes
    /// fused into a corner reflector) come back as reflectors.
    #[test]
    fn opencascade_sheets_are_fitted() {
        use crate::sheetfit::Shape;
        let read = read_step(include_str!("../tests/cad/occ_sheets.step")).unwrap();
        assert!(read.parts.tubes.is_empty());
        assert_eq!((read.parts.flat, read.parts.other), (0, 0));
        let find = |want: &dyn Fn(&Shape) -> bool| {
            read.sheets
                .iter()
                .find(|s| want(&s.shape))
                .unwrap_or_else(|| panic!("{:#?}", read.sheets))
        };
        let near = |a: f64, b: f64, tol: f64| (a - b).abs() <= tol;
        let dish = find(&|s| matches!(s, Shape::Paraboloid(_)));
        let Shape::Paraboloid(f) = dish.shape else {
            unreachable!()
        };
        assert!(
            near(f, 200.0, 0.2) && near(dish.facing[1], 90.0, 0.05),
            "{dish:?}"
        );
        assert!(
            near(dish.vertex[0], 2000.0, 0.3) && near(dish.vertex[2], -1.0, 0.1),
            "{dish:?}"
        );
        let cap = find(&|s| matches!(s, Shape::Sphere(_)));
        let Shape::Sphere(r) = cap.shape else {
            unreachable!()
        };
        assert!(
            near(r, 801.5, 0.5) && near(cap.thickness, 3.0, 0.05),
            "{cap:?}"
        );
        assert!(
            near(cap.facing[0].abs(), 180.0, 0.05) && near(cap.facing[1], 0.0, 0.05),
            "{cap:?}"
        );
        let plate = find(&|s| matches!(s, Shape::Flat));
        assert!(
            near(plate.size[0], 600.0, 0.5) && near(plate.size[1], 400.0, 0.5),
            "{plate:?}"
        );
        assert!(
            near(plate.thickness, 3.0, 0.15) && near(plate.facing[1], 60.0, 0.01),
            "{plate:?}"
        );
        let corner = find(&|s| matches!(s, Shape::Vee(_)));
        let Shape::Vee(angle) = corner.shape else {
            unreachable!()
        };
        assert!(
            near(angle, 90.0, 0.3) && near(corner.facing[0], 45.0, 0.05),
            "{corner:?}"
        );
        assert!(near(corner.thickness, 2.0, 0.1), "{corner:?}");
    }

    /// Our own export of a reflector as a solid sheet reads back as the
    /// reflector it was: shape, focal length, place and wire thickness.
    #[test]
    fn exported_sheets_come_back_as_reflectors() {
        use crate::sheetfit::Shape;
        // A parabolic cylinder (f = 0.3 m) 0.8 x 0.6 m, its aperture 40 mm
        // off the vertex at x = -0.3, facing +x, 2 mm wire.
        let mut wires = Vec::new();
        let at = |i: usize, j: usize| {
            let u = 0.04 + 0.8 * (i as f64 / 16.0 - 0.5);
            let v = 0.6 * (j as f64 / 12.0 - 0.5);
            [-0.3 + u * u / 1.2, u, v]
        };
        for i in 0..=16 {
            for j in 0..=12 {
                if i < 16 {
                    wires.push((at(i, j), at(i + 1, j)));
                }
                if j < 12 {
                    wires.push((at(i, j), at(i, j + 1)));
                }
            }
        }
        let bodies = [
            Body {
                name: "element".into(),
                rods: vec![Rod {
                    name: "dipole".into(),
                    a: [0.0, -0.06, 0.0],
                    b: [0.0, 0.06, 0.0],
                    radius: 0.001,
                    inner: None,
                }],
                sheets: Vec::new(),
            },
            Body {
                name: "dish1".into(),
                rods: Vec::new(),
                sheets: vec![Mesh {
                    name: "dish1".into(),
                    wires,
                    thickness: 0.002,
                    radius: 0.001,
                }],
            },
        ];
        let read = read_step(&write(&bodies, "test")).unwrap();
        assert_eq!(read.parts.tubes.len(), 1);
        assert_eq!(read.sheets.len(), 1);
        let fit = read.sheets[0].scaled(0.001);
        let Shape::ParabolicCylinder(f) = fit.shape else {
            panic!("{fit:?}")
        };
        let near = |a: f64, b: f64, tol: f64| (a - b).abs() <= tol;
        assert!(near(f, 0.3, 0.0005), "{fit:?}");
        assert!(
            near(fit.vertex[0], -0.3, 0.0002) && near(fit.offset[0], 0.04, 0.001),
            "{fit:?}"
        );
        assert!(
            near(fit.facing[0], 0.0, 0.05) && near(fit.facing[1], 0.0, 0.05),
            "{fit:?}"
        );
        assert!(
            near(fit.size[0], 0.8, 0.0005) && near(fit.size[1], 0.6, 0.0005),
            "{fit:?}"
        );
        let spec = fit.reflector("dish1", fit.cells(0.05));
        assert!(near(spec.wire_radius, 0.001, 0.00005), "{spec:?}");
        assert_eq!(spec.cells, [16, 12]);
        assert_eq!(spec.facing, [0.0, 0.0]);
        // It builds into a reflector the deck reads back.
        let mut deck = crate::deck::Deck::parse("CE
GE 0
FR 0 1 0 0 1200
EN
");
        deck.add_reflector(&spec, 1).unwrap();
        deck.expand().unwrap();
        let sets = crate::deck::reflectors::reflector_sets(&deck);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].cells, [16, 12]);
    }

    #[test]
    fn bad_files_say_why() {
        assert!(read_step("solid nothing").unwrap_err().contains("STEP"));
        let broken =
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=CARTESIAN_POINT('',(1.,2.,;\nENDSEC;\n";
        assert!(read_step(broken).unwrap_err().starts_with("line 5"));
        let empty = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
        let read = read_step(empty).unwrap();
        assert!(read.parts.tubes.is_empty());
    }

    #[test]
    fn faceted_solids_go_through_the_mesh_reader() {
        // A 16-sided prism as a faceted B-rep.
        let mut w = String::from("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n");
        let n = 16;
        let mut id = 100;
        let mut faces = Vec::new();
        let point = |id: &mut u64, w: &mut String, p: P3| {
            *id += 1;
            let _ = writeln!(w, "#{id}=CARTESIAN_POINT('',({},{},{}));", p[0], p[1], p[2]);
            *id
        };
        let ring = |k: usize, z: f64| {
            let t = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
            [2.0 * t.cos(), 2.0 * t.sin(), z]
        };
        let mut poly = |id: &mut u64, w: &mut String, pts: Vec<P3>| {
            let ids: Vec<String> = pts
                .iter()
                .map(|p| format!("#{}", point(id, w, *p)))
                .collect();
            *id += 1;
            let lp = *id;
            let _ = writeln!(w, "#{lp}=POLY_LOOP('',({}));", ids.join(","));
            *id += 1;
            let _ = writeln!(w, "#{id}=FACE_OUTER_BOUND('',#{lp},.T.);");
            let b = *id;
            *id += 1;
            let _ = writeln!(w, "#{id}=FACE('',(#{b}));");
            faces.push(*id);
        };
        for k in 0..n {
            poly(
                &mut id,
                &mut w,
                vec![
                    ring(k, 0.0),
                    ring(k + 1, 0.0),
                    ring(k + 1, 30.0),
                    ring(k, 30.0),
                ],
            );
        }
        poly(&mut id, &mut w, (0..n).map(|k| ring(k, 0.0)).collect());
        poly(&mut id, &mut w, (0..n).map(|k| ring(k, 30.0)).collect());
        let list: Vec<String> = faces.iter().map(|f| format!("#{f}")).collect();
        let _ = writeln!(w, "#1=CLOSED_SHELL('',({}));", list.join(","));
        let _ = writeln!(w, "#2=FACETED_BREP('',#1);");
        let _ = writeln!(w, "#3=FACETED_BREP_SHAPE_REPRESENTATION('',(#2),#4);");
        let _ = writeln!(w, "#4=REPRESENTATION_CONTEXT('','');\nENDSEC;");
        let read = read_step(&w).unwrap();
        assert_eq!(read.parts.tubes.len(), 1, "{:?}", read.parts);
        let t = read.parts.tubes[0];
        assert!((length(sub(t.b, t.a)) - 30.0).abs() < 1e-9);
        assert!((t.radius - 2.0).abs() < 1e-9);
        assert_eq!(read.metres_per_unit, None);
    }
}

#[cfg(test)]
mod sheets {
    use super::*;

    /// A mesh over a grid of points f(i, j), i < nu, j < nv, keeping the
    /// sides whose ends are both kept.
    pub fn grid(
        name: &str,
        nu: usize,
        nv: usize,
        thickness: f64,
        f: impl Fn(usize, usize) -> Option<P3>,
    ) -> Mesh {
        let mut wires = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let Some(p) = f(i, j) else { continue };
                if let Some(q) = (i + 1 < nu).then(|| f(i + 1, j)).flatten() {
                    wires.push((p, q));
                }
                if let Some(q) = (j + 1 < nv).then(|| f(i, j + 1)).flatten() {
                    wires.push((p, q));
                }
            }
        }
        Mesh {
            name: name.into(),
            wires,
            thickness,
            radius: thickness / 2.0,
        }
    }

    pub fn plate() -> Mesh {
        // 1.2 m x 0.8 m in the x = 0.3 plane, 4 mm thick.
        grid("plate", 7, 5, 0.004, |i, j| {
            Some([0.3, -0.6 + 0.2 * i as f64, 0.5 + 0.2 * j as f64])
        })
    }

    pub fn corner() -> Mesh {
        // A 90 degree corner: two 0.5 m sides up z, 0.6 m tall.
        grid("corner", 11, 4, 0.002, |i, j| {
            let s = (i as f64 - 5.0) * 0.1;
            let h = std::f64::consts::FRAC_1_SQRT_2;
            Some([-s.abs() * h, s * h, 0.2 * j as f64])
        })
    }

    pub fn dish() -> Mesh {
        // A paraboloid 1 m across, f = 0.4 m, facing +x, round outline.
        grid("dish", 13, 13, 0.003, |i, j| {
            let (u, v) = ((i as f64 - 6.0) / 12.0, (j as f64 - 6.0) / 12.0);
            (u * u + v * v <= 0.25 + 1e-9).then(|| [2.0 + (u * u + v * v) / 1.6, u, v])
        })
    }

    fn volume(p: &Polyhedron) -> f64 {
        // Sum over faces of (centroid . normal) x area / 3, the faces fanned.
        let mut v = 0.0;
        for f in &p.faces {
            for ring in &f.loops {
                for k in 1..ring.len() - 1 {
                    let (a, b, c) = (p.points[ring[0]], p.points[ring[k]], p.points[ring[k + 1]]);
                    v += dot(a, cross(b, c)) / 6.0;
                }
            }
        }
        v
    }

    #[test]
    fn a_flat_plate_is_a_slab() {
        let solid = sheet_solid(&plate()).unwrap();
        assert_eq!(solid.faces.len(), 6, "{:?}", solid.faces.len());
        let expected = 1200.0 * 800.0 * 4.0;
        assert!(
            (volume(&solid) - expected).abs() < 1e-6 * expected,
            "{}",
            volume(&solid)
        );
    }

    #[test]
    fn a_corner_folds_at_full_thickness() {
        let solid = sheet_solid(&corner()).unwrap();
        // Two flat sides, top and bottom (4 faces), the two ends, and the
        // top and bottom rims, each one V-shaped face across the fold.
        assert_eq!(solid.faces.len(), 8, "{}", solid.faces.len());
        let expected = 2.0 * 500.0 * 600.0 * 2.0;
        assert!(
            (volume(&solid) - expected).abs() < 1e-3 * expected,
            "{}",
            volume(&solid)
        );
    }

    #[test]
    fn a_dish_stays_curved_and_closed() {
        let mesh = dish();
        let solid = sheet_solid(&mesh).unwrap();
        // Every edge is used twice, once each way: a closed surface.
        let mut uses: HashMap<(usize, usize), i32> = HashMap::new();
        for f in &solid.faces {
            for ring in &f.loops {
                for i in 0..ring.len() {
                    *uses
                        .entry((ring[i], ring[(i + 1) % ring.len()]))
                        .or_default() += 1;
                }
            }
        }
        for (&(u, v), &n) in &uses {
            assert_eq!(n, 1);
            assert_eq!(uses.get(&(v, u)), Some(&1));
        }
        assert!(volume(&solid) > 0.0);
        // The four spokes past the rim's last whole cells.
        assert_eq!(solid.loose.len(), 4, "{:?}", solid.loose);
        // Its middle is where the wires are.
        let bump: f64 = solid.points.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
        assert!((bump - (2000.0 - 1.5)).abs() < 0.01, "{bump}");
    }

    #[test]
    fn meshes_that_cant_be_sheets_say_why() {
        let line = Mesh {
            name: "line".into(),
            wires: vec![([0.0; 3], [1.0, 0.0, 0.0])],
            thickness: 0.001,
            radius: 0.0005,
        };
        assert!(sheet_solid(&line).unwrap_err().contains("cells"));
        let mut thin = plate();
        thin.thickness = 0.0;
        assert!(sheet_solid(&thin).is_err());
    }

    /// Writes a sample (a hollow-tube dipole on a mast, a tilted rod, and
    /// a plate, a corner and a dish as sheets) to the temp folder, to
    /// check in other CAD software.
    #[test]
    #[ignore]
    fn sample_step() {
        let rod = |name: &str, a: P3, b: P3, radius: f64, inner: Option<f64>| Rod {
            name: name.into(),
            a,
            b,
            radius,
            inner,
        };
        let sheet = |m: Mesh| Body {
            name: m.name.clone(),
            rods: Vec::new(),
            sheets: vec![m],
        };
        let bodies = [
            Body {
                name: "dipole and mast".into(),
                rods: vec![
                    rod(
                        "left",
                        [0.0, -5.1, 10.0],
                        [0.0, 0.0, 10.0],
                        0.006,
                        Some(0.0045),
                    ),
                    rod(
                        "right",
                        [0.0, 0.0, 10.0],
                        [0.0, 5.1, 10.0],
                        0.006,
                        Some(0.0045),
                    ),
                    rod("mast", [0.0, 0.0, 0.0], [0.0, 0.0, 10.0], 0.02, None),
                ],
                sheets: Vec::new(),
            },
            Body {
                name: "reflector".into(),
                rods: vec![rod("tilted", [1.0, 2.0, 3.0], [1.3, 2.4, 3.5], 0.001, None)],
                sheets: Vec::new(),
            },
            sheet(plate()),
            sheet(corner()),
            sheet(dish()),
        ];
        let path = std::env::temp_dir().join("straws-sample.step");
        std::fs::write(&path, write(&bodies, "sample")).unwrap();
        println!("{}", path.display());
    }
}
