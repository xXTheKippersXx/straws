//! Curved reflectors and dishes as wire meshes: an outline (rectangle,
//! ellipse or circle, regular polygon) and a surface profile (flat,
//! parabolic cylinder, paraboloid, two curvatures, V, circular cylinder,
//! sphere, or z = f(x, y) from a formula), pointing any way, off-axis if
//! wanted.
//!
//! - The surface is z(u, v) above the plane through the vertex, where u runs
//!   across and v up the aperture and z towards the focus. Nodes sit on a
//!   regular grid over the aperture; those inside the outline are joined to
//!   their neighbours by one GW card per side (so NEC joins them at every
//!   node), one tag per mesh row or column.
//! - Variables `NAME_x/y/z` (the vertex), `NAME_az`/`NAME_el` (the direction
//!   it faces: azimuth, elevation), `NAME_w`/`NAME_l` (aperture across, up),
//!   `NAME_u0`/`NAME_v0` (the aperture's centre off the axis, for offset
//!   dishes), `NAME_f` (focal length; `NAME_f2` for the second curvature),
//!   `NAME_r` (radius, for cylinders and spheres), `NAME_angle` (a V's
//!   angle), `NAME_wire`. Only the ones its profile uses are made.
//! - The comment `straws: reflector NAME OUTLINE PROFILE NXxNY [z=formula]`
//!   on each card keeps what it is.

use super::expr;
use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};

pub const REFLECTOR_MARK: &str = "straws: reflector";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outline {
    Rectangle,
    /// An ellipse (a circle when as wide as it's high).
    Ellipse,
    /// A regular polygon with this many sides.
    Polygon(usize),
}

impl Outline {
    pub const CHOICES: [Outline; 4] = [
        Outline::Ellipse,
        Outline::Rectangle,
        Outline::Polygon(6),
        Outline::Polygon(8),
    ];

    fn key(self) -> String {
        match self {
            Outline::Rectangle => "rect".into(),
            Outline::Ellipse => "ellipse".into(),
            Outline::Polygon(n) => format!("poly{n}"),
        }
    }

    fn from_key(k: &str) -> Option<Outline> {
        match k {
            "rect" => Some(Outline::Rectangle),
            "ellipse" => Some(Outline::Ellipse),
            _ => k
                .strip_prefix("poly")?
                .parse()
                .ok()
                .filter(|n| (3..=24).contains(n))
                .map(Outline::Polygon),
        }
    }

    pub fn label(self) -> String {
        match self {
            Outline::Rectangle => "rectangle".into(),
            Outline::Ellipse => "circle / ellipse".into(),
            Outline::Polygon(n) => format!("{n}-sided"),
        }
    }

