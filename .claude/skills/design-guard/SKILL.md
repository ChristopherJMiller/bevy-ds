---
name: design-guard
description: Pre-build design gate for the Kill the Serpent game. Use BEFORE implementing any design-bearing issue or feature: it loads the design pillars (docs/design/PILLARS.md) and the relevant GitHub issues (#17 hub + the target epic/spike), checks the planned work aligns with the three pillars and the holistic "few tools, many combos" principle, and STOPS to surface any unresolved Open question the work depends on before code is written. Use when starting a spike/epic, picking up an issue, or whenever about to build a feature that touches game design.
---

# design-guard

The issues are the design record; the pillars are the filter. This skill makes
"refer back to the design before building" mechanical, so we never drift or
silently resolve an open question by accident.

## When to use

- **Use** before implementing any issue in the Kill the Serpent board (#17–32),
  or any feature that touches game design / feel / mechanics.
- **Skip** for pure-mechanical work with no design content (a refactor, a build
  fix, a typo). If unsure, run it — it's cheap.

## Steps

### 1. Identify the target
Which issue does this work implement? If the work maps to **no** issue, that's a
flag — either it's out of scope or an issue is missing. Surface that before
proceeding (you may need `design-sync` to file it first).

### 2. Load the design context
- Read `docs/design/PILLARS.md` (the three pillars + the holistic principle).
- `gh issue view 17 --repo ChristopherJMiller/kts-nds` (the hub: locked control
  model + the repo-wide Open-questions register).
- `gh issue view <target> --repo ChristopherJMiller/kts-nds` (the specific
  epic/spike, including its own `## Locked` / `## Open questions`).

### 3. Pillar + holistic check
For the planned work, answer briefly:
- **Which pillar(s) does this serve?** (the pen is the power / pressure is the
  puzzle / few tools, many combos). It must serve at least one.
- **Does it fight any pillar?** (e.g. demotes the stylus; makes the player a
  sitting duck; adds a segmented "mode" that doesn't interact).
- **Holistic test:** does this mechanic *multiply* against systems we already
  have, or does it stand alone as its own mode? Prefer multiply. If it must
  stand alone, say so out loud and justify it.

### 4. Open-question gate (the important one)
List the target issue's `## Open questions` **and** any relevant entry in #17's
register. If the planned work **depends on** an unresolved question:
- **STOP.** Do not silently pick an answer.
- Present the open question to the user and get a decision — or an explicit
  "defer and build the smallest thing that doesn't bake in either answer."
- Once decided, record it with `design-sync` before continuing.

### 5. Verdict
Emit a short go / no-go:
- Pillars served: …
- Holistic fit: multiplies / stands-alone (justified?)
- Blocking open questions: none / [list] → resolve first

If **go**, build. If **blocked**, resolve the question (then `design-sync` it)
before writing code.
