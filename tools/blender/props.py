"""Environment, structure and gameplay props."""

import math

from lib import ball, box, collider, cyl, export, join, mat, prism, reset, rng, torus

# One shared palette keeps the whole library visually coherent.
WOOD, WOOD_DARK, WOOD_LIGHT = "#8a5a3c", "#5e3b28", "#b98556"
LEAF, LEAF_DARK, LEAF_LIGHT = "#4fa64a", "#2f7d46", "#84c85a"
PINE, PINE_DARK = "#2e7d5b", "#205f4a"
STONE, STONE_DARK, STONE_LIGHT = "#8d939e", "#666c78", "#b4bac4"
GOLD, RED, BLUE, WHITE = "#ffc531", "#e5484d", "#3d8bfd", "#f4f1ea"
ROOF, PLASTER, METAL = "#c8553d", "#f0e2c6", "#9aa3ad"


def tree_pine():
    reset()
    parts = [cyl(0.22, 1.3, material=mat("Bark", WOOD_DARK), verts=8, radius_top=0.14)]
    for i, (r, z, h) in enumerate([(1.25, 0.9, 1.5), (1.0, 1.75, 1.35), (0.72, 2.55, 1.2), (0.42, 3.3, 1.0)]):
        parts.append(cyl(r, h, (0, 0, z), mat("Pine" if i % 2 == 0 else "PineDark", PINE if i % 2 == 0 else PINE_DARK), verts=9, radius_top=0.0, smooth=False))
    export("tree_pine", [join(parts, "tree_pine")], "Pine Tree", "Nature", [collider("cylinder", (0, 0, 1.2), (0.6, 0.6, 2.4))], ["tree"])


def tree_oak():
    reset()
    r = rng(4)
    parts = [cyl(0.3, 1.9, material=mat("Bark", WOOD), verts=8, radius_top=0.2)]
    parts.append(cyl(0.12, 1.0, (0.15, 0, 1.3), mat("Bark", WOOD), verts=6, radius_top=0.07, rot=(0, math.radians(50), 0)))
    blobs = [((0, 0, 2.9), 1.25), ((0.85, 0.2, 2.5), 0.85), ((-0.8, -0.3, 2.55), 0.9), ((0.1, 0.8, 2.6), 0.8), ((0.0, -0.75, 2.5), 0.75), ((0.2, 0.1, 3.7), 0.8)]
    for i, (loc, radius) in enumerate(blobs):
        m = [mat("Leaf", LEAF), mat("LeafDark", LEAF_DARK), mat("LeafLight", LEAF_LIGHT)][i % 3]
        parts.append(ball(radius, loc, m, subdiv=2, lumpy=0.22, seed=r.random() * 9, scale=(1, 1, 0.85)))
    export("tree_oak", [join(parts, "tree_oak")], "Oak Tree", "Nature", [collider("cylinder", (0, 0, 1.3), (0.7, 0.7, 2.6))], ["tree"])


def bush():
    reset()
    parts = [ball(r, loc, mat(n, c), lumpy=0.25, seed=i + 2, scale=(1, 1, 0.8)) for i, (loc, r, n, c) in enumerate([
        ((0, 0, 0.42), 0.6, "Leaf", LEAF), ((0.5, 0.1, 0.3), 0.42, "LeafDark", LEAF_DARK), ((-0.45, -0.1, 0.32), 0.45, "LeafLight", LEAF_LIGHT)])]
    for x, y, z in [(0.2, -0.45, 0.55), (-0.3, -0.35, 0.6), (0.55, -0.2, 0.45)]:
        parts.append(ball(0.07, (x, y, z), mat("Berry", RED, rough=0.4), subdiv=1, smooth=True))
    export("bush", [join(parts, "bush")], "Berry Bush", "Nature", [collider("sphere", (0, 0, 0.4), (1.0, 1.0, 1.0))], ["plant"])


