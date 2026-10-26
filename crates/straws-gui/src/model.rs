//! The deck the app has open.

use std::path::{Path, PathBuf};

use straws_core::deck::edit::{Wire, wires};
use straws_core::deck::{Deck, DeckError, NumericDeck};
use straws_core::output::{Patch, Segment};

/// What File > New starts from: free space, 14.2 MHz, no wires yet.
const BLANK_DECK: &str = concat!(
    "CM New model\n",
    "CE\n",
    "GE 0\n",
    "GN -1\n",
    "FR 0 1 0 0 14.2 0\n",
    "EN\n",
);

pub struct LoadedModel {
    pub path: Option<PathBuf>,
    /// File name for titles.
    pub name: String,
    pub deck: Deck,
    /// The expanded deck, or why it couldn't be expanded.
    pub numeric: Result<NumericDeck, DeckError>,
    /// Segments as the engine laid them out (from a geometry-only run).
    pub segments: Vec<Segment>,
    /// Surface patches, from the same run.
    pub patches: Vec<Patch>,
    /// The deck's straight wires (GW cards), for editing.
    pub wires: Vec<Wire>,
    /// The deck text as last saved or opened, to tell if it has changed.
    pub saved_text: String,
    /// Bumped whenever `segments` is replaced, so views redraw.
    pub geometry_version: u64,
    /// Why the engine couldn't lay out the last geometry, if it couldn't.
    pub geometry_error: Option<String>,
}

/// Most segments on one wire, and in a whole model, accepted from an edit
/// (more would take minutes to solve; type them in the Deck view if meant).
const MAX_WIRE_SEGMENTS: i64 = 5000;
const MAX_SEGMENTS: usize = 20000;

/// Why an edited deck can't be used, in plain words.
pub fn validate(deck: &Deck, overrides: &[(String, f64)]) -> Result<(), String> {
    let n = deck.expand_with(overrides).map_err(|e| e.to_string())?;
    let geometry = &n.cards[..n.control_start()];
    for (k, c) in geometry.iter().enumerate() {
        if !c.is("GW") {
            continue;
        }
        let tag = c.ints[0];
        if c.ints[1] > MAX_WIRE_SEGMENTS {
            return Err(format!(
                "wire {tag} would have {} segments; {MAX_WIRE_SEGMENTS} is the most allowed here",
                c.ints[1]
            ));
        }
        if c.floats.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
            return Err(format!(
                "wire {tag} would have a coordinate that isn't a sensible number"
            ));
        }
        // A zero radius is only allowed when a GC card tapers the wire.
        let tapered = geometry.get(k + 1).is_some_and(|g| g.is("GC"));
        if c.floats[6] <= 0.0 && !tapered {
            return Err(format!("wire {tag} needs a radius greater than zero"));
        }
    }
    if n.estimated_unknowns() > MAX_SEGMENTS {
        return Err(format!(
            "the model would have about {} segments; {MAX_SEGMENTS} is the most allowed here",
            n.estimated_unknowns()
        ));
    }
    Ok(())
}

impl LoadedModel {
    pub fn load(path: &Path) -> std::io::Result<LoadedModel> {
        let deck = Deck::from_file(path)?;
        let name = path
            .file_name()
            .map_or_else(|| "deck".to_string(), |n| n.to_string_lossy().into_owned());
        Ok(LoadedModel::from_deck(Some(path.to_path_buf()), name, deck))
    }

    /// A new, unsaved model from deck text (e.g. a template).
    pub fn from_text(name: &str, text: &str) -> LoadedModel {
        let mut m = LoadedModel::from_deck(None, name.to_string(), Deck::parse(text));
        // Not saved anywhere yet: counts as changed, so it isn't lost.
        m.saved_text.clear();
        m
    }

    /// A new, empty model to draw in.
    pub fn blank() -> LoadedModel {
        LoadedModel::from_deck(None, "Untitled".into(), Deck::parse(BLANK_DECK))
    }

    fn from_deck(path: Option<PathBuf>, name: String, deck: Deck) -> LoadedModel {
        let mut m = LoadedModel {
            path,
            name,
            saved_text: deck.to_text(),
            deck,
            numeric: Err(DeckError::general("not read yet")),
            segments: Vec::new(),
            patches: Vec::new(),
            wires: Vec::new(),
            geometry_version: 0,
            geometry_error: None,
        };
        m.refresh(&[]);
        m
    }

    /// Works the deck out again after an edit (with variable overrides).
    pub fn refresh(&mut self, overrides: &[(String, f64)]) {
        self.numeric = self.deck.expand_with(overrides);
        self.wires = match &self.numeric {
            Ok(n) => wires(&self.deck, n),
            // Keep the last good wires on screen while the deck is broken.
            Err(_) => std::mem::take(&mut self.wires),
        };
    }

