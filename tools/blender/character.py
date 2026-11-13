"""The player/NPC character: mesh, armature, skin weights and animations.

The mesh is built from tubes and blobs whose vertices are weighted by their
position along each limb, so joints bend smoothly. Materials are named
(`Skin`, `Shirt`, `Pants`, `Hair`, `Shoes`, `Eyes`) so the engine can
recolour individual characters.
"""

import math

import bmesh
import bpy
from mathutils import Euler, Matrix, Quaternion, Vector

from lib import collider, export, mat, reset

FPS = 30

# name: (head, tail, parent). Z up, the character faces -Y, +X is its left.
BONES = {
    "Root": ((0, 0, 0), (0, 0, 0.2), None),
    "Hips": ((0, 0, 0.92), (0, 0, 1.06), "Root"),
    "Spine": ((0, 0, 1.06), (0, 0, 1.26), "Hips"),
    "Chest": ((0, 0, 1.26), (0, 0, 1.48), "Spine"),
    "Neck": ((0, 0, 1.48), (0, 0, 1.57), "Chest"),
    "Head": ((0, 0, 1.57), (0, 0, 1.86), "Neck"),
}
for side, s in (("L", 1), ("R", -1)):
    BONES[f"UpperArm.{side}"] = ((0.2 * s, 0, 1.43), (0.45 * s, 0, 1.43), "Chest")
    BONES[f"LowerArm.{side}"] = ((0.45 * s, 0, 1.43), (0.68 * s, 0, 1.43), f"UpperArm.{side}")
    BONES[f"Hand.{side}"] = ((0.68 * s, 0, 1.43), (0.8 * s, 0, 1.43), f"LowerArm.{side}")
    BONES[f"UpperLeg.{side}"] = ((0.1 * s, 0, 0.92), (0.1 * s, 0, 0.52), "Hips")
    BONES[f"LowerLeg.{side}"] = ((0.1 * s, 0, 0.52), (0.1 * s, 0, 0.1), f"UpperLeg.{side}")
    BONES[f"Foot.{side}"] = ((0.1 * s, 0, 0.1), (0.1 * s, -0.2, 0.02), f"LowerLeg.{side}")


def smoothstep(a, b, x):
    t = max(0.0, min(1.0, (x - a) / (b - a)))
    return t * t * (3 - 2 * t)


