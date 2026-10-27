# User guide

A tour of STRAWS. Every view is a tab: drag tabs to rearrange them,
and View > Reset layout puts them back. Help > Keyboard and mouse lists the
shortcuts.

## Starting a model
With nothing open, the 3D model view offers three ways in:
- **Start from a template**: pick an antenna in the Templates view, set the
  design frequency, wire diameter, ground, height and the template's own
  settings (in wavelengths, with metres shown), then *Create this antenna*.
  Every dimension becomes a variable you can tune.
- **Start from a proven design**: the Templates view's *Design library* page
  has published designs (DK7ZB's 2 m Yagis, Trevor Marshall's biquad,
  Cebik's Moxon, the ARRL quad, the classic ground plane) with who designed
  them, what they published, where the dimensions come from and how to build
  them. *Make it for* a frequency or a band: a published design is scaled
  whole, its element diameter too, so it behaves exactly like the original
  (each dimension is a variable with the published value noted beside it);
  formula designs are worked out for your frequency and wire. *Create this
  antenna* also sets the SWR's reference impedance to the one the design is
  fed at (28 Ω for DK7ZB's 28 Ω Yagis).
- **Draw a new antenna**: an empty model in free space at 14.2 MHz. Choose
  *Draw* in the 3D model view and click to place wire ends; each click starts
  the next wire from the last end. Esc or right-click stops. *Front*, *Side* and
  *Top* choose the plane you draw on. Then select the wire to feed and click
  *Feed this wire* in the Wires view.
- **Open a deck**: any `.nec` file, including 4nec2's.
- **Import a drawing**: File > Import from CAD (DXF, STEP, STL), or drop a `.dxf`
  file on the window. Lines, polylines (with their arcs), arcs, circles,
  ellipses and splines become wires; curves are cut into straight pieces
  (10° each unless you choose otherwise). Check the units (the file usually
  says; the overall size shown tells you if they're wrong), set the wire
  diameter and the highest frequency (for the segment lengths), and untick
  layers you don't want, such as dimensions. Ends closer than the join
  distance become one point, so wires that touch in the drawing are joined.
  Each wire is named after its layer. Text, dimensions, hatches and block
  references are left out: explode blocks in your CAD program first. The
  wires go into a new model or the open one; then feed one to run it.
- **Import a solid model** (STEP, or STL as most CAD programs and
  3D-printing tools save): each straight round tube becomes a wire along its
  middle with the tube's own diameter (or one you choose). STEP files say
  their units and keep exact sizes; assemblies are followed, so a part used
  several times gives a wire each time, each in its place. A pipe's inside
  wall is left out, and a tube that meets another (a T) ends on the other's
  middle, where the wires join.
- **Solid sheets in a STEP model** (a plate, a corner reflector, a dish, as
  STRAWS writes them with *solid sheets* ticked, or as drawn in CAD) are
  listed in the import window under *Solid sheets*. Each can come in as one of
  STRAWS's reflectors made of a wire mesh: its shape (flat, V, parabolic
  cylinder, dish, two curvatures, circular cylinder or sphere), size, focal
  length or radius, which way it faces and where it sits are fitted to the
  sheet, and its wires are as thick as the sheet. The cells are chosen for a
  mesh of about a thirteenth of a wavelength at the highest frequency you
  give; change them there or later. Once imported, click the mesh in the 3D
  view to change anything in the reflector's panel. Untick a sheet to leave it
  out. STL files don't say their units:
  millimetres is assumed, and the overall size shown tells you if that's
  wrong. Each tube is listed with its length and diameter; untick a boom or
  mast you don't want as a wire. Flat pieces (tube ends, plates, the faces of
  square bars) and other shapes (bent tubes, dishes) are left out and
  counted; draw plates and dishes with Draw > Plate or Dish. A join distance
  a little larger than a feed gap closes it, meeting in the middle.

## Editing
- **3D model**: in *Select*, click a wire to select it (Ctrl-click for more)
  and drag its ends; every wire end at that point moves with it, so junctions
  stay joined. Ends snap to other wire ends and to the grid (step in the
  toolbar); hold Alt to place freely. Coordinates written as formulas never move
  when dragged: change their variables instead.
- **Wires**: every straight wire with exact numbers. Type a number (in the
  units chosen at the top) or a formula. Tools: add, feed, delete, make copies
  (arrays), mirror, and the material (metal, solid or hollow tube with a wall
  thickness; the skin depth shows whether the wall matters).
- **Deck**: the model as `.nec` text; it updates the model when you pause typing.
- **Variables**: a slider for every `SY` variable. Dragging recalculates the
  slider frequency at once; letting go reruns the sweep. Click a variable's
  name to select the wires that use it. *Edit* renames it (every formula that
  uses it follows), changes its value or formula (checked as you type),
  and sets a description, a group (variables in a group are listed
  together), a unit, the slider's range and step, and whether the optimiser
  may change it. *Delete* writes its current value back into the cards that
  use it.

## Drawing in 3D, including curves
Choose *Draw* in the 3D model view, then what to draw. Everything goes on the
grid plane facing you (use Front, Side or Top) through the last point you
placed, or through a wire or curve end under the mouse; the hint in the
corner says which plane and where.
- **Wire**: click ends one after another; Esc or right-click stops. Hold
  Shift to keep it to 15° steps.
- **Arc**: click its two ends (wire and curve ends snap), then move the mouse
  to bend it and click. The bend snaps to 15° steps, so a semicircle (180°)
  or a quarter circle (90°) is easy; the radius and angle show by the mouse.
- **Semicircle**: click its two ends, then the side it bulges to. Two of
  these make the curved ends of a folded dipole.
- **Circle**: click the centre, then a point on it.
- **Helix**: click the centre of its base, then a point for the radius; it
  runs towards you. **Spiral**: the centre, then the outer radius.
- **Radials**: click the hub (a vertical's base, say; wire ends snap). Four
  level quarter-wave radials appear, and their panel opens: change how many
  there are, their length, their droop below level (about 45° brings a
  ground plane near 50 ohms), the hub's position (height included) and the
  wire size. Each radial is an ordinary wire you can load or give a
  material; clicking one selects the whole set, and Del deletes it. A hub on
  a wire end written as a variable (`h`) follows it.

  **Raised, on the ground, or buried.** The panel's three buttons put the
  radials where you want them, and STRAWS picks how to work them out:
  - *Raised*: the radials are NEC wires. If they meet the antenna's base, the
    whole antenna goes up with them (the Ground view's height). Low raised
    radials over the quick reflection-coefficient ground switch the ground to
    Sommerfeld-Norton, which is right near the ground (a message says so).
  - *On the ground* or *Buried* (a negative hub height is the depth): NEC-2
    can't model wires there, so the radials aren't sent to the engine.
    Instead STRAWS estimates their effect on the feed impedance, efficiency and
    gain (Wait and Pope's method, checked against Brown, Lewis and Epstein's
    1937 measurements to within about 1-3 ohms, see docs/validation.md).
    Results that include it say "estimate". They're drawn dashed in 3D, at
    their depth.
  - Radials closer to the ground than ten wire radii or a thousandth of a
    wavelength count as on the ground (even Sommerfeld-Norton is unreliable
    there).
  - The estimate is rougher with only a few radials (8 or fewer), ignores
    burial depth (a few centimetres is a tiny part of the soil's skin depth;
    the panel warns when it isn't), and uses the first source. Over perfect
    ground buried radials make no difference. The optimiser and the
    convergence check include the estimate too (the optimiser runs slower
    for it: about five engine runs a design).
  - Saved files keep buried radials as comment lines; 4nec2 skips them and,
    on a reflection-coefficient ground, gets NEC's own radial screen in their
    place (marked in the GN card's comment; STRAWS ignores it when reading).

  **Ring.** Tick *ring: joining the tips* in a radial set's panel for a wire
  from each tip to the next (3 or more radials). It follows the set's length,
  droop and count, and is raised or buried with it. (Long radials plus a ring
  make a big loop that can resonate: rings usually go with short radials.)
- **Grid**: a counterpoise mesh. Click its middle (a vertical's base, say): a
  square grid a quarter wavelength across with 4 x 4 cells appears, and its
  panel opens: cells across and along (even, so there's a node in the
  middle), the centre, height, width, length, turn and wire size, and the same
  *Raised / On the ground / Buried* buttons as radials. Every crossing is a
  real junction (each side of a cell is its own wire). On or under the ground
  it's estimated as a wire mesh, which hasn't been checked against
  measurements like radials have; note that right round a vertical's base,
  where the return current is strongest, a mesh screens poorly unless its
  cells are small, so radials usually do better for the same wire.
- **Plate** and **Corner reflector**: reflectors made of a wire mesh (NEC
  models a solid sheet as a mesh spaced well under a tenth of a wavelength).
  Click a plate's centre, or the middle of a corner's fold; it faces (or opens
  towards) the fed wire, one wavelength across and high, with a mesh of a tenth
  of a wavelength at the top of the sweep and a 90° corner. Its panel has the
  cells, centre, width (each side's, for a corner), height, the direction it
  faces, how far a plate leans back or a corner's angle, and the wire size,
  plus the segment count and mesh spacing (orange when the mesh is coarser
  than 0.1 λ). Big meshes are slow: the solve time grows with the cube of the
  segments. For the textbook corner-reflector gain put the driven element
  about half a wavelength in front of the fold; bigger sheets give more gain
  (1 λ sides: about 10.4 dBi; 2 λ: the textbook 12.15 dBi).
- **Dish / curved reflector**: click the vertex (the middle of its back). A
  dish two wavelengths across with f/D 0.4 appears, facing the fed wire, its
  focus marked with a crosshair. In its panel pick the **outline** (circle or
  ellipse, rectangle, 6- or 8-sided) and the **profile**: paraboloid (dish),
  parabolic cylinder (curved one way, a focal line), two curvatures (a focal
  length each way), flat, V (corner), circular cylinder, sphere, or your own
  `z = f(x, y)` formula (x across, y up, z towards the front, all in metres,
  with any variables, e.g. `0.1*x^2`). Every size is a variable: the
  vertex, the direction it faces (azimuth and elevation), the aperture across
  and up, an offset (for an off-axis dish, the aperture's centre off the
  axis), the focal length(s) or radius, the V's angle and the wire.
  **Dipole at the focus** adds a half-wave dipole at the focus and moves the
  feed to it; it stays at the focus when the focal length changes. A bare
  dipole sends about half its power away from the dish: a 3 λ dish gives
  about 12.9 dBi with one, and 15.3 dBi with a reflector element a quarter
  wave behind the dipole (on the far side from the dish).
- **Patch box (closed surface)**: a closed metal box made of NEC surface
  patches (a vehicle body, an equipment box, a mast-head enclosure). Click the
  middle of its bottom face: a box half a wavelength long and deep and a
  quarter high appears, with patches about a tenth of a wavelength across.
  Its panel sets the patches along, across and up, and its variables (bottom
  centre, length, depth, height, the length's direction). NEC-2's patches
  only model *closed* surfaces, which is why it makes a whole box. A wire
  joins the box only at a patch's centre and at right angles to it: odd patch
  counts put a patch centre in the middle of every face, and wire ends snap
  there when drawing. A quarter-wave whip on a 1 m box at 300 MHz gives about
  46 + j25 Ω.

Surface patches from any deck (`SP`, `SM` and patch strips, several `SC`
cards after one `SP`) are drawn too, each as a square of its own area (which
is all NEC uses of a patch). Clicking a patch of a box drawn here selects the
box. The Checks view warns when patches are bigger than 0.04 λ² (NEC-2 asks
for at least 25 per square wavelength), and when they don't form a closed
surface (a flat sheet of patches: use a wire mesh, Draw > Plate, instead). A
surface standing on perfect or real ground may be open at the bottom: its
mirror image closes it.

Curves that join wires get segments of about the same length as theirs, as
NEC likes. Click a curve (with *Select*) to adjust it in a small panel: its
centre, radius, angles, turns, length, wire size and segments, feeding it in
the middle or at the start, or deleting it (or press Del; Esc closes the
panel). Every number is a variable, grouped under the curve's name, so it
also has a slider and the optimiser can change it.

Right-click a wire in the 3D view to feed it, delete it, or make a variable
from it (its length, spacing, height or angle).

The Wires view's **Split** cuts the selected wire into equal pieces. Every
segment stays where it was, and sources and loads stay on the same segment.

The Checks view warns when wires bend very sharply, where NEC is inaccurate
(only where the wires run within a couple of their thicknesses of each other for
a real part of a segment, so fans of thin wires such as discones are fine).

**Convergence check** (at the bottom of the Checks view): *Run the check* runs
the model again with about twice the segments on every wire (2n + 1, so a centre
feed stays in the centre) and shows how much the feed impedance, resonance, peak
gain and front-to-back move at the slider frequency. *Settled* (impedance within
2%, resonance within 0.3%, gain within 0.1 dB, F/B within 1 dB) means the model
has enough segments to trust; *not settled* means add segments and check again.
Surface patches aren't refined.

**Tolerance check** (below it): how carefully does it need building? Set the
build accuracy (say ±2 mm) and the number of builds, and *Run* makes that
many copies of the antenna with every wire end moved at random by up to the
accuracy along each axis (ends that meet move together, so joints stay
joined; ends on the ground stay on it), runs each, and shows the SWR, feed
impedance, resonance, peak gain and F/B as drawn and across the builds: one
build in ten comes out below the "lowest 10%" column and one in ten above the
"highest 10%". Builds run on every core. Arcs, helices and patches are built
as drawn. A 20 m three-element Yagi at ±2 mm moves its resonance by about
±3 kHz; a UHF Yagi at the same accuracy moves much more, which is the point.

**Sensitivity: which wire to trim** (below that): each wire is made a little
longer at its free tips (where you'd trim it; about a thousandth of a
wavelength by default) and the model run again. The table lists every wire,
the most effective first, with how far that moves the resonance, SWR, peak
gain and F/B; shortening it moves them the other way. Click a row to select
the wire. *Colour the wires in 3D by* colours each wire by how much it moves
the chosen figure (yellow most), until the model is next changed. Wires joined
at both ends (loops) and the mesh lines of plates, dishes and grids aren't
listed. On the 20 m test Yagi, 2 cm longer moves the resonance by -27 kHz on
the driven element and well under 1 kHz on the others, while the reflector
moves the F/B most.

## Making variables without touching the deck
Any number can become a named variable with its own slider:
- **Wire table**: right-click a number and choose *Make this a variable*.
- **Measurements**: select wires (in the 3D view or the table), then use
  *Make a variable* in the Wires view, *From the selected wires* in the
  Variables view, or right-click a wire in the 3D view:
  - *Length*: one wire, or *Same length* for several; you choose whether the
    middle, the start or the end stays put.
  - *Distance between the two wires*: the second wire you picked moves.
  - *Spacing along x, y or z*: with three or more wires they become equally
    spaced (a boom pitch), the lowest one staying put.
  - *Height of the lowest point* of the selected wires.
  - *Angle above horizontal*: tilts a wire about one end (inverted-V legs).
- **Ground view**: the ƒ buttons make the height, permittivity or
  conductivity a variable. Changing a value that a variable sets moves that
  variable's slider.
- **Material**: right-click a wire's Material cell in the wire table to make
  its conductivity or (for a tube) its wall thickness a variable.
- **Feedline view**: ƒ buttons for the cable length, balun ratio and loss,
  choke, shield length and a custom cable's values. They're kept in the deck
  (in a "Feedline" group), so they're saved with the model; a choke variable
  also sets the modelled choke.
- **Everything else** (sources, loads, transmission lines, arcs, helices,
  moves, frequency): *Make a variable from another number in the deck* at the
  bottom of the Variables view.
- *Add variable* makes one from a value or formula you type; then use its
  name in any number.

Variables are ordinary `SY` lines with their notes in the line's comment
(e.g. `SY boom=2 ' [unit=m group=Boom]`), so files still open in 4nec2.
Wherever a variable moves a wire end, the ends of other wires joined there
follow it with the same formula, so junctions stay joined (untick *Move
joined wire ends too* in the prompt if you don't want that).
- **Ground**: free space, perfect ground or real ground (types from city to
  salt water, or your own permittivity and conductivity), the method
  (Sommerfeld-Norton is accurate near the ground; reflection coefficient is
  quicker and fine above about 0.1 λ), and the height of the antenna's lowest
  point.
- View > Text size (or Ctrl + / Ctrl - / Ctrl 0) makes everything on screen
  bigger or smaller; it's kept for next time. View also switches the theme.
- Ctrl+Z and Ctrl+Y undo and redo anything. Edits that would leave the model
  unusable (both ends of a wire together, a zero radius, an impossible number)
  are refused with the reason.

## Results
- The **frequency slider** at the top moves every view. The sweep range and
  number of points are next to it; *Run sweep* recalculates.
- **Band** (next to the sweep): pick a band to sweep straight across it. The
  bands come from your choice at the top of the menu: one of the ITU's three
  regions (the amateur allocations of Region 1: Europe, Africa, the Middle
  East and northern Asia; Region 2: the Americas; Region 3: the rest of Asia
  and the Pacific), or your own list. Your licence may allow less (or more)
  than the ITU list, so *Edit my own…* lets you keep exactly your bands:
  start from a region and trim it, or add bands by hand. The chosen list's
  bands are shaded on the SWR plot, and each SWR goal in the Optimiser has a
  *band* menu that fills in a band's edges.
- **3D pattern**: gain in every direction, coloured from purple (weak) to yellow
  (strong). Hover for gain and angles. The component box in the toolbar picks
  total, vertical, horizontal or circular polarisation.
- **Pattern cuts**: the elevation and azimuth cuts through the peak (or angles
  you choose), with beamwidth and front-to-back.
- With two or more sources, a **Feed point** menu at the top of the SWR,
  Smith chart, Matching and Feedline views picks whose impedance they (and
  their exports) show: the active impedance, with the others driven as set
  in the Feeds view.
- **SWR and impedance**, **Smith chart**: against Z₀ in the toolbar. Click a
  plot to go to that frequency.
- **3D model > Currents**: wires coloured by current; hover for the RMS current
  and phase with 100 W in.
- **Near field**: the electric field on a plane (horizontal at a height, or
  vertical) for the power you set. Points above the ICNIRP general-public
  reference level are outlined in red. It's a guide, not a compliance assessment.
- **Checks**: problems with the geometry and how to fix them, and the average
  gain test. Click a problem to select its wires.

## Tuning
- **Weight and wind load** (a line under the Wires view's tools): the straight
  wires' weight from their material (solid, or a tube with its wall), their
  area facing the wind (every wire broadside, the worst case), and the force
  at the wind speed you type. Add the mast, boom and clamps yourself.
- **RDF** (in the 3D pattern view, under the peak): the receiving directivity
  factor, how far the peak stands above the average gain over every direction
  the pattern covers (the upper half over ground). For receive antennas
  (Beverages, K9AY loops, flags) it's the figure that matters: noise comes
  from everywhere, so a higher RDF hears the wanted direction better. Losses
  don't change it.
- **Receive** view: RDF and front-to-back across the sweep (from the
  sweep's patterns; F/B in the azimuth cut at the peak's elevation), with
  the chosen bands shaded and the slider frequency marked; click a plot to
  move the frequency there. Handy for seeing over which band a receive
  antenna (or a phased receive array) keeps its directivity.
- **Feeds**: every source in the model, one row each: voltage or current
  source (current sources are what phased arrays usually want, as coupling
  can't then skew the currents), amplitude and phase with sliders or exact
  numbers, and what each feed point does: its feed impedance with all the
  others driven too (the active impedance), SWR on its own feedline, the power
  it takes and its share, and the current it gets. The pattern follows as you
  drag. *Presets*: all in phase (broadside), end-fire in 90° steps (for
  elements a quarter wave apart), uniform amplitudes, a binomial taper, a
  Dolph-Chebyshev taper (choose how far down the side lobes are), a
  four-square beamed over any of its four corners or broadside to any of its
  four sides (eight directions), and *Steer towards* any azimuth (each element
  lags by how far it sits along that direction, worked out from where the
  elements are, so it suits any layout). *Set the total to* scales the whole
  array to the power you type. Each row's % box is its share of the power:
  type a share and its amplitude is set for it (coupling shifts it a little;
  set it again to fine-tune). *Shares in dB* shows each element's power in
  dB against a reference element you pick instead, and typing a level sets
  that element's amplitude the same way. ƒ makes a source's amplitude and phase variables (for
  sliders, snapshots and the optimiser); ✖ removes it. To add a source,
  right-click a wire in the 3D model view and choose *Add another source here*.
- **Feed network: phasing lines** (at the bottom of the Feeds view, with two
  or more sources): cuts a line of the cable you choose for each element so
  the currents set above all come from one feed point. The lengths come from
  the elements' active impedances (coupling included) at the slider
  frequency, with the cable's loss; a line's length sets the phase it gives
  and the amplitude follows, so the table shows how close it gets, and you
  can trade a little amplitude accuracy for shorter lines. A line under the
  table shows how the lines hold up across the sweep (from the elements'
  impedances at every sweep frequency): the worst share of the wanted
  current that's wrong, with the worst dB and degrees. Tick *Tune the lines
  over the sweep* to adjust the lengths together for the whole sweep rather
  than the slider frequency (set the sweep to your band first, e.g. with the
  Band menu). Phasing lines are narrowband by nature, so a wide band always
  costs some accuracy. *Make the currents exact with an L network at each
  element* adds a part across each line's end and one in series with the
  element, so every current is exactly what's wanted at the slider
  frequency (parts too small to matter are flagged; E12 values on hover);
  they go into the model with the lines. *Feed from a 90° hybrid coupler*
  instead hangs each element off one of an ideal quadrature hybrid's two
  outputs (A, or B a quarter cycle behind; pick per element) through its own
  line: the radio then sees a good match whatever the elements do, and what
  they reflect goes to the hybrid's dump load (the view says how much). A
  hybrid fixes the waves it sends, not the currents, so elements with very
  different impedances can't all get exactly what's wanted; the lines are
  the best possible and the error is shown. The hybrid is worked out by
  STRAWS and isn't written into the deck. Each line is at
  least long enough to reach from the junction (in the middle of the feed
  points). It also gives the junction's impedance (match it with the
  Matching view). *Write the lines into the model* replaces the separate
  sources with a tiny junction wire, one source and a NEC transmission line
  to each element, so NEC runs the whole network: the Feeds view then shows
  the **Phasing network** table, live as you move the frequency slider:
  what's wanted, NEC's currents (NEC's lines are lossless), and an estimate
  with the real cable's loss. *Back to separate sources* undoes it.
- **Optimiser**: tick the variables it may change and set their limits (*Set
  limits* makes them a percentage either side). Add goals: low SWR across a band
  (add one per band), gain in a direction or at the peak, front-to-back or
  front-to-rear, each with a target and a weight. *Search the whole range, then
  fine-tune* is best for a first run; *Fine-tune from here* is quick when the
  design is close. The best design shows in every view as it runs. When it
  finishes the best values are applied: Ctrl+Z or *Put the starting values back*
  undoes it, Ctrl+S saves them into the deck. *Skip designs that can't be
  built* (on by default) throws out designs where wires cross, touch, lie on
  top of each other or go below the ground (unless the starting design has
  those already); *Keep within* x, y or z limits the whole antenna's overall
  size along that axis (a feedline shield isn't counted). The view says how
  many designs were skipped and why.
  Tick *at the radio* on an SWR goal to measure it through the Feedline
  view's balun, connectors and cable instead of at the feed point. If a
  variable sets the cable length or the balun ratio (ƒ in the Feedline view),
  tick it as a variable and the optimiser tunes the feed with the antenna.
  That SWR is scored without the cable's loss (loss only hides a mismatch, so
  a longer, lossier cable can't pass for a better match), and the loss counts
  against the design separately. On a cable matching the radio's impedance its
  length then makes no difference; on a different one (75 Ω coax, ladder line
  to a tuner) it transforms the impedance and is worth tuning.
  *Match a measurement* fits the model to a VNA sweep you've imported
  (Compare view): it compares the reflection coefficients at 21 frequencies
  within the sweep (use *Sweep this range* in the Compare view first if they
  don't overlap) and reports the RMS difference (0 = identical; under 0.05 is
  close). Tick the variables that could really differ from what you built:
  element lengths, height, the ground's permittivity and conductivity, a
  loss. The fitted values tell you what the real antenna is like, for example
  how much longer it behaves than you cut it, or what your ground is.
  *Low common-mode current* keeps the current on the outside of the coax
  under a share of the feed current at a frequency. Model the shield first
  (Feedline view, *Model the shield*; the goal shows ⚠ until you do), and
  make the choke a variable (ƒ there) so the optimiser can size it; the
  antenna's own variables can be tuned with it.
- **Compare**: take a snapshot, change the design, and both show on the SWR,
  Smith chart and pattern plots. *Restore* brings a design back.
  Under **Measurements**, *Import a VNA sweep* (or File > Import a VNA sweep,
  or drop the file on the window) reads a Touchstone file (.s1p, or S11 of an
  .s2p) and draws it on the SWR, R and X, and Smith plots within the sweep.
  If you measured at the radio end of a cable, tick *Measured through the
  feedline*: the cable, connectors and balun set in the Feedline view are
  taken off again, so you compare the antenna with the antenna. *Sweep this
  range* sets the model's sweep to the measured frequencies.
- **Matching**: networks that match the antenna at the slider frequency to Z₀
  (L, pi and T with a chosen Q, or a stub), with part values or the nearest
  standard parts, drawn on a Smith chart, and the SWR across the band with the
  network in place.
- **Feedline**: the cable (or your own), its length and any balun or unun:
  the SWR at the radio, the loss, and how much power reaches the antenna.
  The **Feed system** view (next to Feedline) starts from that chain, adds
  a matching network (an L, pi or T network at the antenna, an L network
  at the radio, or a shorted or open stub near the antenna) and tunes it over the sweep, or
  only the bands you tick, for the lowest worst SWR at the radio, with a
  weighting against loss. It can also tune the cable length (between
  limits you give) and the balun ratio. Parts have a Q you set, so sharp,
  lossy networks show their cost. The result is listed with exact values and
  with the nearest E12 or E24 parts, and plotted against what you have now;
  *Use this cable length and balun ratio* copies those back. It works on the
  sweep's feed impedances, so it's instant (no engine runs); chokes and
  common-mode current aren't part of it yet.
  *Connectors* adds a mated pair (plug and socket) at the antenna end and/or
  the radio end: BNC, N or SMA with their published maximum loss (scaled
  with √f; the figure at the slider frequency is shown, with a warning above
  the connector's rated frequency), or *Other* with a loss you type (UHF
  PL-259/SO-239 has no dependable published figure, so measure it or
  estimate). At the antenna you can also give the length of 50 Ω line the
  connectors and any adapter add between the feed point and the cable: at
  UHF and above a few centimetres turn the impedance noticeably.
  *Model the shield* adds the outside of the coax as a wire running down from
  the feed point (to the ground, or a length you set), with a choke of the
  resistance you choose at its top. The view then shows the common-mode
  current: how much of the feed current flows back down the shield. More than
  about 10% is worth a choke.

## Saving and exporting
- Ctrl+S saves, with the slider values written into the deck's `SY` lines.
  A new model saves as a **project** (`.straws`), which keeps everything set
  up around the antenna: the notes (Notes view), the Feedline settings
  (cable, connectors, balun, choke), the optimiser's variables, limits and
  goals, the comparison snapshots (swept again when the project opens) and
  measured VNA sweeps. A project opens exactly as you left it, and the title
  shows "(changed)" when any of that has changed since it was saved.
- A `.nec` deck you open saves as a `.nec` deck, holding the antenna only.
  File > Save as offers both kinds: choose a project to keep the rest too.
- A project file starts with the plain NEC deck, readable in any text
  editor; the rest follows a `'straws project` line, where NEC stops reading.
  File > Export > **NEC deck** writes the antenna alone as a `.nec` file for
  4nec2 or any other NEC program, leaving the open file as it is.
- File > Export: CSV of the sweep, the pattern or the currents (for
  spreadsheets, Octave or MATLAB), Touchstone `.s1p`, DXF of the wires or STL
  of them as solid tubes for 3D printing, a STEP solid model for CAD (see
  below), a **PDF report** (key figures, a
  drawing of the antenna, the pattern cuts, SWR, resistance and reactance
  across the sweep, the wires and the deck; generated meshes, radials and
  curves are summed up in a line each) or the same report as a web page, SVG images of the SWR plot, Smith chart or
  pattern cuts (for documents and slides), or a PNG screenshot of the whole
  window or of any one view on screen.
- File > Export > **Solid model (STEP)** writes the antenna for CAD programs
  (FreeCAD, Fusion, SolidWorks, Onshape and others) in millimetres, exactly
  where it is in the model. Every wire is a solid tube at its real diameter
  (hollow if its material is a tube with a wall), and wires joined to each
  other are one body: a biquad's element and its reflector arrive as two
  separate, named objects. Plates, corner reflectors, dishes, curved
  reflectors and grids can go out as solid sheets instead of wire meshes:
  tick them in the export window and choose the thickness (the mesh's wire
  diameter, or your own). A sheet is centred on its wires, flat parts are
  flat faces, and any wires sticking out past the mesh's last whole cell stay
  tubes. Only those meshes can be sheets; other wires are always tubes.
- File > Export > **Build sheet**: a page to take to the workbench (print it
  to PDF): a cut list (identical pieces grouped, lengths in your length unit,
  curves measured along the curve), the stock to buy (total length of each
  material and size), where parallel elements sit along a boom (a split
  driven element's halves share a row; centre to centre, with the gaps), and
  every piece. Lengths are the conductors as modelled: add overlaps at tube
  joints, clamps, insulators and the feed hardware yourself.

## Limits
NEC-2 models thin wires and simple surface patches. It can't model dielectrics
or solid structures (patch antennas, horns), and wires should be much thinner
than their segments are long: the Checks view says when a model goes past what
NEC handles well.
