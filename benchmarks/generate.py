"""Writes the benchmark decks in this folder.

Run from the repository root:  python benchmarks/generate.py
The decks are committed, so this only needs re-running if the set changes.
"""
from pathlib import Path

HERE = Path(__file__).parent
C = 299.792458


def write(name, comments, body):
    lines = [f"CM {c}" for c in comments] + ["CE"] + body + ["EN"]
    (HERE / f"{name}.nec").write_text("\n".join(lines) + "\n", newline="\n")


def dipole(f, h=None):
    half = 0.475 * C / f / 2
    z = h if h is not None else 0
    return [f"GW 1 21 0 {-half:.6f} {z} 0 {half:.6f} {z} 0.001"]


def yagi(f, h=0.0):
    # 7 elements, 45 segments each: 315 segments.
    lam = C / f
    elems = [(-0.20, 0.500), (0.0, 0.473), (0.10, 0.440), (0.25, 0.435),
             (0.42, 0.430), (0.60, 0.426), (0.80, 0.420)]
    out = []
    for tag, (x, length) in enumerate(elems, start=1):
        half = length * lam / 2
        out.append(f"GW {tag} 45 {x * lam:.6f} {-half:.6f} {h} {x * lam:.6f} {half:.6f} {h} 0.006")
    return out


def grid(f, n, h=None):
    """Square wire-grid reflector (n x n nodes, one segment per link) with a
    dipole in front of it: 2n(n-1) + 21 segments."""
    lam = C / f
    d = lam / 10
    x0 = -0.2 * lam
    z0 = h if h is not None else 0
    off = (n - 1) * d / 2
    out = []
    tag = 1
    for i in range(n):
        for j in range(n - 1):
            y1, y2 = -off + j * d, -off + (j + 1) * d
            z = z0 - off + i * d
            out.append(f"GW {tag} 1 {x0:.5f} {y1:.5f} {z:.5f} {x0:.5f} {y2:.5f} {z:.5f} 0.002")
            tag += 1
            y = -off + i * d
            za, zb = z0 - off + j * d, z0 - off + (j + 1) * d
            out.append(f"GW {tag} 1 {x0:.5f} {y:.5f} {za:.5f} {x0:.5f} {y:.5f} {zb:.5f} 0.002")
            tag += 1
    half = 0.47 * lam / 2
    out.append(f"GW {tag} 21 0 {-half:.5f} {z0} 0 {half:.5f} {z0} 0.005")
    return out, tag


SOMMERFELD = "GN 2 0 0 0 13 0.005"

write("01_small_dipole_sweep",
      ["Benchmark: 21-segment dipole, 101-point sweep, free space."],
      dipole(14.2) + ["GE 0", "EX 0 1 11 0 1 0", "FR 0 101 0 0 13.2 0.02", "XQ"])

write("02_small_dipole_sweep_ground",
      ["Benchmark: 21-segment dipole 10 m up, 101-point sweep,",
       "Sommerfeld-Norton ground."],
      dipole(14.2, 10) + ["GE 1", SOMMERFELD, "EX 0 1 11 0 1 0",
                          "FR 0 101 0 0 13.2 0.02", "XQ"])

write("03_medium_yagi_sweep",
      ["Benchmark: 7-element Yagi, 315 segments, 51-point sweep, free space."],
      yagi(28.5) + ["GE 0", "EX 0 2 23 0 1 0", "FR 0 51 0 0 28.0 0.02", "XQ"])

write("04_medium_yagi_sweep_ground",
      ["Benchmark: 7-element Yagi 15 m up, 315 segments, 51-point sweep,",
       "Sommerfeld-Norton ground."],
      yagi(28.5, 15) + ["GE 1", SOMMERFELD, "EX 0 2 23 0 1 0",
                        "FR 0 51 0 0 28.0 0.02", "XQ"])

write("05_medium_yagi_pattern",
      ["Benchmark: 7-element Yagi, 315 segments, one frequency,",
       "full-sphere pattern at 1 degree (65,341 points)."],
      yagi(28.5) + ["GE 0", "EX 0 2 23 0 1 0", "FR 0 1 0 0 28.5 0",
                    "RP 0 181 361 1000 0 0 1 1"])

g, feed = grid(14.2, 28)
write("06_large_grid",
      [f"Benchmark: wire-grid reflector and dipole, {2 * 28 * 27 + 21} segments,",
       "3 frequencies, free space."],
      g + ["GE 0", f"EX 0 {feed} 11 0 1 0", "FR 0 3 0 0 14.0 0.2", "XQ"])

g, feed = grid(14.2, 28, h=40)
write("07_large_grid_ground",
      [f"Benchmark: wire-grid reflector and dipole 40 m up, {2 * 28 * 27 + 21} segments,",
       "one frequency, Sommerfeld-Norton ground."],
      g + ["GE 1", SOMMERFELD, f"EX 0 {feed} 11 0 1 0", "FR 0 1 0 0 14.2 0", "XQ"])

g, feed = grid(14.2, 39)
write("08_xlarge_grid",
      [f"Benchmark: wire-grid reflector and dipole, {2 * 39 * 38 + 21} segments,",
       "one frequency, free space."],
      g + ["GE 0", f"EX 0 {feed} 11 0 1 0", "FR 0 1 0 0 14.2 0", "XQ"])
print("benchmark decks written")
