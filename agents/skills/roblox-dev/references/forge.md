# Forge: RoForge, VFXForge and MoonForge in the sandbox

AR1P-D's tools, from the private `Opus-Systems-OS/Forge` repo, mounted at
`/workspace/Forge`:

| Tool | Makes | Who uses it |
|---|---|---|
| **RoForge** | 3D models in a live headless Blender, with the `rf` toolkit, inspector, PBR bake and export | 3D Modeler |
| **VFXForge** | Effects: a JSON spec compiled to particles, beams and textures, with a contact sheet | VFX & Animation |
| **MoonForge** | Animations: a JSON plan of key poses baked at 60 fps with IK and springs, with a contact sheet | VFX & Animation |

## You are the AI: the spend rule

Each tool can call an AI of its own, which would bill an account outside the
fleet. In this sandbox **you** are the AI: you write the spec, plan or `rf`
code, run the no-AI path, look at the image, and iterate.

- **Never run:** RoForge `build`, `chat`, `doctor --ping-ai`; VFXForge and
  MoonForge `generate`, `revise`, `test`, `login`, or their bridges
  (`serve`, the default command).
- **Never configure an API key** or a provider profile for them.
- **Use only:** RoForge's tools through its `Bridge`; VFXForge `presets`,
  `preset`, `preview`, `texture`, `textures`, `search` and
  `director.build_spec`; MoonForge `bake` and `director.inspect`.

## Setup, once per session

Work on a copy, so the tools can write their config and output:

```sh
cp -r /workspace/Forge ~/forge
bash <skill dir>/scripts/setup.sh --blender   # RoForge only: Blender 4.5 LTS (~380 MB)
export PATH="$HOME/.local/bin:$PATH"
```

VFXForge and MoonForge are plain Python 3.8+ with no dependencies.

## RoForge (3D Modeler)

Use it when a model needs more than `build.py` with primitives and modifiers can
give, such as organic shapes or baked detail. `models/<name>/build.py` stays the
source of truth; write it with `rf` (its API is
`~/forge/RoForge/app/roforge/prompts/rf_api.md`; read it first).

A headless Blender lives only as long as the Python process that launched it,
so run **one script per pass** that rebuilds the model from `build.py`.
RoForge reads `config.toml` with `tomllib`, so run it with Blender's bundled
Python 3.11:

```sh
BPY=$(ls ~/.local/opt/blender-4.5.14-linux-x64/4.5/python/bin/python3.1*)
ROFORGE_BLENDER=$HOME/.local/bin/blender PYTHONPATH=$HOME/forge/RoForge/app "$BPY" pass.py
```

```python
# pass.py: one build → look → check pass for models/<name>
import base64, pathlib, sys
from roforge import config, tools
from roforge.bridge import Bridge

d = pathlib.Path("models/coin")
bridge = Bridge(config.load(), mode="headless")
call = lambda tool, args: tools.run(bridge.call, tool, args)

call("new_project", {"name": d.name, "description": "spin pickup, 2 studs"})
r = call("run_python", {"code": (d / "build.py").read_text()})
if r.is_error: sys.exit(r.text)
r = call("render_preview", {"views": ["iso", "front", "right", "top"], "scale_ref": True})
for i, (media_type, b64) in enumerate(r.images):   # (media type, base64) pairs
    ext = media_type.split("/")[-1].replace("jpeg", "jpg")
    (d / f"preview{'' if i == 0 else i}.{ext}").write_bytes(base64.b64decode(b64))
print(call("inspect", {}).text)
print(call("validate", {}).text)
# Final pass only:
# print(call("bake", {"name": d.name, "target_tris": 2000}).text)
# print(call("export", {"name": d.name, "fmt": "fbx"}).text)
```

- Look at `preview.png` every pass, as in `modeling.md`.
- Pass the house budget as `target_tris` (props 2,000, hero 10,000).
- `export` writes to `~/forge/RoForge/projects/<name>/export/`. Copy the
  FBX to `models/<name>/<name>.fbx`, then `upload-asset.sh` it as usual.
- The baked colour, normal, roughness and metalness maps are **not uploaded
  yet**: the Roblox mesh arrives with its base colour only. List the maps in
  the PR so Mr. Walker can add a `SurfaceAppearance` in Studio.
- **RoForge has never run on Linux or without a GPU.** If its previews or bake
  fail here, say so in your report, and fall back to `blender-run.sh` and
  `render-preview.sh` for this model.

## VFXForge (VFX & Animation)

Each effect is `vfx/<name>/spec.json`. For the format, start from the closest
preset in `~/forge/VFXForge/core/presets/` (`AnimeSlash`, `Explosion`,
`GroundSlam`, `HealAura`, …); the craft guide is
`~/forge/VFXForge/core/prompts.py`.

```sh
cd ~/forge/VFXForge
python3 vfxforge.py presets                                        # list presets
python3 vfxforge.py preview /workspace/<repo>/vfx/<name>/spec.json --out /workspace/<repo>/vfx/<name>/sheet.png
python3 vfxforge.py texture smoke_flipbook --params '{"toon":3}'   # procedural textures (vfxforge.py textures lists them)
python3 vfxforge.py search "shockwave ring"                        # Creator Store decals, no key
```

Look at `sheet.png`, change the spec, and repeat. Then compile it, which
normalizes it, gets textures, lints it and builds the instances:

```sh
cd ~/forge/VFXForge && python3 -c '
import json, sys; sys.path.insert(0, ".")
from core import config, director
raw = json.load(open(sys.argv[1]))
r = director.build_spec(config.load(), raw, {"sheet": False, "scale": 1.0})
print("\n".join(r["warnings"]), r["report_text"], sep="\n")
json.dump(r, open(sys.argv[2], "w"), indent=1)' /workspace/<repo>/vfx/<name>/spec.json /workspace/<repo>/vfx/<name>/compiled.json
```

## MoonForge (VFX & Animation)

Each animation is `animations/<name>/plan.json`: key poses with times, eases
and holds. The format is `~/forge/MoonForgeAI/examples/heavy_punch_r15.json`,
and the craft guide is `~/forge/MoonForgeAI/core/prompts.py`. R15 and R6 are
built in. A custom rig needs the rig JSON the Studio plugin extracts; ask
Jarvis for it rather than guessing.

```sh
cd ~/forge/MoonForgeAI && python3 -c '
import json, shutil, sys; sys.path.insert(0, ".")
from core import director, rigs
plan = json.load(open(sys.argv[1]))
res, report, sheet = director.inspect(plan, rigs.template(sys.argv[3]), {"mode": "smart", "fps": 60}, "check")
print(report)
if sheet: shutil.copy(sheet, sys.argv[2])' /workspace/<repo>/animations/<name>/plan.json /workspace/<repo>/animations/<name>/sheet.png R15
python3 moonforge.py bake /workspace/<repo>/animations/<name>/plan.json --rig R15 --out /workspace/<repo>/animations/<name>/baked.json
```

Read the report (foot slide, balance, pops, clipping), look at `sheet.png`,
fix the plan, and repeat until the report is clean.

## The Studio gap: what reaches the game

Nothing in this sandbox can turn a compiled effect or a baked animation into
Roblox instances. That step is the Studio plugins'. Until Forge has file
exporters:

- Commit `vfx/<name>/{spec.json,compiled.json,sheet.png}` and
  `animations/<name>/{plan.json,baked.json,sheet.png}` on the feature branch.
- Don't reference them from `src/`. Leave the hook in code as a clearly named
  TODO with the effect or animation name, so the build stays clean.
- In the PR, list each one with its sheet and say: "import in Studio with the
  VFXForge or MoonForge plugin".
