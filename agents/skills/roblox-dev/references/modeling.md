# Modeling for Roblox: models as Blender scripts

Each model is a directory, `models/<name>/`:

| File | Who writes it | What it is |
|---|---|---|
| `build.py` | the Modeler | bpy script that builds the model on an empty scene. The only hand-written file. |
| `<name>.fbx` | `blender-run.sh` | What Roblox imports |
| `<name>.glb` | `blender-run.sh` | The same model as glTF, for previews and web tools |
| `preview.png` | `render-preview.sh` | 512 px render linked in the PR |
| `asset.json` | `upload-asset.sh` | `{asset_id, operation, file, sha256, uploaded_at}` |

## `build.py` rules

- **Start from nothing.** The runner hands you an empty scene. Don't
  import `.blend` files; build from primitives, bmesh, curves and modifiers.
- **Parameters at the top**, in studs, copied from the design doc's spec:
  `DIAMETER = 2.0`. A size change is then a one-line diff.
- **Scale: 1 Blender unit = 1 stud.** The export bakes transforms, uses Y up
  and faces -Z forward, which matches Roblox.
- **Name every object.** The FBX keeps names, and they show up in Studio.
- **Materials:** use Principled BSDF with base colour, metallic and
  roughness. Roblox takes the mesh and a base colour; set PBR looks in
  Studio with `SurfaceAppearance`, and note in the PR when you'd want one.
  No image textures unless the design asks.
- **Triangle budget:** props ≤ 2,000, hero objects ≤ 10,000, never above
  20,000 (Roblox's hard limit per mesh). Use `STATS` from `blender-run.sh`.
  Keep bevel segments and cylinder vertex counts modest.
- **Shade smooth plus a bevel** reads as "finished" in Roblox's lighting;
  flat-shaded faceting reads as low effort unless the art style is low-poly.
- **Pivot at the base centre** for things that stand on the ground, and at
  the centre for pickups that spin.

## Iterating

1. Write `build.py`.
2. `blender-run.sh`: read STATS.
3. `render-preview.sh`: look at `preview.png`. You can read images.
4. Adjust the parameters and repeat. Two or three passes is normal; stop
   when it matches the spec.
5. `upload-asset.sh` once, when it's right.

## Where Blender runs

| Environment | Blender | Preview device |
|---|---|---|
| `roblox-dev` (cloud; Mr. Walker on the Mac) | Ubuntu's `blender` 4.0, with `python3-numpy` for the exporters | CPU, about 2 s per preview, no denoiser |
| `rig-gpu` (the Windows rig, RTX 5070) | the worker image's Blender | GPU (OptiX/CUDA), picked automatically |

## The owner's Blender plugin

A friend's "Claude for Blender" plugin may be added later. When it is, its
use goes here. It will run inside this sandbox's Blender through the same
scripts, not as a separate tool.
