# Design doc template: `design/<slug>.md`

Keep it to about a page. The Programmer builds from this. You, the owner,
playtest against its last section.

```markdown
# <Feature name>

## Pitch
One or two sentences: what the player does and why it's fun.

## Rules
- Numbered, testable statements. "Coins respawn 30 s after pickup", not
  "coins come back after a while".

## Numbers
| Tunable | Value | Why |
|---|---|---|
| CoinValue | 5 | … |

Every row becomes a field in `src/shared/Config.luau`, with the same name.

## Player-facing text
Exact strings for UI, notifications and errors.

## Edge cases
- Leaving mid-action, two players at once, a full inventory, a DataStore
  outage, exploit attempts (what the server must reject).

## Out of scope
What this deliberately does not do.

## Playtest
Steps the owner follows in Studio or the test place, and what they should
see at each step.
```