def rocks():
    for name, display, seed, radius, scale in [("rock_large", "Large Rock", 3, 1.1, (1.2, 1.0, 0.8)), ("rock_small", "Small Rock", 8, 0.45, (1.1, 0.9, 0.7)), ("rock_tall", "Tall Rock", 12, 0.8, (0.8, 0.75, 1.7))]:
        reset()
        obj = ball(radius, (0, 0, radius * scale[2] * 0.55), mat("Stone", STONE, rough=0.9), subdiv=2, lumpy=0.45, seed=seed, scale=scale, flat_bottom=-0.6)
        obj = join([obj], name)
        size = obj.dimensions
        export(name, [obj], display, "Nature", [collider("box", (0, 0, size.z / 2), (size.x * 0.8, size.y * 0.8, size.z * 0.9))], ["rock"])


def flower_and_grass():
    reset()
    parts = [cyl(0.025, 0.42, material=mat("Stem", LEAF_DARK), verts=5)]
    for i in range(6):
        a = i * math.tau / 6
        parts.append(ball(0.09, (math.cos(a) * 0.12, math.sin(a) * 0.12, 0.44), mat("Petal", "#ff8fab", rough=0.6), subdiv=1, scale=(1, 1, 0.45), smooth=True))
    parts.append(ball(0.08, (0, 0, 0.46), mat("Pollen", GOLD), subdiv=1, smooth=True))
    parts.append(prism([(0, 0), (0.16, 0.05), (0.05, 0.16)], 0.02, (0.05, 0, 0.12), mat("Stem", LEAF_DARK)))
    export("flower", [join(parts, "flower")], "Flower", "Nature", [], ["plant", "decor"])

    reset()
    r = rng(2)
    parts = []
    for i in range(9):
        a, d, h = r.random() * math.tau, r.random() * 0.22, 0.25 + r.random() * 0.3
        parts.append(cyl(0.035, h, (math.cos(a) * d, math.sin(a) * d, 0), mat("Grass", LEAF_LIGHT if i % 2 else LEAF), verts=4, radius_top=0.0, rot=(r.uniform(-0.3, 0.3), r.uniform(-0.3, 0.3), 0), smooth=False))
    export("grass_tuft", [join(parts, "grass_tuft")], "Grass Tuft", "Nature", [], ["plant", "decor"])


def mushroom():
    reset()
    parts = [cyl(0.1, 0.32, material=mat("MushroomStem", WHITE), verts=8, radius_top=0.07)]
    parts.append(ball(0.3, (0, 0, 0.3), mat("MushroomCap", RED, rough=0.5), subdiv=2, scale=(1, 1, 0.65), flat_bottom=0.0, smooth=True))
    for x, y, z in [(0.12, -0.14, 0.44), (-0.15, 0.05, 0.45), (0.02, 0.17, 0.44), (0.0, -0.02, 0.5)]:
        parts.append(ball(0.045, (x, y, z), mat("MushroomStem", WHITE), subdiv=1, scale=(1, 1, 0.5), smooth=True))
    export("mushroom", [join(parts, "mushroom")], "Mushroom", "Nature", [], ["plant", "pickup"])


def log_and_stump():
    reset()
    parts = [cyl(0.32, 2.0, (-1.0, 0, 0.32), mat("Bark", WOOD_DARK), verts=10, rot=(0, math.radians(90), 0))]
    parts += [cyl(0.27, 0.02, (x, 0, 0.32), mat("WoodCut", WOOD_LIGHT), verts=10, rot=(0, math.radians(90), 0)) for x in (-1.01, 0.99)]
    export("log", [join(parts, "log")], "Log", "Nature", [collider("box", (0, 0, 0.32), (2.0, 0.6, 0.64))], ["wood", "resource"])
    reset()
    parts = [cyl(0.5, 0.5, material=mat("Bark", WOOD_DARK), verts=10, radius_top=0.42), cyl(0.38, 0.02, (0, 0, 0.5), mat("WoodCut", WOOD_LIGHT), verts=10)]
    export("stump", [join(parts, "stump")], "Tree Stump", "Nature", [collider("cylinder", (0, 0, 0.25), (0.9, 0.9, 0.5))], ["wood"])


def cloud():
    reset()
    parts = [ball(r, loc, mat("Cloud", "#ffffff", rough=1.0), subdiv=2, scale=(1, 1, 0.7), smooth=True) for loc, r in [((0, 0, 0), 1.4), ((1.5, 0.1, -0.15), 1.05), ((-1.5, -0.1, -0.2), 1.0), ((0.6, 0.3, 0.55), 0.95), ((-2.6, 0, -0.35), 0.65), ((2.6, 0, -0.3), 0.7)]]
    export("cloud", [join(parts, "cloud")], "Cloud", "Nature", [], ["sky", "decor"])


