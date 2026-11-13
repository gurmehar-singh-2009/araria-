"""Renders a contact sheet of exported models so they can be reviewed.

    blender -b -P tools/blender/preview.py -- out.png [name-filter]

It imports the `.glb` files (not the source scenes), so it shows exactly
what the engine will load.
"""

import math
import os
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(__file__))
import lib  # noqa: E402

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
out = args[0] if args else "preview.png"
name_filter = args[1] if len(args) > 1 else ""

bpy.ops.wm.read_factory_settings(use_empty=True)
files = sorted(f for f in os.listdir(lib.MODELS_DIR) if f.endswith(".glb") and name_filter in f)
columns = max(1, math.ceil(math.sqrt(len(files) * 1.6)))
spacing = 6.0
for index, name in enumerate(files):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=os.path.join(lib.MODELS_DIR, name))
    new = [o for o in bpy.data.objects if o not in before]
    roots = [o for o in new if o.parent is None]
    lo, hi = lib.bounds(new)
    size = max((hi - lo).length, 0.01)
    scale = min(1.0, 4.5 / size) if size > 4.5 else (1.6 if size < 1.2 else 1.0)
    col, row = index % columns, index // columns
    for root in roots:
        root.scale = root.scale * scale
        root.location = Vector((col * spacing, -row * spacing, 0)) + root.location * scale

rows = math.ceil(len(files) / columns)
bpy.ops.mesh.primitive_plane_add(size=400, location=(0, 0, -0.01))
ground = bpy.context.object
ground_mat = bpy.data.materials.new("Ground")
ground_mat.use_nodes = True
next(n for n in ground_mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED").inputs["Base Color"].default_value = (0.55, 0.6, 0.66, 1)
ground.data.materials.append(ground_mat)

sun = bpy.data.lights.new("Sun", "SUN")
sun.energy = 3.5
sun_obj = bpy.data.objects.new("Sun", sun)
sun_obj.rotation_euler = (math.radians(50), 0, math.radians(35))
bpy.context.collection.objects.link(sun_obj)

world = bpy.data.worlds.new("World")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.75, 0.85, 1.0, 1)
world.node_tree.nodes["Background"].inputs[1].default_value = 0.9
bpy.context.scene.world = world

cam = bpy.data.cameras.new("Cam")
cam.type = "ORTHO"
cam.ortho_scale = max(columns * spacing, rows * spacing * 1.9)
cam_obj = bpy.data.objects.new("Cam", cam)
center = Vector(((columns - 1) * spacing / 2, -(rows - 1) * spacing / 2, 1.2))
cam_obj.location = center + Vector((0, -60, 45))
cam_obj.rotation_euler = (math.radians(53), 0, 0)
bpy.context.collection.objects.link(cam_obj)
scene = bpy.context.scene
scene.camera = cam_obj
scene.render.resolution_x = 1900
scene.render.resolution_y = int(1900 * rows / columns * 0.95) + 100
scene.render.filepath = out
engines = [i.identifier for i in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items]
scene.render.engine = "BLENDER_EEVEE" if "BLENDER_EEVEE" in engines else engines[0]
bpy.ops.render.render(write_still=True)
print("rendered", out, len(files), "models")
