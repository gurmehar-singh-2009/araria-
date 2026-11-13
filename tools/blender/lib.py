"""Shared helpers for Araria's Blender asset pipeline.

Run through `build.py`; every asset is modelled procedurally with bpy so the
whole library can be regenerated with one command. Modelling uses Blender's
coordinates (Z up, -Y is the front); the glTF exporter converts to the
engine's (Y up, +Z front).
"""

import json
import math
import os
import random

import bmesh
import bpy
from mathutils import Matrix, Vector, noise

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MODELS_DIR = os.path.join(ROOT, "assets", "models")
MANIFEST_PATH = os.path.join(ROOT, "assets", "manifest.json")

_materials = {}
manifest_models = {}


def reset():
    """Empties the scene (keeps cached material definitions out of the file)."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    _materials.clear()


def hex_rgb(value):
    value = value.lstrip("#")
    srgb = [int(value[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    # Principled BSDF expects linear colour.
    return [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in srgb]


def mat(name, color, rough=0.75, metal=0.0, emit=0.0, alpha=1.0):
    """A cached Principled material. Names are stable so the engine can
    recolour by material name (e.g. the character's `Shirt`)."""
    if name in _materials:
        return _materials[name]
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    rgb = hex_rgb(color)
    bsdf.inputs["Base Color"].default_value = (*rgb, 1.0)
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    if emit > 0:
        bsdf.inputs["Emission Color"].default_value = (*rgb, 1.0)
        bsdf.inputs["Emission Strength"].default_value = emit
    if alpha < 1:
        bsdf.inputs["Alpha"].default_value = alpha
        m.surface_render_method = "BLENDED"
    _materials[name] = m
    return m


def _finish(bm, name, material, smooth, loc=(0, 0, 0), rot=(0, 0, 0), scale=(1, 1, 1)):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    obj.location, obj.rotation_euler, obj.scale = loc, rot, scale
    if material:
        mesh.materials.append(material)
    for poly in mesh.polygons:
        poly.use_smooth = bool(smooth)
    if smooth and smooth is not True:
        mesh.set_sharp_from_angle(angle=math.radians(smooth))
    return obj


def box(size, loc=(0, 0, 0), material=None, bevel=0.0, rot=(0, 0, 0), name="box", taper=1.0):
    """A cuboid; `taper` scales the top face for trapezoid shapes."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=size, verts=bm.verts)
    if taper != 1.0:
        top = [v for v in bm.verts if v.co.z > 0]
        bmesh.ops.scale(bm, vec=(taper, taper, 1), verts=top)
    if bevel > 0:
        bmesh.ops.bevel(bm, geom=bm.edges[:], offset=bevel, segments=2, affect="EDGES", profile=0.7)
    return _finish(bm, name, material, 35 if bevel > 0 else False, loc, rot)


def cyl(radius, height, loc=(0, 0, 0), material=None, verts=12, radius_top=None, rot=(0, 0, 0), smooth=40, name="cyl", bevel=0.0):
    """A cylinder or cone standing on `loc` (its base is at loc.z)."""
    bm = bmesh.new()
    top = radius if radius_top is None else radius_top
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=verts, radius1=radius, radius2=max(top, 0.0001), depth=height)
    bmesh.ops.translate(bm, vec=(0, 0, height / 2), verts=bm.verts)
    if bevel > 0:
        rim = [e for e in bm.edges if abs(e.verts[0].co.z - e.verts[1].co.z) < 1e-5]
        bmesh.ops.bevel(bm, geom=rim, offset=bevel, segments=2, affect="EDGES")
    return _finish(bm, name, material, smooth, loc, rot)


def ball(radius, loc=(0, 0, 0), material=None, subdiv=2, scale=(1, 1, 1), lumpy=0.0, seed=0, smooth=False, name="ball", flat_bottom=None):
    """An icosphere, optionally distorted with noise for rocks and foliage."""
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=subdiv, radius=radius)
    if lumpy > 0:
        offset = Vector((seed * 13.1, seed * 7.7, seed * 3.3))
        for v in bm.verts:
            n = noise.noise(v.co * (1.3 / radius) + offset)
            v.co += v.co.normalized() * n * lumpy * radius
    if flat_bottom is not None:
        for v in bm.verts:
            if v.co.z < flat_bottom * radius:
                v.co.z = flat_bottom * radius
    return _finish(bm, name, material, smooth, loc, (0, 0, 0), scale)


def torus(major, minor, loc=(0, 0, 0), material=None, rot=(0, 0, 0), major_seg=20, minor_seg=8, name="torus"):
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    for i in range(major_seg):
        a = 2 * math.pi * i / major_seg
        for j in range(minor_seg):
            b = 2 * math.pi * j / minor_seg
            r = major + minor * math.cos(b)
            bm.verts.new((r * math.cos(a), r * math.sin(a), minor * math.sin(b)))
    bm.verts.ensure_lookup_table()
    for i in range(major_seg):
        for j in range(minor_seg):
            a, b = i * minor_seg + j, i * minor_seg + (j + 1) % minor_seg
            c, d = ((i + 1) % major_seg) * minor_seg + (j + 1) % minor_seg, ((i + 1) % major_seg) * minor_seg + j
            bm.faces.new([bm.verts[a], bm.verts[d], bm.verts[c], bm.verts[b]])
    return _finish(bm, name, material, True, loc, rot)


def prism(points, depth, loc=(0, 0, 0), material=None, rot=(0, 0, 0), name="prism", bevel=0.0):
    """Extrudes a 2D outline (x, z pairs) along Y, centred on Y."""
    bm = bmesh.new()
    front = [bm.verts.new((x, -depth / 2, z)) for x, z in points]
    back = [bm.verts.new((x, depth / 2, z)) for x, z in points]
    bm.faces.new(front)
    bm.faces.new(list(reversed(back)))
    n = len(points)
    for i in range(n):
        bm.faces.new([front[i], back[i], back[(i + 1) % n], front[(i + 1) % n]])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    if bevel > 0:
        bmesh.ops.bevel(bm, geom=bm.edges[:], offset=bevel, segments=1, affect="EDGES")
    return _finish(bm, name, material, False, loc, rot)


def join(objs, name):
    """Joins objects into one mesh whose origin is the world origin."""
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    if len(objs) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = name
    obj.data.name = name
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return obj


def to_engine(v):
    """Blender (x, y, z) → engine (x, z, -y), rounded."""
    return [round(v[0], 4), round(v[2], 4), round(-v[1], 4)]


def to_engine_size(v):
    return [round(abs(v[0]), 4), round(abs(v[2]), 4), round(abs(v[1]), 4)]


def collider(shape, center, size):
    """A collision primitive given in Blender coordinates. For cylinders and
    capsules `size` is (diameter, diameter, height)."""
    return {"shape": shape, "center": to_engine(center), "size": to_engine_size(size) if shape == "box" else [round(size[0], 4), round(size[2], 4), round(size[0], 4)]}


def bounds(objs):
    lo, hi = Vector((1e9,) * 3), Vector((-1e9,) * 3)
    for o in objs:
        if o.type != "MESH":
            continue
        for corner in o.bound_box:
            w = o.matrix_world @ Vector(corner)
            lo, hi = Vector(map(min, lo, w)), Vector(map(max, hi, w))
    return lo, hi


def export(asset, objs, display_name, category, colliders=None, tags=(), animations=()):
    """Writes `assets/models/<asset>.glb` and records its manifest entry."""
    os.makedirs(MODELS_DIR, exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    lo, hi = bounds(objs)
    triangles = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in objs if o.type == "MESH")
    path = os.path.join(MODELS_DIR, asset + ".glb")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_apply=not animations,
        export_yup=True,
        export_animations=bool(animations),
        export_skins=bool(animations),
        export_animation_mode="ACTIONS" if animations else "ACTIONS",
        export_force_sampling=True,
        export_materials="EXPORT",
    )
    manifest_models["models/" + asset] = {
        "file": "models/" + asset + ".glb",
        "name": display_name,
        "category": category,
        "size": to_engine_size(hi - lo),
        "center": to_engine((lo + hi) / 2),
        "colliders": colliders or [],
        "animations": list(animations),
        "triangles": triangles,
        "tags": list(tags),
    }
    print(f"exported {asset}: {triangles} tris, {os.path.getsize(path) // 1024} KiB")


def write_manifest():
    """Merges the model entries into assets/manifest.json (audio and textures
    are maintained by other tools and preserved)."""
    data = {"version": 1, "models": {}, "audio": {}, "textures": {}}
    if os.path.exists(MANIFEST_PATH):
        with open(MANIFEST_PATH) as f:
            data.update(json.load(f))
    data["models"] = dict(sorted(manifest_models.items()))
    with open(MANIFEST_PATH, "w") as f:
        json.dump(data, f, indent=1)
        f.write("\n")
    print(f"manifest: {len(data['models'])} models")


def rng(seed):
    return random.Random(seed)