def crate_and_barrel():
    reset()
    parts = [box((0.92, 0.92, 0.92), (0, 0, 0.5), mat("Wood", WOOD_LIGHT))]
    for a, b in [(-0.45, -0.45), (-0.45, 0.45), (0.45, -0.45), (0.45, 0.45)]:
        parts.append(box((0.12, 0.12, 1.0), (a, b, 0.5), mat("WoodTrim", WOOD), bevel=0.015))
        parts.append(box((1.0, 0.12, 0.12), (0, a, 0.5 + b), mat("WoodTrim", WOOD), bevel=0.015))
        parts.append(box((0.12, 1.0, 0.12), (a, 0, 0.5 + b), mat("WoodTrim", WOOD), bevel=0.015))
    export("crate", [join(parts, "crate")], "Crate", "Props", [collider("box", (0, 0, 0.5), (1, 1, 1))], ["container"])

    reset()
    parts = []
    for z, r0, r1 in [(0, 0.38, 0.46), (0.5, 0.46, 0.38)]:
        parts.append(cyl(r0, 0.5, (0, 0, z), mat("Wood", WOOD), verts=14, radius_top=r1))
    for z in (0.14, 0.5, 0.86):
        parts.append(cyl(0.47 if z == 0.5 else 0.43, 0.06, (0, 0, z - 0.03), mat("Iron", STONE_DARK, rough=0.4, metal=0.8), verts=14))
    export("barrel", [join(parts, "barrel")], "Barrel", "Props", [collider("cylinder", (0, 0, 0.5), (0.9, 0.9, 1.0))], ["container"])


def fence_and_sign():
    reset()
    parts = [box((0.14, 0.14, 1.1), (x, 0, 0.55), mat("Wood", WOOD), bevel=0.02) for x in (-0.95, 0.95)]
    parts += [box((2.0, 0.07, 0.14), (0, 0, z), mat("WoodLight", WOOD_LIGHT), bevel=0.015) for z in (0.4, 0.82)]
    export("fence", [join(parts, "fence")], "Fence", "Structures", [collider("box", (0, 0, 0.55), (2.0, 0.2, 1.1))], ["barrier"])
    reset()
    parts = [box((0.12, 0.12, 1.5), (0, 0, 0.75), mat("Wood", WOOD), bevel=0.02), box((1.1, 0.08, 0.6), (0, -0.08, 1.25), mat("WoodLight", WOOD_LIGHT), bevel=0.03)]
    export("sign", [join(parts, "sign")], "Signpost", "Props", [collider("box", (0, 0, 0.75), (0.2, 0.2, 1.5))], ["decor"])


def lamp_post():
    reset()
    iron = mat("Iron", "#3b4252", rough=0.45, metal=0.8)
    parts = [cyl(0.2, 0.25, material=iron, verts=10, radius_top=0.12), cyl(0.07, 2.9, (0, 0, 0.2), iron, verts=8), cyl(0.2, 0.08, (0, 0, 3.1), iron, verts=8)]
    parts.append(cyl(0.16, 0.42, (0, 0, 3.18), mat("LampGlass", "#ffe9a8", rough=0.3, emit=6.0), verts=8, radius_top=0.2))
    parts.append(cyl(0.26, 0.2, (0, 0, 3.6), iron, verts=8, radius_top=0.03))
    export("lamp_post", [join(parts, "lamp_post")], "Lamp Post", "Props", [collider("cylinder", (0, 0, 1.6), (0.3, 0.3, 3.2))], ["light"])