def loft(rings, material, weight_fn, name):
    """Builds a tube through `rings` [(center, radius_x, radius_y, axis)] and
    weights each vertex with `weight_fn(position) -> {bone: weight}`."""
    segments = 12
    bm = bmesh.new()
    loops = []
    for center, rx, ry, axis in rings:
        axis = Vector(axis).normalized()
        side = axis.cross(Vector((0, 1, 0)))
        if side.length < 1e-4:
            side = Vector((1, 0, 0))
        side.normalize()
        up = axis.cross(side).normalized()
        loop = []
        for i in range(segments):
            a = math.tau * i / segments
            loop.append(bm.verts.new(Vector(center) + side * math.cos(a) * rx + up * math.sin(a) * ry))
        loops.append(loop)
    for a, b in zip(loops, loops[1:]):
        for i in range(segments):
            bm.faces.new([a[i], a[(i + 1) % segments], b[(i + 1) % segments], b[i]])
    bm.faces.new(list(reversed(loops[0])))
    bm.faces.new(loops[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(bm, material, weight_fn, name)


def blob(center, radius, scale, material, weight_fn, name, subdiv=2, cut_below=None):
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=subdiv, radius=radius)
    if cut_below is not None:
        doomed = [v for v in bm.verts if v.co.z < cut_below * radius]
        bmesh.ops.delete(bm, geom=doomed, context="VERTS")
    for v in bm.verts:
        v.co = Vector((v.co.x * scale[0], v.co.y * scale[1], v.co.z * scale[2])) + Vector(center)
    return finish(bm, material, weight_fn, name)


def finish(bm, material, weight_fn, name):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    mesh.materials.append(material)
    for poly in mesh.polygons:
        poly.use_smooth = True
    for v in mesh.vertices:
        weights = weight_fn(v.co)
        total = sum(weights.values())
        for bone, w in weights.items():
            if w <= 0.001:
                continue
            group = obj.vertex_groups.get(bone) or obj.vertex_groups.new(name=bone)
            group.add([v.index], w / total, "REPLACE")
    return obj


def rigid(bone):
    return lambda _co: {bone: 1.0}


def build_mesh():
    skin, shirt, pants = mat("Skin", "#f2c9a0", rough=0.7), mat("Shirt", "#3d8bfd", rough=0.8), mat("Pants", "#2b3350", rough=0.85)
    hair, shoes, eyes = mat("Hair", "#4a3628", rough=0.9), mat("Shoes", "#f4f1ea", rough=0.6), mat("Eyes", "#1d2433", rough=0.2)
    parts = []

    def torso_weights(co):
        hips = 1 - smoothstep(0.98, 1.1, co.z)
        chest = smoothstep(1.2, 1.32, co.z)
        return {"Hips": hips, "Spine": max(0.0, 1 - hips - chest), "Chest": chest}

    z = Vector((0, 0, 1))
    parts.append(loft([((0, 0, 0.855), 0.17, 0.12, z), ((0, 0, 0.9), 0.2, 0.135, z), ((0, 0, 1.0), 0.205, 0.14, z), ((0, 0, 1.12), 0.185, 0.125, z)],
                      pants, torso_weights, "pelvis"))
    parts.append(loft([((0, 0, 1.1), 0.19, 0.13, z), ((0, 0, 1.22), 0.2, 0.135, z), ((0, 0, 1.36), 0.225, 0.14, z), ((0, 0, 1.46), 0.21, 0.125, z), ((0, 0, 1.5), 0.11, 0.09, z)],
                      shirt, torso_weights, "torso"))
    parts.append(loft([((0, 0, 1.47), 0.07, 0.07, z), ((0, 0, 1.6), 0.065, 0.065, z)], skin,
                      lambda co: {"Neck": 1 - smoothstep(1.5, 1.58, co.z), "Head": smoothstep(1.5, 1.58, co.z)}, "neck"))

    parts.append(blob((0, 0, 1.74), 0.22, (1.0, 0.98, 1.02), skin, rigid("Head"), "head", subdiv=3))
    parts.append(blob((0, 0.02, 1.765), 0.235, (1.02, 1.02, 1.0), hair, rigid("Head"), "hair", subdiv=3, cut_below=0.12))
    parts.append(blob((0, 0.1, 1.68), 0.2, (1.0, 0.75, 0.9), hair, rigid("Head"), "hair_back", subdiv=2))
    for s in (1, -1):
        parts.append(blob((0.085 * s, -0.195, 1.74), 0.03, (1, 0.6, 1.25), eyes, rigid("Head"), "eye", subdiv=2))
    parts.append(blob((0, -0.21, 1.655), 0.03, (1.7, 0.5, 0.45), mat("Mouth", "#b55d57", rough=0.6), rigid("Head"), "mouth", subdiv=1))

    for side, s in (("L", 1), ("R", -1)):
        x = Vector((s, 0, 0))

        def arm_weights(co, side=side, s=s):
            d = co.x * s
            shoulder = 1 - smoothstep(0.16, 0.26, d)
            lower = smoothstep(0.4, 0.5, d)
            return {"Chest": shoulder, "UpperArm." + side: max(0.0, 1 - shoulder - lower), "LowerArm." + side: lower}

        parts.append(loft([((0.17 * s, 0, 1.43), 0.075, 0.085, x), ((0.24 * s, 0, 1.43), 0.078, 0.082, x), ((0.36 * s, 0, 1.43), 0.066, 0.068, x), ((0.44 * s, 0, 1.43), 0.062, 0.064, x)],
                          shirt, arm_weights, "sleeve"))
        parts.append(loft([((0.43 * s, 0, 1.43), 0.056, 0.058, x), ((0.5 * s, 0, 1.43), 0.055, 0.056, x), ((0.6 * s, 0, 1.43), 0.05, 0.05, x), ((0.69 * s, 0, 1.43), 0.044, 0.044, x)],
                          skin, arm_weights, "forearm"))
        parts.append(blob((0.745 * s, 0, 1.43), 0.065, (1.15, 0.8, 0.95), skin, rigid("Hand." + side), "hand"))

        def leg_weights(co, side=side):
            hip = smoothstep(0.84, 0.94, co.z)
            lower = 1 - smoothstep(0.47, 0.57, co.z)
            return {"Hips": hip, "UpperLeg." + side: max(0.0, 1 - hip - lower), "LowerLeg." + side: lower}

        lx = 0.1 * s
        parts.append(loft([((lx, 0, 0.95), 0.095, 0.105, z), ((lx, 0, 0.82), 0.098, 0.1, z), ((lx, 0, 0.62), 0.082, 0.085, z), ((lx, 0, 0.52), 0.076, 0.08, z),
                           ((lx, 0, 0.4), 0.074, 0.078, z), ((lx, 0, 0.2), 0.064, 0.066, z), ((lx, 0, 0.12), 0.062, 0.064, z)], pants, leg_weights, "leg"))
        parts.append(blob((lx, -0.06, 0.065), 0.1, (0.85, 1.55, 0.65), shoes, rigid("Foot." + side), "shoe"))

    bpy.ops.object.select_all(action="DESELECT")
    for p in parts:
        p.select_set(True)
    bpy.context.view_layer.objects.active = parts[0]
    bpy.ops.object.join()
    body = bpy.context.view_layer.objects.active
    body.name = body.data.name = "Character"
    return body


def build_armature():
    data = bpy.data.armatures.new("Armature")
    rig = bpy.data.objects.new("Armature", data)
    bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    for name, (head, tail, parent) in BONES.items():
        bone = data.edit_bones.new(name)
        bone.head, bone.tail = head, tail
        bone.roll = 0.0
        if parent:
            bone.parent = data.edit_bones[parent]
    bpy.ops.object.mode_set(mode="OBJECT")
    for pose_bone in rig.pose.bones:
        pose_bone.rotation_mode = "QUATERNION"
    return rig


class Pose:
    """Rotations (degrees, about the character's X/Y/Z axes as carried by
    the parent bone) plus a hips offset, for one instant."""

    def __init__(self):
        self.rot = {}
        self.hips = Vector((0, 0, 0))

    def r(self, bone, x=0.0, y=0.0, z=0.0):
        self.rot[bone] = (x, y, z)
        return self

    def both(self, bone, x=0.0, y=0.0, z=0.0):
        # Mirror: Y and Z rotations flip sign on the right side.
        self.rot[bone + ".L"] = (x, y, z)
        self.rot[bone + ".R"] = (x, -y, -z)
        return self


def apply_pose(rig, pose, frame):
    for pose_bone in rig.pose.bones:
        x, y, z = pose.rot.get(pose_bone.name, (0, 0, 0))
        world = Euler((math.radians(x), math.radians(y), math.radians(z)), "XYZ").to_quaternion()
        rest = pose_bone.bone.matrix_local.to_quaternion()
        pose_bone.rotation_quaternion = rest.inverted() @ world @ rest
        pose_bone.keyframe_insert("rotation_quaternion", frame=frame)
        if pose_bone.name == "Hips":
            rest_inv = pose_bone.bone.matrix_local.to_3x3().inverted()
            pose_bone.location = rest_inv @ pose.hips
            pose_bone.keyframe_insert("location", frame=frame)


# Arms hang at the sides in the neutral pose (the rig is modelled in T-pose).
ARM_DOWN = 72.0


def neutral():
    p = Pose()
    p.both("UpperArm", y=ARM_DOWN)
    p.both("LowerArm", z=-12)
    return p


def idle(t):
    w = math.sin(t * math.tau)
    p = neutral()
    p.hips = Vector((0, 0, -0.008 + 0.008 * w))
    p.r("Spine", x=1.0 + w).r("Chest", x=1.5 * w).r("Head", x=-1.5 * w, z=3 * math.sin(t * math.tau / 2))
    p.both("UpperArm", y=ARM_DOWN + 2 * w, x=2 * w)
    return p


def locomotion(t, stride, lean, bounce, arm):
    a = t * math.tau
    s, c = math.sin(a), math.cos(a)
    p = Pose()
    p.hips = Vector((0, 0, -0.03 + bounce * abs(c)))
    p.r("Hips", x=-lean * 0.4, z=6 * s).r("Spine", x=-lean * 0.5).r("Chest", x=-lean * 0.3, z=-9 * s).r("Head", x=lean * 0.6, z=3 * s)
    for side, k in (("L", 1), ("R", -1)):
        swing = s * k
        p.r("UpperLeg." + side, x=-stride * swing - 4)
        # The knee bends while the leg travels forward.
        p.r("LowerLeg." + side, x=stride * 1.25 * max(0.0, math.cos(a + (0 if k == 1 else math.pi) - 0.6)) + 6)
        p.r("Foot." + side, x=-stride * 0.3 * swing)
        p.r("UpperArm." + side, y=ARM_DOWN * k, x=arm * swing)
        p.r("LowerArm." + side, x=-arm * 0.6 - arm * 0.3 * swing, z=-10 * k)
    return p


def walk(t):
    return locomotion(t, 26, 3, 0.022, 28)


def run(t):
    return locomotion(t, 44, 12, 0.05, 55)


def jump(t):
    p = Pose()
    k = smoothstep(0, 0.5, t)
    p.r("Spine", x=-4 * k).r("Chest", x=-6 * k).r("Head", x=-6 * k)
    p.both("UpperArm", y=ARM_DOWN - 105 * k, x=-20 * k)
    p.both("LowerArm", z=-20)
    p.r("UpperLeg.L", x=-38 * k).r("LowerLeg.L", x=62 * k).r("UpperLeg.R", x=8 * k).r("LowerLeg.R", x=24 * k)
    p.both("Foot", x=22 * k)
    return p


def fall(t):
    w = math.sin(t * math.tau)
    p = Pose()
    p.r("Spine", x=4).r("Chest", x=3).r("Head", x=8)
    p.both("UpperArm", y=18 + 8 * w, x=-10 + 6 * w)
    p.both("LowerArm", z=-25 + 6 * w)
    p.r("UpperLeg.L", x=-22 + 6 * w).r("LowerLeg.L", x=38).r("UpperLeg.R", x=-6 - 6 * w).r("LowerLeg.R", x=30)
    p.both("Foot", x=18)
    return p


def land(t):
    k = math.sin(min(t, 1.0) * math.pi) * (1 - 0.5 * t)
    p = neutral()
    p.hips = Vector((0, 0, -0.22 * k))
    p.r("Spine", x=-16 * k).r("Chest", x=-10 * k).r("Head", x=12 * k)
    p.both("UpperLeg", x=-58 * k)
    p.both("LowerLeg", x=105 * k)
    p.both("Foot", x=-40 * k)
    p.both("UpperArm", y=ARM_DOWN - 25 * k, x=18 * k)
    return p


def sit(t):
    w = math.sin(t * math.tau)
    p = neutral()
    p.hips = Vector((0, 0, -0.42))
    p.r("Spine", x=2 + w).r("Head", x=-2 * w)
    p.both("UpperLeg", x=-88, z=6)
    p.both("LowerLeg", x=88)
    p.both("UpperArm", y=ARM_DOWN - 4, x=-18)
    p.both("LowerArm", x=-35, z=-10)
    return p


def wave(t):
    k = math.sin(min(1.0, t * 5) * math.pi / 2) * math.sin(min(1.0, (1 - t) * 5) * math.pi / 2)
    w = math.sin(t * math.tau * 3)
    p = idle(t)
    p.r("UpperArm.R", y=-(ARM_DOWN - 150 * k), z=10 * k)
    p.r("LowerArm.R", z=12 + (40 + 28 * w) * k)
    p.r("Hand.R", z=18 * w * k)
    p.r("Head", z=-8 * k, y=4 * k)
    return p


def cheer(t):
    k = math.sin(min(1.0, t * 4) * math.pi / 2) * math.sin(min(1.0, (1 - t) * 4) * math.pi / 2)
    hop = abs(math.sin(t * math.tau * 2)) * k
    p = neutral()
    p.hips = Vector((0, 0, 0.09 * hop - 0.03 * k))
    p.r("Chest", x=6 * k).r("Head", x=10 * k)
    p.both("UpperArm", y=ARM_DOWN - 150 * k, x=-8 * k)
    p.both("LowerArm", z=-12 - 22 * k * (1 - hop))
    p.both("UpperLeg", x=-14 * k * (1 - hop))
    p.both("LowerLeg", x=28 * k * (1 - hop))
    return p


def dance(t):
    a = t * math.tau
    s, s2 = math.sin(a), math.sin(a * 2)
    p = Pose()
    p.hips = Vector((0.05 * s, 0, -0.05 - 0.035 * abs(s2)))
    p.r("Hips", z=12 * s, y=5 * s).r("Spine", y=-6 * s).r("Chest", z=-16 * s, y=-5 * s).r("Head", z=8 * s, y=6 * s2)
    p.r("UpperArm.L", y=ARM_DOWN - 70 - 45 * s, x=-25 * s2).r("UpperArm.R", y=-(ARM_DOWN - 70 + 45 * s), x=25 * s2)
    p.r("LowerArm.L", z=-55 - 25 * s2).r("LowerArm.R", z=55 - 25 * s2)
    p.r("UpperLeg.L", x=-14 - 12 * max(0, s), z=8).r("UpperLeg.R", x=-14 - 12 * max(0, -s), z=-8)
    p.r("LowerLeg.L", x=26 + 22 * max(0, s)).r("LowerLeg.R", x=26 + 22 * max(0, -s))
    return p


def point(t):
    k = math.sin(min(1.0, t * 4) * math.pi / 2) * math.sin(min(1.0, (1 - t) * 4) * math.pi / 2)
    p = idle(t)
    p.r("UpperArm.R", y=-ARM_DOWN * (1 - k), z=88 * k)
    p.r("LowerArm.R", z=12 * (1 - k))
    p.r("Chest", z=10 * k).r("Head", z=6 * k)
    return p


def collapse(t):
    k = smoothstep(0, 0.75, t)
    p = neutral()
    p.hips = Vector((0, 0.35 * k, -0.78 * k))
    p.r("Hips", x=82 * k).r("Spine", x=4 * k).r("Head", x=-12 * k, z=25 * k)
    p.r("UpperLeg.L", x=-10 * k, z=10 * k).r("UpperLeg.R", x=-22 * k, z=-8 * k).r("LowerLeg.R", x=30 * k)
    p.r("UpperArm.L", y=ARM_DOWN - 50 * k, x=-20 * k).r("UpperArm.R", y=-(ARM_DOWN - 95 * k))
    return p


# name: (pose function, seconds, loops)
ANIMATIONS = {
    "Idle": (idle, 2.4, True),
    "Walk": (walk, 0.8, True),
    "Run": (run, 0.56, True),
    "Jump": (jump, 0.3, False),
    "Fall": (fall, 0.7, True),
    "Land": (land, 0.34, False),
    "Sit": (sit, 2.4, True),
    "Wave": (wave, 1.8, False),
    "Cheer": (cheer, 1.4, False),
    "Dance": (dance, 1.6, True),
    "Point": (point, 1.4, False),
    "Collapse": (collapse, 0.9, False),
}


def bake_animations(rig):
    rig.animation_data_create()
    for name, (fn, seconds, _loops) in ANIMATIONS.items():
        action = bpy.data.actions.new(name)
        rig.animation_data.action = action
        frames = max(2, round(seconds * FPS))
        for frame in range(frames + 1):
            apply_pose(rig, fn(frame / frames), frame)
        track = rig.animation_data.nla_tracks.new()
        track.name = name
        track.strips.new(name, 0, action)
        rig.animation_data.action = None
    # Leave the rig in its rest pose for export.
    for pose_bone in rig.pose.bones:
        pose_bone.rotation_quaternion = Quaternion()
        pose_bone.location = Vector((0, 0, 0))


def build_character():
    reset()
    bpy.context.scene.render.fps = FPS
    body = build_mesh()
    rig = build_armature()
    body.parent = rig
    modifier = body.modifiers.new("Armature", "ARMATURE")
    modifier.object = rig
    bake_animations(rig)
    return rig, body


def build():
    rig, body = build_character()
    bpy.ops.object.select_all(action="DESELECT")
    rig.select_set(True)
    body.select_set(True)
    bpy.context.view_layer.objects.active = rig
    export("character", [rig, body], "Character", "Characters", [collider("capsule", (0, 0, 0.9), (0.72, 0.72, 1.8))], ["character"], animations=list(ANIMATIONS))