    /// Whether (a, b), fractions of the width and height from the middle
    /// (-0.5 to 0.5), is inside.
    fn inside(self, a: f64, b: f64) -> bool {
        let (x, y) = (2.0 * a, 2.0 * b);
        match self {
            Outline::Rectangle => x.abs() <= 1.0 + 1e-9 && y.abs() <= 1.0 + 1e-9,
            Outline::Ellipse => x * x + y * y <= 1.0 + 1e-9,
            Outline::Polygon(n) => {
                // Inscribed in the ellipse, a corner at the top.
                let (r, t) = (x.hypot(y), y.atan2(x));
                let sector = 2.0 * std::f64::consts::PI / n as f64;
                let t = (t - std::f64::consts::FRAC_PI_2).rem_euclid(sector) - sector / 2.0;
                r * t.cos() <= (sector / 2.0).cos() + 1e-9
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Profile {
    Flat,
    /// Curved across only (a focal line).
    ParabolicCylinder,
    /// A dish.
    Paraboloid,
    /// Focal length `f` across and `f2` up.
    TwoCurvatures,
    /// A V folded along the up line, with this angle between its sides.
    Vee,
    /// A circular cylinder of radius `r`, curved across.
    Cylinder,
    /// A sphere of radius `r`.
    Sphere,
    /// z = this formula of x (across) and y (up), in metres, with any
    /// variables.
    Formula(String),
}

impl Profile {
    pub const CHOICES: [&'static str; 8] = [
        "paraboloid (dish)",
        "parabolic cylinder",
        "two curvatures",
        "flat",
        "V (corner)",
        "circular cylinder",
        "sphere",
        "z = f(x, y)",
    ];

    pub fn index(&self) -> usize {
        match self {
            Profile::Paraboloid => 0,
            Profile::ParabolicCylinder => 1,
            Profile::TwoCurvatures => 2,
            Profile::Flat => 3,
            Profile::Vee => 4,
            Profile::Cylinder => 5,
            Profile::Sphere => 6,
            Profile::Formula(_) => 7,
        }
    }

    pub fn from_index(k: usize, formula: &str) -> Profile {
        match k {
            0 => Profile::Paraboloid,
            1 => Profile::ParabolicCylinder,
            2 => Profile::TwoCurvatures,
            3 => Profile::Flat,
            4 => Profile::Vee,
            5 => Profile::Cylinder,
            6 => Profile::Sphere,
            _ => Profile::Formula(formula.to_string()),
        }
    }

    fn key(&self) -> String {
        match self {
            Profile::Flat => "flat".into(),
            Profile::ParabolicCylinder => "pcyl".into(),
            Profile::Paraboloid => "dish".into(),
            Profile::TwoCurvatures => "two".into(),
            Profile::Vee => "vee".into(),
            Profile::Cylinder => "cyl".into(),
            Profile::Sphere => "sphere".into(),
            Profile::Formula(_) => "formula".into(),
        }
    }

    fn from_key(k: &str, formula: Option<&str>) -> Option<Profile> {
        Some(match k {
            "flat" => Profile::Flat,
            "pcyl" => Profile::ParabolicCylinder,
            "dish" => Profile::Paraboloid,
            "two" => Profile::TwoCurvatures,
            "vee" => Profile::Vee,
            "cyl" => Profile::Cylinder,
            "sphere" => Profile::Sphere,
            "formula" => Profile::Formula(formula?.to_string()),
            _ => return None,
        })
    }

    /// The variables this profile uses beyond the common ones.
    fn variables(&self) -> &'static [&'static str] {
        match self {
            Profile::ParabolicCylinder | Profile::Paraboloid => &["f"],
            Profile::TwoCurvatures => &["f", "f2"],
            Profile::Vee => &["angle"],
            Profile::Cylinder | Profile::Sphere => &["r"],
            Profile::Flat | Profile::Formula(_) => &[],
        }
    }

    /// The depth towards the focus at (u, v), as a formula of `u` and `v`
    /// texts (themselves formulas) and the reflector's variables.
    fn depth(&self, p: &str, u: &str, v: &str) -> String {
        let var = |s: &str| format!("{p}_{s}");
        match self {
            Profile::Flat => "0".into(),
            Profile::ParabolicCylinder => format!("({u})^2/(4*{})", var("f")),
            Profile::Paraboloid => format!("(({u})^2+({v})^2)/(4*{})", var("f")),
            Profile::TwoCurvatures => {
                format!("(({u})^2/(4*{})+({v})^2/(4*{}))", var("f"), var("f2"))
            }
            Profile::Vee => format!("abs({u})/tan({}/2)", var("angle")),
            Profile::Cylinder => format!("({r}-sqr({r}^2-({u})^2))", r = var("r")),
            Profile::Sphere => format!("({r}-sqr({r}^2-({u})^2-({v})^2))", r = var("r")),
            Profile::Formula(f) => {
                let f = expr::replace_name(f, "x", &format!("({u})"));
                format!("({})", expr::replace_name(&f, "y", &format!("({v})")))
            }
        }
    }

    /// The focus's distance from the vertex along the axis, if it has one.
    pub fn focus(&self, get: impl Fn(&str) -> Option<f64>) -> Option<f64> {
        match self {
            Profile::ParabolicCylinder | Profile::Paraboloid | Profile::TwoCurvatures => get("f"),
            Profile::Cylinder | Profile::Sphere => get("r").map(|r| r / 2.0),
            _ => None,
        }
    }

    fn focus_formula(&self, p: &str) -> Option<String> {
        match self {
            Profile::ParabolicCylinder | Profile::Paraboloid | Profile::TwoCurvatures => {
                Some(format!("{p}_f"))
            }
            Profile::Cylinder | Profile::Sphere => Some(format!("{p}_r/2")),
            _ => None,
        }
    }
}

/// A new reflector (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectorSpec {
    pub name: String,
    pub outline: Outline,
    pub profile: Profile,
    /// The vertex, before the Ground view's height shift.
    pub vertex: [f64; 3],
    /// Direction it faces: azimuth and elevation.
    pub facing: [f64; 2],
    /// Aperture across and up.
    pub size: [f64; 2],
    /// The aperture's centre off the axis (across, up).
    pub offset: [f64; 2],
    /// Focal length (and the second one), or radius, or V angle, as used.
    pub focal: [f64; 2],
    pub radius: f64,
    pub angle: f64,
    pub cells: [usize; 2],
    pub segments: i64,
    pub wire_radius: f64,
}

/// A reflector found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectorSet {
    pub name: String,
    pub outline: Outline,
    pub profile: Profile,
    pub cells: [usize; 2],
    pub tags: Vec<i64>,
    pub lines: Vec<usize>,
}

/// The reflectors in a deck (made by [`Deck::add_reflector`]).
pub fn reflector_sets(deck: &Deck) -> Vec<ReflectorSet> {
    let mut out: Vec<ReflectorSet> = Vec::new();
    for (i, line) in deck.lines.iter().enumerate() {
        let LineKind::Card { mnemonic, fields } = &line.kind else {
            continue;
        };
        let Some(rest) = line
            .comment
            .as_deref()
            .and_then(|c| c.strip_prefix(REFLECTOR_MARK))
        else {
            continue;
        };
        if mnemonic != "GW" {
            continue;
        }
        let (rest, formula) = match rest.split_once(" z=") {
            Some((r, f)) => (r, Some(f.trim())),
            None => (rest, None),
        };
        let w: Vec<&str> = rest.split_whitespace().collect();
        let [name, outline, profile, size] = w[..] else {
            continue;
        };
        let (Some(outline), Some(profile)) = (
            Outline::from_key(outline),
            Profile::from_key(profile, formula),
        ) else {
            continue;
        };
        let Some((Ok(nx), Ok(ny))) = size
            .split_once('x')
            .map(|(a, b)| (a.parse::<usize>(), b.parse::<usize>()))
        else {
            continue;
        };
        let tag = fields
            .first()
            .and_then(|t| t.parse::<f64>().ok())
            .map_or(0, |t| t as i64);
        let k = match out.iter().position(|r| r.name == name) {
            Some(k) => k,
            None => {
                out.push(ReflectorSet {
                    name: name.to_string(),
                    outline,
                    profile,
                    cells: [nx, ny],
                    tags: Vec::new(),
                    lines: Vec::new(),
                });
                out.len() - 1
            }
        };
        let r = &mut out[k];
        r.lines.push(i);
        if !r.tags.contains(&tag) {
            r.tags.push(tag);
        }
    }
    out
}

/// What a reflector variable is, in a few words.
pub fn reflector_label(suffix: &str) -> &'static str {
    match suffix {
        "x" => "vertex x",
        "y" => "vertex y",
        "z" => "vertex height",
        "az" => "faces (azimuth)",
        "el" => "faces (elevation)",
        "w" => "aperture across",
        "l" => "aperture up",
        "u0" => "offset across",
        "v0" => "offset up",
        "f" => "focal length",
        "f2" => "focal length (up)",
        "r" => "radius",
        "angle" => "V angle",
        "wire" => "wire radius",
        _ => "",
    }
}