def campfire():
    reset()
    parts = []
    for i in range(8):
        a = i * math.tau / 8
        parts.append(ball(0.17, (math.cos(a) * 0.55, math.sin(a) * 0.55, 0.1), mat("Stone", STONE, rough=0.9), subdiv=1, lumpy=0.3, seed=i, scale=(1, 1, 0.75)))
    for i in range(4):
        a = i * math.tau / 4 + 0.4
        parts.append(cyl(0.07, 0.75, (math.cos(a) * 0.3, math.sin(a) * 0.3, 0.05), mat("Bark", WOOD_DARK), verts=6, rot=(math.sin(a) * 0.9, -math.cos(a) * 0.9, 0)))
    for r, h, c, e in [(0.26, 0.6, "#ff7b1c", 5.0), (0.16, 0.8, "#ffc531", 8.0)]:
        parts.append(cyl(r, h, (0, 0, 0.12), mat("Flame" + c, c, emit=e), verts=7, radius_top=0.0, smooth=False))
    export("campfire", [join(parts, "campfire")], "Campfire", "Props", [collider("cylinder", (0, 0, 0.15), (1.3, 1.3, 0.3))], ["light", "survival"])


def chest():
    reset()
    gold = mat("Gold", GOLD, rough=0.3, metal=0.9)
    parts = [box((1.2, 0.8, 0.55), (0, 0, 0.275), mat("Wood", WOOD), bevel=0.03)]
    parts.append(cyl(0.4, 1.2, (-0.6, 0, 0.55), mat("WoodDark", WOOD_DARK), verts=12, rot=(0, math.radians(90), 0)))
    for x in (-0.5, 0, 0.5):
        parts.append(box((0.1, 0.84, 0.59), (x, 0, 0.285), gold, bevel=0.01))
    parts.append(box((0.18, 0.06, 0.22), (0, -0.42, 0.52), gold, bevel=0.02))
    export("chest", [join(parts, "chest")], "Treasure Chest", "Props", [collider("box", (0, 0, 0.45), (1.2, 0.8, 0.9))], ["container", "treasure"])


def house():
    reset()
    parts = [box((4.4, 3.6, 0.3), (0, 0, 0.15), mat("Stone", STONE), bevel=0.04), box((4.0, 3.2, 2.4), (0, 0, 1.5), mat("Plaster", PLASTER, rough=0.9))]
    parts.append(prism([(-2.4, 0), (2.4, 0), (0, 1.9)], 3.7, (0, 0, 2.7), mat("Roof", ROOF, rough=0.8)))
    parts.append(prism([(-2.0, 0), (2.0, 0), (0, 1.58)], 3.22, (0, 0, 2.7), mat("Plaster", PLASTER, rough=0.9)))
    for x in (-2.0, 2.0):
        for y in (-1.6, 1.6):
            parts.append(box((0.22, 0.22, 2.4), (x, y, 1.5), mat("Wood", WOOD_DARK), bevel=0.02))
    parts.append(box((0.95, 0.1, 1.75), (0, -1.62, 1.17), mat("Door", WOOD), bevel=0.03))
    parts.append(ball(0.06, (0.3, -1.7, 1.15), mat("Gold", GOLD, rough=0.3, metal=0.9), subdiv=1, smooth=True))
    for x in (-1.25, 1.25):
        parts.append(box((0.85, 0.08, 0.85), (x, -1.61, 1.75), mat("Window", "#9bd4ff", rough=0.1, emit=0.6), bevel=0.02))
        parts.append(box((0.95, 0.1, 0.1), (x, -1.63, 1.75), mat("Wood", WOOD_DARK)))
        parts.append(box((0.1, 0.1, 0.95), (x, -1.63, 1.75), mat("Wood", WOOD_DARK)))
    parts.append(box((0.6, 0.6, 1.5), (1.2, 0.6, 3.6), mat("Brick", "#a8573f", rough=0.9), bevel=0.03))
    export("house", [join(parts, "house")], "Cottage", "Structures", [collider("box", (0, 0, 1.35), (4.4, 3.6, 2.7)), collider("box", (0, 0, 3.3), (2.6, 3.7, 1.3))], ["building"])


