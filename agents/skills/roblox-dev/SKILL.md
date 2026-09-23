---
name: roblox-dev
description: Build features for an Opus Systems Roblox game as a three-agent team — design doc, Luau implementation in a Rojo project, checks, PR, and a publish to the game's test place. Use for any Roblox game work in an Opus-Systems-OS game repo: new mechanics, systems, UI, balancing, bug fixes, or publishing a build to test.
---

# Roblox game development (Opus Systems)

The game lives in a private GitHub repo as a **Rojo project**: Luau files on
disk, synced into Roblox Studio by the owner and built into a place file by
`rojo build`. There is no Studio in this sandbox and nothing here can press
Play. The owner playtests. Your checks are formatting, linting, full type
checking against the Roblox API, and a clean build.

## The team

The session runs as `roblox-director`. It delegates to the two specialists
on its roster. All three share this sandbox and its filesystem.

| Agent | Owns | Never |
|---|---|---|
| **Director** | Scoping the task, delegating, reviewing both outputs against each other, the branch, the PR, the test publish, the final report | Writes large amounts of game code itself |
| **Game Designer** | `design/<feature>.md`: the mechanic, every number (in a table), player-facing text, edge cases, what "done" feels like, and what to playtest | Writes Luau |
| **Programmer** | Luau under `src/`, tunables in `src/shared/Config.luau`, until `scripts/check.sh` passes | Changes a number the design fixed, without saying so in the PR |

The order is: design → review → implement → checks → review → PR → publish
to test → report. The Director sends the design doc's path to the Programmer
rather than retelling it. If the design and the implementation disagree, the
design doc is updated or the Programmer's deviation is written into the PR,
never left silent.

## Non-negotiables

- **The server is authoritative.** Clients request and the server decides.
  Every `RemoteEvent` and `RemoteFunction` handler validates its arguments'
  types, ranges and rate on the server. Nothing trusts a client-sent amount,
  position or price.
- **DataStores:** only `UpdateAsync` for anything read-modify-write, always
  inside `pcall`, with retries and backoff, and a save on `PlayerRemoving` and
  `game:BindToClose`. Never `SetAsync` over a player's whole save.
- **`--!strict` at the top of every file.** No `any` without a comment
  saying why.
- **Current APIs only.** Use `task.wait`/`task.spawn`/`task.delay`, never
  `wait`/`spawn`/`delay`. Use `:GetService`, never `game.Workspace`. Use
  `Instance.new(class)` and then set `Parent` last.
- **Tunables live in `src/shared/Config.luau`**, so a balance change is a
  one-line diff.
- **Never push to `main`** and never force-push. Branch `feature/<slug>`
  and open a PR.
- **Publish only with `scripts/publish-test.sh`.** It targets the test
  experience alone, and the key it uses cannot reach the live game.
  Publishing the live game is the owner's.
- **No secrets in the repo**, and no HTTP calls to anything except Roblox's
  own services from game code without the owner asking.

## Workflow

### 0. Tools, once per session

```sh
bash <skill dir>/scripts/setup.sh       # rojo, selene, stylua, luau-lsp → ~/.local/bin (~2 s)
export PATH="$HOME/.local/bin:$PATH"
```

The game repo is mounted under `/workspace/<repo>`. If it is not, clone it
with `gh repo clone Opus-Systems-OS/<repo>`; `GH_TOKEN` is preset.

### 1. Branch

```sh
cd /workspace/<repo> && git checkout -b feature/<slug>
```

### 2. Design (Designer)

Write `design/<slug>.md` from `references/design-doc.md`. Keep it short
enough to read in two minutes. Put numbers in a table, not in prose.

### 3. Implement (Programmer)

The layout is in `references/luau-conventions.md`. Iterate until the checks
are clean:

```sh
bash <skill dir>/scripts/check.sh       # stylua --check, selene, luau-lsp analyze, rojo build
```

Fix formatting with `stylua src`. Never silence a selene or luau-lsp finding
without a comment saying why it is wrong.

### 4. Push and PR (Director)

`git push` cannot work in this sandbox: the token is substituted only into
request headers. Commit locally, then:

```sh
node <skill dir>/scripts/push-tree.mjs Opus-Systems-OS/<repo> feature/<slug> -m "<summary>"
gh pr create -R Opus-Systems-OS/<repo> --head feature/<slug> --title "…" --body-file /tmp/pr.md
```

The PR body contains the design doc's path, what was built, any deviation
from the design, how to playtest (steps, and what you should see), and the
check output's last line.

### 5. Publish to test (Director)

```sh
bash <skill dir>/scripts/publish-test.sh    # from the repo root; builds, then publishes
```

It prints the version number and the play link. A failure is reported
as-is; don't retry more than once.

### 6. Report

End with a short report to whoever started the session:
- the PR URL;
- the test version and link;
- what to playtest;
- anything left undone.

## Cost

One budget covers all three agents. Keep delegation messages short and point
at files instead of pasting them. Don't have two agents read the same large
file. When the budget is getting close, stop, push what is there as a
**draft** PR, and report.
