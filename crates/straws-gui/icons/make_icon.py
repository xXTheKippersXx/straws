"""The STRAWS icon: a pink crazy straw (a straight straw with round loops
beside it, as real ones are) on a muted blue disc with a thick light blue
rim. Flat and cartoony; transparent outside the circle; the straw stays
inside the disc."""
import math
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "straws-icon.svg"

RIM = "#a9dcf2"
DISC = "#5476a0"
DISC_SHADE = "#4a6992"
INK = "#2a2340"
PINK = "#ff6fae"
END = "#b8336f"

W = 28.0        # straw width
OUTLINE = 12.0  # added to the width for the outline (half each side)
TILT = math.radians(7)  # the whole straw leans a little to the right

pts = []


def line_to(x, y, step=2.0):
    x0, y0 = pts[-1]
    n = max(1, int(math.hypot(x - x0, y - y0) / step))
    for k in range(1, n + 1):
        pts.append((x0 + (x - x0) * k / n, y0 + (y - y0) * k / n))


def loop(radius, side):
    """A full round loop from the current point (going straight up), out to
    `side` (-1 left, +1 right), back to the same point."""
    px, py = pts[-1]
    cx = px + side * radius
    n = int(2 * math.pi * radius / 2.0)
    for k in range(1, n + 1):
        t = 2 * math.pi * k / n
        pts.append((cx - side * radius * math.cos(t), py - radius * math.sin(t)))


def bend(radius, degrees):
    """Turns right by `degrees` from going straight up, round a curve."""
    px, py = pts[-1]
    cx = px + radius
    n = max(2, int(radius * math.radians(degrees) / 2.0))
    for k in range(1, n + 1):
        t = math.radians(degrees) * k / n
        pts.append((cx - radius * math.cos(t), py - radius * math.sin(t)))


# Up from below the disc, a big loop to the left, a smaller one to the
# right, then a bend to the right and a straight top.
pts.append((250.0, 520.0))
line_to(250.0, 318.0)
loop(66.0, -1)
line_to(250.0, 222.0)
loop(50.0, +1)
line_to(250.0, 168.0)
bend(34.0, 42.0)
ang = math.radians(-90 + 42)
ex, ey = pts[-1]
for k in range(1, 30):
    pts.append((ex + k * 2.0 * math.cos(ang), ey + k * 2.0 * math.sin(ang)))

# Lean the whole straw a little, about the disc's centre.
c, s = math.cos(TILT), math.sin(TILT)
pts = [(256 + (x - 256) * c - (y - 256) * s, 256 + (x - 256) * s + (y - 256) * c) for x, y in pts]

along = [0.0]
for (x1, y1), (x2, y2) in zip(pts, pts[1:]):
    along.append(along[-1] + math.hypot(x2 - x1, y2 - y1))

# Stretches passing over an earlier part are drawn again on top.
over = []
for i, (x, y) in enumerate(pts):
    over.append(any(
        along[i] - along[j] > 2.5 * W and math.hypot(x - pts[j][0], y - pts[j][1]) < W + OUTLINE
        for j in range(i)))
runs, i = [], 0
while i < len(pts):
    if over[i]:
        j = i
        while j + 1 < len(pts) and over[j + 1]:
            j += 1
        runs.append((i, j))
        i = j + 1
    else:
        i += 1


def path(i, j):
    i, j = max(i, 0), min(j, len(pts) - 1)
    return "M " + " L ".join(f"{x:.1f} {y:.1f}" for x, y in pts[i:j + 1])


def layers(i, j, extend):
    lo, hi = max(i - extend, 0), min(j + extend, len(pts) - 1)
    return (f'<path d="{path(i, j)}" stroke="{INK}" stroke-width="{W + OUTLINE}"/>\n'
            f'<path d="{path(lo, hi)}" stroke="{PINK}" stroke-width="{W}"/>\n')


body = layers(0, len(pts) - 1, 0)
for i, j in runs:
    body += layers(i - 4, j + 4, 6)

tip, before = pts[-1], pts[-3]
rot = math.degrees(math.atan2(tip[1] - before[1], tip[0] - before[0])) + 90
reach = max(math.hypot(x - 256, y - 256) for x, y in pts[:]) if False else None
tipd = math.hypot(tip[0] - 256, tip[1] - 256) + (W + OUTLINE) / 2
far = max(math.hypot(x - 256, y - 256) for x, y in pts if y < 400) + (W + OUTLINE) / 2

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
  <defs>
    <clipPath id="disc"><circle cx="256" cy="256" r="200"/></clipPath>
  </defs>
  <circle cx="256" cy="256" r="244" fill="{INK}"/>
  <circle cx="256" cy="256" r="236" fill="{RIM}"/>
  <circle cx="256" cy="256" r="206" fill="{INK}"/>
  <circle cx="256" cy="256" r="200" fill="{DISC_SHADE}"/>
  <g clip-path="url(#disc)">
    <circle cx="232" cy="232" r="200" fill="{DISC}"/>
    <g fill="none" stroke-linecap="butt" stroke-linejoin="round">
{body}    </g>
    <ellipse cx="{tip[0]:.1f}" cy="{tip[1]:.1f}" rx="{(W + OUTLINE) / 2:.1f}" ry="{W * 0.3:.1f}" transform="rotate({rot:.1f} {tip[0]:.1f} {tip[1]:.1f})" fill="{INK}"/>
    <ellipse cx="{tip[0]:.1f}" cy="{tip[1]:.1f}" rx="{W / 2 - 3:.1f}" ry="{W * 0.15:.1f}" transform="rotate({rot:.1f} {tip[0]:.1f} {tip[1]:.1f})" fill="{END}"/>
  </g>
</svg>
'''
open(OUT, "w").write(svg)
print(f"tip reaches {tipd:.0f}, farthest (above the stem's foot) {far:.0f}, of 200; crossings {runs}")