def tower():
    reset()
    parts = [cyl(1.5, 5.0, material=mat("Stone", STONE_LIGHT, rough=0.9), verts=12, radius_top=1.3, smooth=False)]
    parts.append(cyl(1.6, 0.5, (0, 0, 5.0), mat("StoneDark", STONE_DARK, rough=0.9), verts=12, smooth=False))
    for i in range(12):
        if i % 2 == 0:
            a = i * math.tau / 12 + math.tau / 24
            parts.append(box((0.5, 0.35, 0.5), (math.cos(a) * 1.42, math.sin(a) * 1.42, 5.75), mat("StoneDark", STONE_DARK, rough=0.9), rot=(0, 0, a + math.pi / 2)))
    parts.append(box((0.9, 0.2, 1.6), (0, -1.45, 0.8), mat("Door", WOOD), bevel=0.03))
    for z in (2.4, 3.9):
        parts.append(box((0.3, 0.2, 0.7), (0, -1.38, z), mat("WindowDark", "#1d2433", rough=0.3)))
    export("tower", [join(parts, "tower")], "Stone Tower", "Structures", [collider("cylinder", (0, 0, 3.0), (3.0, 3.0, 6.0))], ["building"])


def bridge():
    reset()
    parts = [box((1.9, 0.5, 0.1), (0, y, 0.3), mat("WoodLight", WOOD_LIGHT), bevel=0.015) for y in [i * 0.56 - 1.96 for i in range(8)]]
    for x in (-0.9, 0.9):
        parts.append(box((0.14, 4.4, 0.16), (x, 0, 0.2), mat("Wood", WOOD_DARK), bevel=0.02))
        parts.append(box((0.08, 4.4, 0.08), (x, 0, 1.05), mat("Wood", WOOD), bevel=0.01))
        parts += [box((0.1, 0.1, 0.85), (x, y, 0.65), mat("Wood", WOOD), bevel=0.01) for y in (-2.1, -0.7, 0.7, 2.1)]
    export("bridge", [join(parts, "bridge")], "Wooden Bridge", "Structures", [collider("box", (0, 0, 0.2), (1.9, 4.4, 0.4)), collider("box", (-0.9, 0, 0.7), (0.14, 4.4, 0.8)), collider("box", (0.9, 0, 0.7), (0.14, 4.4, 0.8))], ["path"])


def pickups():
    def gold_mat():
        return mat("Gold", GOLD, rough=0.25, metal=1.0)

    reset()
    gold = gold_mat()
    parts = [cyl(0.4, 0.1, (0, 0.05, 0.5), gold, verts=20, rot=(math.radians(90), 0, 0), bevel=0.02), cyl(0.27, 0.13, (0, 0.065, 0.5), mat("GoldDeep", "#f09a00", rough=0.3, metal=1.0), verts=20, rot=(math.radians(90), 0, 0))]
    export("coin", [join(parts, "coin")], "Coin", "Pickups", [], ["pickup"])

    reset()
    gem = mat("Gem", "#35d6ed", rough=0.05, metal=0.2, emit=0.8)
    parts = [cyl(0.3, 0.3, (0, 0, 0.45), gem, verts=6, radius_top=0.18, smooth=False), cyl(0.0, 0.45, (0, 0, 0.0), gem, verts=6, radius_top=0.3, smooth=False)]
    export("gem", [join(parts, "gem")], "Gem", "Pickups", [], ["pickup"])

    reset()
    heart = mat("Heart", RED, rough=0.3, emit=0.5)
    parts = [ball(0.24, (x, 0, 0.62), heart, subdiv=2, smooth=True) for x in (-0.17, 0.17)]
    parts.append(cyl(0.0, 0.5, (0, 0, 0.1), heart, verts=12, radius_top=0.38, smooth=True))
    obj = join(parts, "heart")
    obj.scale = (1, 0.55, 1)
    export("heart", [join([obj], "heart")], "Heart", "Pickups", [], ["pickup"])

    reset()
    pts = []
    for i in range(10):
        a = math.pi / 2 + i * math.tau / 10
        r = 0.45 if i % 2 == 0 else 0.2
        pts.append((math.cos(a) * r, 0.5 + math.sin(a) * r))
    export("star", [join([prism(pts, 0.16, (0, 0, 0), mat("Star", GOLD, rough=0.3, metal=0.6, emit=1.2), bevel=0.03)], "star")], "Star", "Pickups", [], ["pickup"])

    reset()
    gold = gold_mat()
    parts = [torus(0.16, 0.05, (0, 0, 0.75), gold, rot=(math.radians(90), 0, 0)), box((0.08, 0.06, 0.5), (0, 0, 0.36), gold), box((0.18, 0.06, 0.07), (0.09, 0, 0.2), gold), box((0.14, 0.06, 0.07), (0.07, 0, 0.34), gold)]
    export("key", [join(parts, "key")], "Key", "Pickups", [], ["pickup"])