    /// Keeps an edit already made to `self.deck` if the deck still works,
    /// otherwise puts `before` back and says why. A deck that was already
    /// broken (e.g. as opened) accepts any edit, since it may be the fix.
    pub fn keep_if_valid(
        &mut self,
        before: Deck,
        overrides: &[(String, f64)],
    ) -> Result<(), String> {
        if self.numeric.is_ok()
            && let Err(why) = validate(&self.deck, overrides)
        {
            self.deck = before;
            self.refresh(overrides);
            return Err(why);
        }
        self.refresh(overrides);
        Ok(())
    }

    /// How far the Ground view's height card lifts the antenna now
    /// (metres). New geometry goes before that card, so take this off the
    /// height of a point on screen.
    pub fn lift(&self) -> f64 {
        self.numeric.as_ref().map_or(0.0, |n| {
            straws_core::deck::ground::height_now(n, &self.deck)
        })
    }

    /// Whether the deck has any structure for the engine to work on.
    pub fn has_structure(&self) -> bool {
        self.numeric.as_ref().is_ok_and(|n| {
            n.cards[..n.control_start()]
                .iter()
                .any(|c| matches!(c.mnemonic.as_str(), "GW" | "GA" | "GH" | "SP" | "SM" | "GF"))
        })
    }

    /// The next free wire number.
    pub fn next_tag(&self) -> i64 {
        let from_segments = self.segments.iter().map(|s| i64::from(s.tag)).max();
        let from_wires = self.wires.iter().map(|w| w.tag).max();
        // Radials on or under the ground are kept as comments: their tags
        // stay theirs.
        let from_buried = straws_core::deck::radials::kept_tags(&self.deck)
            .into_iter()
            .max();
        from_segments
            .max(from_wires)
            .max(from_buried)
            .unwrap_or(0)
            .max(0)
            + 1
    }

    /// Segments to draw: everything except helper wires the deck reader
    /// added (4nec2 current-source dummies, 9900 m away).
    pub fn visible_segments(&self) -> impl Iterator<Item = &Segment> {
        let helpers: &[i64] = match &self.numeric {
            Ok(n) => &n.helper_tags,
            Err(_) => &[],
        };
        self.segments
            .iter()
            .filter(move |s| !helpers.contains(&i64::from(s.tag)))
    }

    /// Absolute segment number of segment `seg` (1-based) on wire `tag`.
    /// Tag 0 means `seg` is already absolute, as in NEC.
    fn absolute_segment(&self, tag: i64, seg: i64) -> Option<u32> {
        if tag == 0 {
            return u32::try_from(seg).ok();
        }
        let mut on_tag: Vec<u32> = self
            .segments
            .iter()
            .filter(|s| i64::from(s.tag) == tag)
            .map(|s| s.number)
            .collect();
        on_tag.sort_unstable();
        on_tag
            .get(usize::try_from(seg).ok()?.checked_sub(1)?)
            .copied()
    }

    /// Absolute numbers of the segments that are fed: voltage sources, and
    /// for 4nec2 current sources the real feed segment behind the helper.
    pub fn feed_segments(&self) -> Vec<u32> {
        let Ok(n) = &self.numeric else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for c in &n.cards[n.control_start()..] {
            let (tag, seg) = match c.mnemonic.as_str() {
                "EX" if matches!(c.ints[0], 0 | 5) && !n.helper_tags.contains(&c.ints[1]) => {
                    (c.ints[1], c.ints[2])
                }
                "NT" if n.helper_tags.contains(&c.ints[0]) => (c.ints[2], c.ints[3]),
                _ => continue,
            };
            if let Some(s) = self.absolute_segment(tag, seg) {
                out.push(s);
            }
        }
        out
    }

    /// Centre and radius of a sphere round all visible wires (metres).
    pub fn bounds(&self) -> ([f32; 3], f32) {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        let mut any = false;
        for s in self.visible_segments() {
            let (a, b) = s.ends();
            for p in [a, b] {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            any = true;
        }
        for p in self.patches.iter().flat_map(Patch::corners) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
            any = true;
        }
        for w in &self.wires {
            for p in [w.a, w.b] {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            any = true;
        }
        if !any {
            return ([0.0; 3], 5.0);
        }
        let c = [0, 1, 2].map(|k| ((lo[k] + hi[k]) / 2.0) as f32);
        let r = ((0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt() / 2.0) as f32;
        (c, r.max(1e-3))
    }
}
