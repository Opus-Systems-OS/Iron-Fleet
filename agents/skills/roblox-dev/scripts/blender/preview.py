# Run inside Blender (blender -b -P preview.py -- <model dir>). Opens the scene
# export.py saved, frames every mesh with a camera and a key/fill light, and
# renders <model dir>/preview.png at 512 px with Cycles — on the GPU when one
# is there (the rig), else the CPU (the cloud sandbox: ~2 s). The apt Blender
# has no OpenImageDenoise, so denoising stays off.
import math
import os
import sys

import bpy
from mathutils import Vector

model_dir = os.path.abspath((sys.argv[sys.argv.index("--") + 1 :] or ["."])[0])
name = os.path.basename(model_dir.rstrip("/"))
bpy.ops.wm.open_mainfile(filepath=f"/tmp/{name}.blend")
scene = bpy.context.scene

corners = [o.matrix_world @ Vector(c) for o in scene.objects if o.type == "MESH" for c in o.bound_box]
lo = Vector((min(c.x for c in corners), min(c.y for c in corners), min(c.z for c in corners)))
hi = Vector((max(c.x for c in corners), max(c.y for c in corners), max(c.z for c in corners)))
center, size = (lo + hi) / 2, max((hi - lo).length, 0.01)

cam = bpy.data.objects.new("PreviewCam", bpy.data.cameras.new("PreviewCam"))
scene.collection.objects.link(cam)
cam.location = center + Vector((size * 1.1, -size * 1.4, size * 0.9))
cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
for n, loc, energy in (("Key", (1.5, -1.5, 2.0), 800), ("Fill", (-2.0, -0.5, 1.0), 250)):
    light = bpy.data.objects.new(n, bpy.data.lights.new(n, "AREA"))
    light.data.energy = energy * size * size
    light.data.size = size
    light.location = center + Vector(loc) * size
    light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(light)
world = scene.world or bpy.data.worlds.new("World")
scene.world = world
world.use_nodes = True
# Metals only look like metal if there is something to reflect: a warm
# grey world lights the model; the rendered background stays dark.
bg = world.node_tree.nodes["Background"]
bg.inputs[0].default_value = (0.55, 0.5, 0.45, 1)
bg.inputs[1].default_value = 0.8
scene.render.film_transparent = True

scene.render.engine = "CYCLES"
scene.cycles.samples = 32
scene.cycles.use_denoising = False
prefs = bpy.context.preferences.addons["cycles"].preferences
device = "CPU"
for backend in ("OPTIX", "CUDA"):
    try:
        prefs.compute_device_type = backend
        prefs.get_devices()
        if any(d.type == backend for d in prefs.devices):
            for d in prefs.devices:
                d.use = d.type == backend
            device = "GPU"
            break
    except Exception:
        continue
scene.cycles.device = device
scene.render.resolution_x = scene.render.resolution_y = 512
scene.render.filepath = os.path.join(model_dir, "preview.png")
bpy.ops.render.render(write_still=True)
print(f"PREVIEW {scene.render.filepath} device={device}")
