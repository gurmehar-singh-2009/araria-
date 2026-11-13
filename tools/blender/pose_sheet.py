"""Renders the character in a frame of every animation, for review.

    blender -b -P tools/blender/pose_sheet.py -- out.png
"""

import math
import os
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(__file__))
import character  # noqa: E402

out = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else "poses.png"
rig, body = character.build_character()
names = list(character.ANIMATIONS)
# Two sample times per animation, side by side.
samples = [(n, t) for n in names for t in (0.3, 0.75)]
columns = 8
for index, (name, t) in enumerate(samples):
    fn = character.ANIMATIONS[name][0]
    character.apply_pose(rig, fn(t), 1)
    bpy.context.view_layer.update()
    deps = bpy.context.evaluated_depsgraph_get()
    mesh = bpy.data.meshes.new_from_object(body.evaluated_get(deps))
    copy = bpy.data.objects.new(f"{name}_{index}", mesh)
    bpy.context.collection.objects.link(copy)
    copy.location = Vector(((index % columns) * 1.5, 0, -(index // columns) * 2.3))
    copy.rotation_euler = (0, 0, math.radians(-35))
body.hide_render = True
rig.hide_render = True

rows = math.ceil(len(samples) / columns)
sun = bpy.data.objects.new("Sun", bpy.data.lights.new("Sun", "SUN"))
sun.data.energy = 3.0
sun.rotation_euler = (math.radians(60), 0, math.radians(-20))
bpy.context.collection.objects.link(sun)
world = bpy.data.worlds.new("World")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.7, 0.78, 0.9, 1)
bpy.context.scene.world = world
cam = bpy.data.objects.new("Cam", bpy.data.cameras.new("Cam"))
cam.data.type = "ORTHO"
cam.data.ortho_scale = columns * 1.5 + 0.5
cam.location = ((columns - 1) * 0.75, -30, -(rows - 1) * 1.15 + 0.95)
cam.rotation_euler = (math.radians(90), 0, 0)
bpy.context.collection.objects.link(cam)
scene = bpy.context.scene
scene.camera = cam
scene.render.resolution_x = 1900
scene.render.resolution_y = int(1900 * (rows * 2.3 + 0.3) / (columns * 1.5 + 0.5))
scene.render.filepath = out
bpy.ops.render.render(write_still=True)
print("rendered", out)