/// `+f*term`, `-f*term`, or nothing for 0.
fn times(f: f64, term: &str) -> String {
    if f.abs() < 1e-12 {
        String::new()
    } else if f < 0.0 {
        format!("-{}*{term}", format_real(-f))
    } else {
        format!("+{}*{term}", format_real(f))
    }
}

/// A point `u` across, `v` up and `w` towards the focus from the vertex,
/// as deck formulas (u, v, w are formulas too).
fn point(p: &str, u: &str, v: &str, w: &str) -> [String; 3] {
    let var = |s: &str| format!("{p}_{s}");
    let (az, el) = (var("az"), var("el"));
    // across = (-sin az, cos az, 0); up = (-sin el cos az, -sin el sin az,
    // cos el); axis = (cos el cos az, cos el sin az, sin el).
    [
        format!(
            "{}-({u})*sin({az})-({v})*sin({el})*cos({az})+({w})*cos({el})*cos({az})",
            var("x")
        ),
        format!(
            "{}+({u})*cos({az})-({v})*sin({el})*sin({az})+({w})*cos({el})*sin({az})",
            var("y")
        ),
        format!("{}+({v})*cos({el})+({w})*sin({el})", var("z")),
    ]
}

/// The reflector's cards: rows then columns of the mesh inside the outline.
fn reflector_cards(
    p: &str,
    outline: Outline,
    profile: &Profile,
    cells: [usize; 2],
    first_tag: i64,
    segments: i64,
) -> Vec<String> {
    let var = |s: &str| format!("{p}_{s}");
    let formula = match profile {
        Profile::Formula(f) => format!(" z={f}"),
        _ => String::new(),
    };
    let mark = format!(
        "{REFLECTOR_MARK} {p} {} {} {}x{}{formula}",
        outline.key(),
        profile.key(),
        cells[0],
        cells[1]
    );
    let [nx, ny] = cells;
    let frac = |i: usize, n: usize| i as f64 / n as f64 - 0.5;
    let inside = |i: usize, j: usize| outline.inside(frac(i, nx), frac(j, ny));
    let node = |i: usize, j: usize| -> [String; 3] {
        let u = format!("{}{}", var("u0"), times(frac(i, nx), &var("w")));
        let v = format!("{}{}", var("v0"), times(frac(j, ny), &var("l")));
        let w = profile.depth(p, &u, &v);
        point(p, &u, &v, &w)
    };
    let card = |tag: i64, a: [String; 3], b: [String; 3]| {
        let mut f = vec![tag.to_string(), segments.to_string()];
        f.extend(a);
        f.extend(b);
        f.push(var("wire"));
        super::edit::card_line("GW", &f, Some(&mark))
    };
    let mut out = Vec::new();
    let mut tag = first_tag;
    for j in 0..=ny {
        let mut any = false;
        for i in 0..nx {
            if inside(i, j) && inside(i + 1, j) {
                out.push(card(tag, node(i, j), node(i + 1, j)));
                any = true;
            }
        }
        tag += i64::from(any);
    }
    for i in 0..=nx {
        let mut any = false;
        for j in 0..ny {
            if inside(i, j) && inside(i, j + 1) {
                out.push(card(tag, node(i, j), node(i, j + 1)));
                any = true;
            }
        }
        tag += i64::from(any);
    }
    out
}

