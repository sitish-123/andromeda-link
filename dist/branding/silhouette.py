# Writes the one-colour Andromeda mark (the A and its ring, with gaps where they cross) as SVG path data.
# Run: uv run --with shapely python dist/branding/silhouette.py
import math
from pathlib import Path

from shapely import affinity
from shapely.geometry import LineString, Point, Polygon
from shapely.ops import unary_union

HERE = Path(__file__).parent
A = Polygon([(132, 410), (246, 102), (270, 102), (392, 410), (340, 410), (258, 178), (160, 410)])
CENTER, RX, RY, TILT = (256, 300), 200, 46, -34
RING, GAP, NODE = 30, 14, 26  # widths in the 512 viewBox, heavier than the colour mark so 16 px stays legible


def arc(start, end):
    points = [(RX * math.cos(t), RY * math.sin(t)) for t in (start + (end - start) * i / 200 for i in range(201))]
    line = affinity.rotate(LineString(points), TILT, origin=(0, 0))
    return affinity.translate(line, *CENTER)


def node():
    x, y = -120, 37
    angle = math.radians(TILT)
    return Point(CENTER[0] + x * math.cos(angle) - y * math.sin(angle), CENTER[1] + x * math.sin(angle) + y * math.cos(angle))


back = arc(math.pi - 0.05, 2 * math.pi + 0.05).buffer(RING / 2, cap_style="flat")
front = arc(0, math.pi).buffer(RING / 2, cap_style="flat")
dot = node().buffer(NODE)
shape = unary_union([
    A.difference(front.buffer(GAP)).difference(dot.buffer(GAP)),
    back.difference(A.buffer(GAP)),
    front.difference(dot.buffer(GAP)),
    dot,
])


def path(geometry):
    rings = []
    for polygon in getattr(geometry, "geoms", [geometry]):
        for ring in [polygon.exterior, *polygon.interiors]:
            coords = list(ring.coords)
            rings.append("M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in coords[:-1]) + "Z")
    return "".join(rings)


data = path(shape.simplify(0.4))
(HERE / "andromeda-mark-path.txt").write_text(data + "\n")
(HERE / "andromeda-symbolic.svg").write_text(
    f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="{data}" fill="#2E3436"/></svg>\n'
)
