# Luau conventions for Opus Systems games

## Layout (Rojo `default.project.json`)

| On disk | In the DataModel | Holds |
|---|---|---|
| `src/server/` | `ServerScriptService.Server` | `*.server.luau` entry points, server modules |
| `src/client/` | `StarterPlayer.StarterPlayerScripts.Client` | `*.client.luau` entry points, UI controllers |
| `src/shared/` | `ReplicatedStorage.Shared` | `Config.luau`, types, pure logic both sides use |
| `design/` | nothing (not synced) | design docs |
| `types/globalTypes.d.luau` | nothing | Roblox API types for `luau-lsp`, pinned |
| `roblox.yml` | nothing | selene's Roblox standard library, pinned |

A file named `*.server.luau` is a `Script`, `*.client.luau` is a
`LocalScript`, and anything else is a `ModuleScript`. A folder with
`init.luau` is a module named after the folder.

## Style

- Tabs, 110 columns, double quotes, and parentheses on every call
  (`stylua.toml`). Run `stylua src` rather than formatting by hand.
- Services are fetched at the top of the file:
  `local Players = game:GetService("Players")`.
- Module shape: `local M = {}` … `return M`, with no top-level side effects
  in a module.
- Types: `export type` in the module that owns the data, and annotated
  function parameters and returns.

## Networking

- One `Remotes` folder in `ReplicatedStorage`, created by the server at
  startup, with one `RemoteEvent` per intent. Names are verbs:
  `RequestPurchase`, `ClaimReward`.
- Every server handler starts by validating `typeof` for each argument,
  ranges, ownership, and a per-player cooldown. It returns early on bad
  input without erroring.

## Persistence

- One `DataStore` per game, with keys `player_<UserId>`. The whole profile is
  a table with a `version` field for migrations.
- Loading: `pcall` + retry. If loading ultimately fails, kick the player with
  a friendly message instead of letting them play on a blank profile, which
  would then save over the real one.

## Checks

`scripts/check.sh` runs exactly what CI runs. A PR is not ready until it
prints `all checks passed`.