def gameplay():
    reset()
    parts = [cyl(0.5, 0.12, material=mat("Stone", STONE_DARK), verts=12, bevel=0.02), cyl(0.05, 2.4, (0, 0, 0.1), mat("Pole", "#e8e8ee", rough=0.4, metal=0.5), verts=8), ball(0.09, (0, 0, 2.52), mat("Gold", GOLD, rough=0.3, metal=0.9), subdiv=1, smooth=True)]
    parts.append(prism([(0.05, 2.4), (1.15, 2.05), (0.05, 1.7)], 0.03, (0, 0, 0), mat("Flag", "#22c55e", rough=0.8)))
    export("checkpoint_flag", [join(parts, "checkpoint_flag")], "Checkpoint Flag", "Gameplay", [collider("cylinder", (0, 0, 1.2), (0.3, 0.3, 2.4))], ["checkpoint"])

    reset()
    parts = [cyl(1.5, 0.12, material=mat("PadBase", "#2a3142", rough=0.5, metal=0.4), verts=24, bevel=0.03), cyl(1.25, 0.05, (0, 0, 0.11), mat("PadGlow", "#4cc9f0", rough=0.3, emit=2.5), verts=24), cyl(0.8, 0.03, (0, 0, 0.15), mat("PadBase", "#2a3142", rough=0.5, metal=0.4), verts=24), cyl(0.55, 0.03, (0, 0, 0.17), mat("PadGlow", "#4cc9f0", rough=0.3, emit=2.5), verts=24)]
    export("spawn_pad", [join(parts, "spawn_pad")], "Spawn Pad", "Gameplay", [collider("cylinder", (0, 0, 0.08), (3.0, 3.0, 0.16))], ["spawn"])

    reset()
    parts = [box((2.0, 2.0, 0.16), (0, 0, 0.08), mat("TrapBase", "#3a3f4b", rough=0.5, metal=0.6), bevel=0.02)]
    for x in (-0.6, 0, 0.6):
        for y in (-0.6, 0, 0.6):
            parts.append(cyl(0.2, 0.75, (x, y, 0.16), mat("Spike", "#d9dee6", rough=0.25, metal=0.9), verts=6, radius_top=0.0, smooth=False))
    export("spikes", [join(parts, "spikes")], "Spike Trap", "Gameplay", [collider("box", (0, 0, 0.4), (1.9, 1.9, 0.8))], ["hazard"])

    reset()
    parts = [cyl(1.0, 0.2, material=mat("PadBase", "#2a3142", rough=0.5, metal=0.4), verts=20, bevel=0.03), cyl(0.82, 0.14, (0, 0, 0.2), mat("Bounce", "#ff5db1", rough=0.4, emit=1.0), verts=20, radius_top=0.7)]
    parts.append(prism([(-0.3, 0), (0.3, 0), (0, 0.35)], 0.5, (0, 0, 0.345), mat("BounceArrow", WHITE, emit=1.5), rot=(math.radians(-90), 0, 0)))
    export("bounce_pad", [join(parts, "bounce_pad")], "Bounce Pad", "Gameplay", [collider("cylinder", (0, 0, 0.17), (2.0, 2.0, 0.34))], ["launcher"])

    reset()
    check = [mat("ArchWhite", WHITE), mat("ArchBlack", "#20242e")]
    parts = [cyl(0.25, 5.0, (x, 0, 0), mat("ArchPole", RED, rough=0.4), verts=10) for x in (-5, 5)]
    for i in range(20):
        for j in range(2):
            parts.append(box((0.5, 0.2, 0.5), (-4.75 + i * 0.5, 0, 4.25 + j * 0.5), check[(i + j) % 2]))
    export("finish_arch", [join(parts, "finish_arch")], "Finish Arch", "Gameplay", [collider("cylinder", (-5, 0, 2.5), (0.5, 0.5, 5)), collider("cylinder", (5, 0, 2.5), (0.5, 0.5, 5))], ["racing"])

    reset()
    parts = [box((0.8, 0.8, 0.06), (0, 0, 0.03), mat("ConeBase", "#20242e")), cyl(0.3, 0.9, (0, 0, 0.06), mat("Cone", "#ff7b1c", rough=0.5), verts=12, radius_top=0.05), cyl(0.215, 0.16, (0, 0, 0.36), mat("ConeStripe", WHITE), verts=12, radius_top=0.17)]
    export("traffic_cone", [join(parts, "traffic_cone")], "Traffic Cone", "Gameplay", [collider("cylinder", (0, 0, 0.45), (0.6, 0.6, 0.9))], ["racing"])

    reset()
    parts = [ball(0.75, (0, 0, 0.45), mat("Stone", STONE_DARK, rough=0.9), subdiv=2, lumpy=0.4, seed=5, flat_bottom=-0.55)]
    for i, (x, y, z, s) in enumerate([(0.3, -0.35, 0.75, 0.2), (-0.35, -0.2, 0.6, 0.16), (0.0, 0.3, 0.9, 0.22), (-0.2, -0.45, 0.3, 0.13), (0.5, 0.1, 0.5, 0.15)]):
        parts.append(ball(s, (x, y, z), mat("Ore", "#ffb347", rough=0.3, metal=0.9, emit=0.3), subdiv=1, lumpy=0.3, seed=i))
    export("ore_rock", [join(parts, "ore_rock")], "Ore Rock", "Resources", [collider("sphere", (0, 0, 0.5), (1.3, 1.3, 1.3))], ["resource", "survival"])

    reset()
    crystal = mat("Crystal", "#b388ff", rough=0.05, emit=1.5)
    parts = [ball(0.5, (0, 0, 0.1), mat("Stone", STONE_DARK, rough=0.9), subdiv=1, lumpy=0.3, seed=2, scale=(1.2, 1.2, 0.4))]
    for x, y, h, r, tilt in [(0, 0, 1.6, 0.26, (0, 0)), (0.35, 0.1, 1.0, 0.18, (0.1, 0.4)), (-0.3, 0.15, 1.1, 0.2, (0.2, -0.4)), (0.05, -0.35, 0.8, 0.16, (-0.45, 0.05))]:
        parts.append(cyl(r, h * 0.75, (x, y, 0.1), crystal, verts=6, rot=(tilt[0], tilt[1], 0), smooth=False))
        dz = h * 0.75
        parts.append(cyl(r, h * 0.25, (x + math.sin(tilt[1]) * dz, y - math.sin(tilt[0]) * dz, 0.1 + math.cos(tilt[0]) * math.cos(tilt[1]) * dz), crystal, verts=6, radius_top=0.0, rot=(tilt[0], tilt[1], 0), smooth=False))
    export("crystal", [join(parts, "crystal")], "Crystal Cluster", "Resources", [collider("cylinder", (0, 0, 0.6), (0.9, 0.9, 1.2))], ["resource", "light"])