fn check_cells(cells: [usize; 2]) -> Result<(), String> {
    if cells.iter().any(|&n| !(2..=80).contains(&n)) {
        return Err("a reflector has 2 to 80 cells each way".into());
    }
    Ok(())
}

/// The focus (if the profile has one), as a point worked out from the
/// variables `get` gives (vertex in the deck's own coordinates).
pub fn focus_point(profile: &Profile, get: impl Fn(&str) -> Option<f64>) -> Option<[f64; 3]> {
    let d = profile.focus(&get)?;
    let (az, el) = (get("az")?.to_radians(), get("el")?.to_radians());
    Some([
        get("x")? + d * el.cos() * az.cos(),
        get("y")? + d * el.cos() * az.sin(),
        get("z")? + d * el.sin(),
    ])
}

impl Deck {
    /// Adds a reflector: its variables and cards, at [`Deck::geometry_end`],
    /// with tags clear of any in use.
    pub fn add_reflector(&mut self, spec: &ReflectorSpec, first_tag: i64) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the reflector needs a name".into());
        }
        check_cells(spec.cells)?;
        if spec.size.iter().any(|&s| s <= 0.0) || spec.wire_radius <= 0.0 {
            return Err("the aperture and the wire must be more than zero".into());
        }
        let c = spec.vertex.map(|v| format_real((v * 1e7).round() / 1e7));
        let n = format_real;
        let mut vars: Vec<(&str, String, &str, &str)> = vec![
            ("x", c[0].clone(), "m", "vertex x"),
            ("y", c[1].clone(), "m", "vertex y"),
            ("z", c[2].clone(), "m", "vertex height"),
            (
                "az",
                n(spec.facing[0]),
                "deg",
                "direction it faces (azimuth)",
            ),
            (
                "el",
                n(spec.facing[1]),
                "deg",
                "direction it faces (elevation)",
            ),
            ("w", n(spec.size[0]), "m", "aperture across"),
            ("l", n(spec.size[1]), "m", "aperture up"),
            ("u0", n(spec.offset[0]), "m", "aperture offset across"),
            ("v0", n(spec.offset[1]), "m", "aperture offset up"),
        ];
        for &k in spec.profile.variables() {
            vars.push(match k {
                "f" => ("f", n(spec.focal[0]), "m", "focal length"),
                "f2" => ("f2", n(spec.focal[1]), "m", "focal length (up)"),
                "r" => ("r", n(spec.radius), "m", "radius"),
                _ => ("angle", n(spec.angle), "deg", "angle of the V"),
            });
        }
        vars.push(("wire", n(spec.wire_radius), "m", "wire radius"));
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        if let Profile::Formula(f) = &spec.profile {
            expr::names_in(f).iter().try_for_each(|name| {
                let known = ["x", "y"].contains(&name.as_str())
                    || self.variables().iter().any(|v| v.name.eq_ignore_ascii_case(name))
                    || expr::RESERVED.contains(&name.as_str());
                if known {
                    Ok(())
                } else {
                    Err(format!("the formula uses \"{name}\", which isn't a variable (use x and y for across and up)"))
                }
            })?;
        }
        let first_tag = first_tag.max(self.highest_tag() + 1);
        let cards = reflector_cards(
            p,
            spec.outline,
            &spec.profile,
            spec.cells,
            first_tag,
            spec.segments.max(1),
        );
        if cards.is_empty() {
            return Err("no mesh fits inside that outline: use more cells".into());
        }
        for (name, (suffix, value, unit, description)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {description}"),
                unit: unit.to_string(),
                group: p.to_string(),
                fixed: matches!(*suffix, "x" | "y" | "z" | "az" | "el" | "wire"),
                ..VarNotes::default()
            };
            self.add_variable(name, value, &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// Rebuilds reflector `name` with another outline, profile or cell
    /// count, keeping its variables (adding any the new profile needs, from
    /// `spec`'s numbers) and segments.
    pub fn reshape_reflector(&mut self, name: &str, spec: &ReflectorSpec) -> Result<(), String> {
        check_cells(spec.cells)?;
        let Some(r) = reflector_sets(self).into_iter().find(|r| r.name == name) else {
            return Err(format!("there's no reflector called {name}"));
        };
        let segments = match &self.lines[r.lines[0]].kind {
            LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        };
        let before = self.clone();
        for &k in spec.profile.variables() {
            let full = format!("{name}_{k}");
            if !self
                .variables()
                .iter()
                .any(|v| v.name.eq_ignore_ascii_case(&full))
            {
                let (value, unit, what) = match k {
                    "f" => (spec.focal[0], "m", "focal length"),
                    "f2" => (spec.focal[1], "m", "focal length (up)"),
                    "r" => (spec.radius, "m", "radius"),
                    _ => (spec.angle, "deg", "angle of the V"),
                };
                let notes = VarNotes {
                    description: format!("{name} {what}"),
                    unit: unit.into(),
                    group: name.into(),
                    ..VarNotes::default()
                };
                self.add_variable(&full, &format_real(value), &notes)?;
            }
        }
        let r = reflector_sets(self)
            .into_iter()
            .find(|r| r.name == name)
            .ok_or("the reflector went missing")?;
        let first = r.tags.iter().copied().min().unwrap_or(1);
        let at = r.lines[0];
        self.remove_lines(&r.lines);
        let cards = reflector_cards(
            name,
            spec.outline,
            &spec.profile,
            spec.cells,
            first,
            segments,
        );
        if cards.is_empty() {
            *self = before;
            return Err("no mesh fits inside that outline: use more cells".into());
        }
        // Tags clear of others: if the old ones aren't enough, start afresh.
        let needed = (spec.cells[0] + spec.cells[1] + 2) as i64;
        let cards = if needed as usize <= r.tags.len() {
            cards
        } else {
            let fresh = self.highest_tag() + 1;
            reflector_cards(
                name,
                spec.outline,
                &spec.profile,
                spec.cells,
                fresh,
                segments,
            )
        };
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// A free name for a new reflector (`dish1`, `dish2`, ...).
    pub fn reflector_name(&self) -> String {
        let taken: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        (1..)
            .map(|i| format!("dish{i}"))
            .find(|name| {
                let prefix = format!("{name}_");
                !taken.iter().any(|v| v.starts_with(&prefix))
            })
            .unwrap_or_default()
    }

    /// Removes a reflector's cards and the variables only it used.
    pub fn remove_reflector(&mut self, name: &str) {
        let Some(r) = reflector_sets(self).into_iter().find(|r| r.name == name) else {
            return;
        };
        self.remove_lines(&r.lines);
        let prefix = format!("{name}_").to_ascii_lowercase();
        let mine: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name)
            .filter(|v| v.to_ascii_lowercase().starts_with(&prefix))
            .collect();
        for v in mine {
            if self.uses(&v).is_empty() {
                let _ = self.delete_variable(&v, None);
            }
        }
    }

    /// Adds a half-wave dipole (half-length `half`, `segments` segments,
    /// across the aperture) at reflector `name`'s focus, fed in the middle
    /// (the deck's first source moves to it). Its position is a formula of
    /// the reflector's variables, so it stays at the focus. Returns its tag.
    pub fn dipole_at_focus(
        &mut self,
        name: &str,
        half: f64,
        segments: i64,
        wire_radius: f64,
        tag: i64,
    ) -> Result<i64, String> {
        let r = reflector_sets(self)
            .into_iter()
            .find(|r| r.name == name)
            .ok_or(format!("there's no reflector called {name}"))?;
        let d = r
            .profile
            .focus_formula(name)
            .ok_or("this shape has no focus")?;
        let h = format_real(half);
        let end = |s: &str| point(name, &format!("{s}{h}"), "0", &d);
        let tag = tag.max(self.highest_tag() + 1);
        let segments = if segments % 2 == 0 {
            segments + 1
        } else {
            segments
        };
        let mut f = vec![tag.to_string(), segments.to_string()];
        f.extend(end("-"));
        f.extend(end(""));
        f.push(format_real(wire_radius));
        let line = super::edit::card_line("GW", &f, Some(&format!("{name} feed at the focus")));
        let at = self.geometry_end();
        self.insert_lines(at, &[line]);
        self.feed_wire(tag, segments);
        Ok(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(outline: Outline, profile: Profile) -> ReflectorSpec {
        ReflectorSpec {
            name: "dish1".into(),
            outline,
            profile,
            vertex: [0.0, 0.0, 0.0],
            facing: [0.0, 0.0],
            size: [2.0, 2.0],
            offset: [0.0, 0.0],
            focal: [0.8, 0.8],
            radius: 2.0,
            angle: 90.0,
            cells: [8, 8],
            segments: 1,
            wire_radius: 0.001,
        }
    }

    fn nodes(d: &Deck) -> Vec<[f64; 3]> {
        d.expand()
            .unwrap()
            .cards
            .iter()
            .filter(|c| c.is("GW"))
            .flat_map(|c| {
                [
                    [c.floats[0], c.floats[1], c.floats[2]],
                    [c.floats[3], c.floats[4], c.floats[5]],
                ]
            })
            .collect()
    }

    #[test]
    fn a_dish_is_a_paraboloid_inside_its_circle() {
        let mut d = Deck::parse("GE 0\nEN\n");
        d.add_reflector(&spec(Outline::Ellipse, Profile::Paraboloid), 1)
            .unwrap();
        let r = &reflector_sets(&d)[0];
        assert_eq!(
            (r.outline, r.profile.clone()),
            (Outline::Ellipse, Profile::Paraboloid)
        );
        // Facing +x: depth x = (y² + z²) / 4f, inside the 2 m circle.
        for p in nodes(&d) {
            assert!(
                (p[0] - (p[1] * p[1] + p[2] * p[2]) / 3.2).abs() < 1e-9,
                "{p:?}"
            );
            assert!(p[1].hypot(p[2]) <= 1.0 + 1e-9);
        }
        let focus = focus_point(&r.profile, |k| {
            d.variables()
                .iter()
                .find(|v| v.name == format!("dish1_{k}"))
                .and_then(|v| v.literal)
        });
        assert_eq!(focus, Some([0.8, 0.0, 0.0]));
        // A dipole at the focus, fed.
        let tag = d.dipole_at_focus("dish1", 0.24, 11, 0.001, 1).unwrap();
        let n = d.expand().unwrap();
        let dip = n
            .cards
            .iter()
            .find(|c| c.is("GW") && c.ints[0] == tag)
            .unwrap();
        assert!((dip.floats[0] - 0.8).abs() < 1e-9 && (dip.floats[1] + 0.24).abs() < 1e-9);
        assert!(
            n.cards
                .iter()
                .any(|c| c.is("EX") && c.ints[1] == tag && c.ints[2] == 6)
        );
    }

    #[test]
    fn other_shapes() {
        // A sphere: every node r from the centre r along the axis.
        let mut d = Deck::parse("GE 0\nEN\n");
        d.add_reflector(&spec(Outline::Rectangle, Profile::Sphere), 1)
            .unwrap();
        for p in nodes(&d) {
            assert!(((p[0] - 2.0).powi(2) + p[1] * p[1] + p[2] * p[2]).sqrt() - 2.0 < 1e-9);
        }
        // A formula: z = 0.1*x (a tilted plane) facing up (+z).
        let mut d = Deck::parse("GE 0\nEN\n");
        let mut s = spec(Outline::Polygon(6), Profile::Formula("0.1*x".into()));
        s.facing = [0.0, 90.0];
        d.add_reflector(&s, 1).unwrap();
        assert_eq!(
            reflector_sets(&d)[0].profile,
            Profile::Formula("0.1*x".into())
        );
        for p in nodes(&d) {
            // Across is +y, so the depth (now up, z) is 0.1 * y.
            assert!((p[2] - 0.1 * p[1]).abs() < 1e-9, "{p:?}");
        }
        // Reshaped into a V: a new variable for its angle.
        d.reshape_reflector("dish1", &spec(Outline::Rectangle, Profile::Vee))
            .unwrap();
        assert!(d.variables().iter().any(|v| v.name == "dish1_angle"));
        assert_eq!(reflector_sets(&d)[0].profile, Profile::Vee);
        let mut bad = spec(Outline::Ellipse, Profile::Formula("x*q".into()));
        bad.name = "dish2".into();
        assert!(d.add_reflector(&bad, 1).unwrap_err().contains("\"q\""));
    }
}
