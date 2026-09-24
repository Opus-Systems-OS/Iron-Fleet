# Run inside Blender (blender -b --factory-startup -P export.py -- <model dir>).
# Executes <model dir>/build.py on an empty scene, then exports every mesh as
# <name>.fbx (what Roblox imports) and <name>.glb, saves /tmp/<name>.blend for
# the preview render, and prints one STATS line. Modifiers are applied on
# export; transforms are baked; Y is up, as Roblox expects.
import json
import os
import sys

import bpy

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
if not args:
    raise SystemExit("usage: blender -b --factory-startup -P export.py -- <model dir>")
model_dir = os.path.abspath(args[0])
name = os.path.basename(model_dir.rstrip("/"))
build = os.path.join(model_dir, "build.py")
if not os.path.exists(build):
    raise SystemExit(f"no build.py in {model_dir}")

bpy.ops.wm.read_factory_settings(use_empty=True)
code = compile(open(build).read(), build, "exec")
exec(code, {"__name__": "__main__", "__file__": build, "bpy": bpy})

meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
if not meshes:
    raise SystemExit("build.py made no mesh objects")

depsgraph = bpy.context.evaluated_depsgraph_get()
tris = 0
for o in meshes:
    m = o.evaluated_get(depsgraph).to_mesh()
    m.calc_loop_triangles()
    tris += len(m.loop_triangles)
    o.evaluated_get(depsgraph).to_mesh_clear()

fbx = os.path.join(model_dir, f"{name}.fbx")
bpy.ops.export_scene.fbx(
    filepath=fbx,
    use_selection=False,
    object_types={"MESH", "EMPTY"},
    use_mesh_modifiers=True,
    mesh_smooth_type="FACE",
    apply_scale_options="FBX_SCALE_ALL",
    bake_space_transform=True,
    axis_forward="-Z",
    axis_up="Y",
    add_leaf_bones=False,
)
glb = os.path.join(model_dir, f"{name}.glb")
bpy.ops.export_scene.gltf(filepath=glb, export_format="GLB", export_apply=True)
bpy.ops.wm.save_as_mainfile(filepath=f"/tmp/{name}.blend")

print("STATS " + json.dumps({
    "name": name,
    "objects": len(meshes),
    "triangles": tris,
    "fbx_bytes": os.path.getsize(fbx),
    "glb_bytes": os.path.getsize(glb),
    "blender": bpy.app.version_string,
}))