def kart():
    reset()
    body, dark, chrome = mat("KartBody", RED, rough=0.3, metal=0.3), mat("KartDark", "#20242e", rough=0.6), mat("Chrome", "#d9dee6", rough=0.2, metal=1.0)
    parts = [box((1.3, 2.3, 0.3), (0, 0, 0.42), body, bevel=0.08), box((1.0, 0.9, 0.3), (0, -0.85, 0.62), body, bevel=0.1, taper=0.7)]
    parts.append(box((0.9, 0.5, 0.7), (0, 0.75, 0.85), dark, bevel=0.08, taper=0.8))
    parts.append(box((0.75, 0.6, 0.12), (0, 0.3, 0.6), dark, bevel=0.03))
    parts.append(box((1.5, 0.12, 0.1), (0, 1.2, 1.15), body, bevel=0.02))
    parts += [box((0.08, 0.1, 0.6), (x, 1.2, 0.85), dark) for x in (-0.5, 0.5)]
    parts.append(cyl(0.03, 0.5, (0, -0.25, 0.55), dark, verts=6, rot=(math.radians(-35), 0, 0)))
    parts.append(torus(0.17, 0.035, (0, -0.52, 0.97), dark, rot=(math.radians(55), 0, 0), major_seg=14, minor_seg=6))
    parts.append(box((1.5, 0.15, 0.12), (0, -1.2, 0.36), chrome, bevel=0.03))
    for x in (-0.78, 0.78):
        for y in (-0.8, 0.8):
            parts.append(cyl(0.34, 0.3, (x - 0.15, y, 0.34), dark, verts=14, rot=(0, math.radians(90), 0), bevel=0.05))
            parts.append(cyl(0.16, 0.32, (x - 0.16, y, 0.34), chrome, verts=10, rot=(0, math.radians(90), 0)))
    export("kart", [join(parts, "kart")], "Kart", "Vehicles", [collider("box", (0, 0, 0.5), (1.7, 2.5, 0.9))], ["vehicle", "racing"])


