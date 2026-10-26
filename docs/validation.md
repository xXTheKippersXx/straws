# Validation

A feature isn't done until its numbers match the reference. This page explains how results are
checked, and records where things stand.

## How results are compared

The reference engine is 4nec2's `nec2dxs` (a double-precision NEC-2D build by Arie Voors). Each
check runs **the same expanded deck** through nec2c and through `nec2dxs`, reads both outputs
with the same parser, and compares them (`crates/straws-core/src/validate.rs`):

| Quantity | Allowed difference | Why this much |
|---|---|---|
| Feed impedance | 0.02% of \|Z\| + 0.001 Ω | nec2c prints 5 significant figures, NEC-2D prints 6 |
| Segment currents | 0.02% of the largest current | same |
| Gain (each pattern point within 40 dB of the peak) | 0.02 dB at the peak, plus 0.003 dB per dB below it | both engines print gains to 0.01 dB; weaker directions are more sensitive |
| Average gain | 0.1% | |
| Efficiency | 0.02 percentage points | |

Deep nulls (more than 40 dB below the peak) aren't compared: a null's depth swings with tiny
numerical differences and isn't a meaningful check.

Decks are written for both engines no wider than 80 columns, because the Fortran engine reads only
80 columns of each line (a longer line silently loses its last fields).

## The two test sets

1. **Public regression decks**: `tests/decks/` holds 20 decks written for this project, covering
   free space, perfect ground, reflection-coefficient and Sommerfeld-Norton grounds, low wires
   over real ground, loads (`LD 0`–`LD 5`, 4nec2's `LD 6` traps and `LD 7` insulated wire),
   transmission lines, networks, NEC's buried radial screen, 4nec2 current sources (`EX 6`), arcs, helices, surface patches,
   symmetry, `GM`/`GR`/`GX` transforms, tapered wires, `EK`, percentage feed positions, and
   linear and logarithmic sweeps. Their reference outputs from `nec2dxs` are in
   `tests/references/`. `cargo test` checks all of them on Windows, Linux and macOS (CI), so CI
   doesn't need 4nec2.
2. **4nec2's bundled models** (744 decks in `C:\4nec2\models`), checked locally because they
   aren't ours to publish:

   ```
   straws-cli validate C:\4nec2\models --engine-dir C:\4nec2\exe --pattern
   ```

   `--pattern` adds a 10° whole-sphere pattern to decks that don't ask for one. A full run takes
   about 3½ minutes on the dev machine.

To regenerate the public references after adding a deck (Windows with 4nec2 installed):

```
straws-cli reference tests/decks --out tests/references
```

## Results (2026-10-18)

Public regression decks: **21 of 21 match**.

4nec2's 744 models (743 with the current version of the deck reader):

| Outcome | Decks |
|---|---:|
| Match within the tolerances above | 718 |
| Not compared: rejected by both engines (errors in the deck itself), not comparable (NGF files `GF`/`WG`, a NEC-4 card, undefined symbols, no `GE` card), or skipped (over 1,500 segments, or over 60 s in one engine) | 23 |
| **Differ** | **2** (see below) |

(Re-run 2026-10-19 in the testing protocol: 712 match, up from 710; reading
old-Mac CR-only line endings is the likely gain. Re-run 2026-10-20: 714 match,
after a reader fix for units written inside tab-separated fields, `-51 ft`.
That bug made G5RV, 2lsloper and Vehicle read as different antennas, which
this comparison can't see on its own, because both engines run our reading of
the deck; `straws-cli check-expand` against 4nec2's own expansions is what
checks the reader.) 2026-10-25: 718 match: the four patch-strip decks
(`coffee`, `coffee1`, `VE4MAfeed`, `W2IMUdual`) now run, after a fix in nec2c
itself (patch strips, see `crates/straws-core/nec2c/README.straws.md`); all 15
of 4nec2's patch models match. New public deck `patch_strips_box.nec`.

Typical agreement is about 1 part in 100,000 for impedance (the last digit nec2c prints) and
0.00–0.01 dB for gain.

### Remaining differences

- **`ELEVRAD1`**: radials 1.8 cm (0.0001 λ) above Sommerfeld ground; the deck's own comment says
  results there aren't accurate. 4% apart. Possibly a last Sommerfeld difference worth chasing.
- **`weird monopole`**: run at 1 kHz on a 10 cm structure (segments a ten-millionth of a
  wavelength), far outside what NEC can solve; both engines give noise. The live geometry checks
  (phase 3) should warn about this.

## Radials on or under the ground (estimated)

NEC-2 can't model wires in or on lossy ground, so STRAWS estimates radials
there (`analysis::buried`, Wait and Pope's compensation method over the field of
a perfect-ground run). Checked against Brown, Lewis and Epstein, "Ground systems
as a factor in antenna efficiency", Proc. IRE 25(6), 1937, Figs. 36-37, as
digitised in github.com/stevenmburns/momwire issue 838 (a second-hand reading of
the graphs, about ±1 ohm): 3 MHz, a 21.4 m mast 2.5 in across, No. 8 copper
radials 15 cm deep, soil 2 mS/m (permittivity not given; 5 used, 5-30 moves the
results by a couple of ohms). Test `buried_radials_match_brown_lewis_and_epstein`.

| Radials | Length | Measured base R | STRAWS |
|---:|---:|---:|---:|
| 2 | 0.411 λ | over 50 Ω | 55 Ω |
| 15 | 0.411 λ | 34 Ω | 35.0 Ω |
| 30 | 0.411 λ | 30 Ω | 30.7 Ω |
| 60 | 0.411 λ | 26 Ω | 27.6 Ω |
| 113 | 0.411 λ | 24.3 Ω | 25.8 Ω |
| 15 | 0.137 λ | 33 Ω | 36.2 Ω |
| 30 | 0.137 λ | 31 Ω | 33.3 Ω |
| 60 | 0.137 λ | 31 Ω | 32.0 Ω |
| 113 | 0.137 λ | 31 Ω | 31.6 Ω |

It also gives the known behaviour for a 7 MHz quarter-wave vertical over
average ground: with no radials 132 Ω (NEC-2's own bare-ground answer: 183 Ω,
itself shaky for a wire touching lossy ground), 4 radials about 57 Ω whatever
their length, 16 quarter-wave radials about 9 Ω of ground loss, 120 within about
1 Ω of perfect ground. Rougher with few radials (the screen formula assumes an
even "mat").

## Bugs found in nec2c and fixed here

Validating against 4nec2's engine found real bugs in nec2c 1.3.1, all fixed in
`crates/straws-core/nec2c` and listed in `README.straws.md`:

1. **Wire conductivity losses (`LD 5`) were wrong for thin wires.** `zint()` lacked the `return`
   statements of the Fortran, so every wire used the thick-wire skin-effect formula. Fixing this
   brought about 70 of 4nec2's models into agreement.
2. **Sommerfeld-Norton ground: incomplete contour integral** in `evlua()` for some combinations of
   wire height and distance (up to 5% impedance error on 80 m beams). Fix found by xnec2c.
3. **Sommerfeld-Norton ground: misplaced bracket** in `gshank()`'s convergence test. Fix found by
   xnec2c.
4. **Crash on a second number on the `GE` card** (nec2c tried to write a plot file that wasn't
   open). The deck reader now drops that field, which isn't part of NEC-2.
5. **Buffer overflow** reading comment lines longer than 80 characters.

Also found: a wire of zero length makes nec2c run forever. The deck reader rejects such wires
with a clear message before they reach the engine.