def tools_and_misc():
    reset()
    parts = [cyl(0.04, 0.9, material=mat("Wood", WOOD), verts=6), box((0.4, 0.07, 0.26), (0.14, 0, 0.78), mat("Steel", "#c3cad3", rough=0.3, metal=0.9), bevel=0.02, taper=0.6, rot=(0, math.radians(90), 0))]
    export("axe", [join(parts, "axe")], "Axe", "Tools", [], ["tool", "survival"])

    reset()
    parts = [cyl(0.05, 0.85, material=mat("Wood", WOOD_DARK), verts=6), cyl(0.09, 0.1, (0, 0, 0.8), mat("Iron", "#3b4252", rough=0.45, metal=0.8), verts=8), cyl(0.09, 0.3, (0, 0, 0.9), mat("Flame", "#ff9a1c", emit=7.0), verts=7, radius_top=0.0, smooth=False)]
    export("torch", [join(parts, "torch")], "Torch", "Props", [], ["light"])

    reset()
    stone = mat("Stone", STONE, rough=0.9)
    parts = [cyl(0.95, 0.75, material=stone, verts=12, smooth=False), cyl(0.72, 0.05, (0, 0, 0.72), mat("Water", "#3aa6ff", rough=0.05, emit=0.2), verts=12)]
    parts += [box((0.14, 0.14, 1.5), (x, 0, 1.4), mat("Wood", WOOD_DARK), bevel=0.02) for x in (-0.8, 0.8)]
    parts.append(prism([(-1.15, 0), (1.15, 0), (0, 0.7)], 1.4, (0, 0, 2.1), mat("Roof", ROOF, rough=0.8)))
    parts.append(cyl(0.05, 1.7, (-0.85, 0, 1.7), mat("Wood", WOOD), verts=6, rot=(0, math.radians(90), 0)))
    export("well", [join(parts, "well")], "Well", "Structures", [collider("cylinder", (0, 0, 0.4), (1.9, 1.9, 0.8))], ["decor", "water"])

    reset()
    parts = [box((1.6, 0.5, 0.1), (0, 0, 0.5), mat("WoodLight", WOOD_LIGHT), bevel=0.02), box((1.6, 0.08, 0.5), (0, 0.24, 0.85), mat("WoodLight", WOOD_LIGHT), bevel=0.02)]
    parts += [box((0.1, 0.5, 0.5), (x, 0, 0.25), mat("Iron", "#3b4252", rough=0.45, metal=0.8)) for x in (-0.65, 0.65)]
    export("bench", [join(parts, "bench")], "Bench", "Props", [collider("box", (0, 0, 0.3), (1.6, 0.5, 0.6))], ["seat"])

    reset()
    tent = mat("Canvas", "#e9c46a", rough=0.95)
    parts = [prism([(-1.4, 0), (1.4, 0), (0, 1.7)], 2.6, (0, 0, 0), tent), prism([(-0.5, 0), (0.5, 0), (0, 1.2)], 0.05, (0, -1.3, 0), mat("TentDark", "#3a2e24"))]
    parts.append(cyl(0.04, 1.9, (0, -1.32, 0), mat("Wood", WOOD_DARK), verts=6))
    export("tent", [join(parts, "tent")], "Tent", "Structures", [collider("box", (0, 0, 0.6), (2.2, 2.6, 1.2))], ["survival", "building"])


def build():
    for f in [tree_pine, tree_oak, bush, rocks, flower_and_grass, mushroom, log_and_stump, cloud, crate_and_barrel, fence_and_sign, lamp_post, campfire, chest, house, tower, bridge, pickups, gameplay, kart, tools_and_misc]:
        f()
