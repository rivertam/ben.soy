---
name: audit-muscle-weights
description: Audit the lifting archive for exercises lacking muscle-weight data, research reasonable ratios, and enter them via the seed table or the live admin form. Use after new exercises appear in production, or to check tag/weight drift.
---

# Audit muscle weights

Every exercise should carry weighted muscle connections (`exercise_muscles`
rows, ratio 1..=100) so its sets earn muscle credit. Read the "Muscle
weights" section of `docs/fitness.md` before changing anything.

## 1. Find gaps

- Exercise list: `curl -s https://benjisponge.com/api/fitness/facets | jq -r '.exercises[].value'`
- Compare against `SEED_WEIGHTS` in `src/app/interests/lifting/muscle_seed.rs`.
- An exercise absent from the seed table still gets tag-derived defaults at
  seeding time, and one that already has rows is fine — so the real check
  is the live page: `https://benjisponge.com/lifting/exercise/<urlencoded name>`
  showing "no stored weights" or an obviously wrong split is a gap.
- Pure cardio (Running, Rowing, Stair Stepper) is deliberately weightless —
  not a gap.

## 2. Research ratios

Use the granular vocabulary from `src/app/interests/lifting/muscle_taxonomy.rs`
(28 ids: anterior/lateral/posterior-delts, upper/mid/lower-traps, lats,
rhomboids, spinal-erectors, upper/mid/lower-chest, serratus-anterior, biceps,
brachialis, triceps, forearm-flexors/-extensors, abs, obliques, hip-flexors,
quads, hamstrings, adductors, glute-max, glute-med, gastrocnemius, soleus).

Rubric: prime movers 75–100 (the movement's main target at 100), strong
synergists 40–70, stabilizers 10–35, omit anything below 10. When in doubt
fan out research subagents per batch of exercises and have a second pass
skeptically verify anatomical sense (grip credit modest, subdivisions match
the movement angle).

## 3. Apply

Two paths, different trade-offs:

- **Seed table (preferred for batches):** add entries to `SEED_WEIGHTS`,
  run `just check` (the seed tests validate ids/ranges/duplicates), and
  deploy. Seeds only apply to exercises with zero stored rows — to make a
  changed seed take effect for an already-seeded exercise you must either
  edit it in the admin form or delete its rows in the database first.
- **Admin form (immediate, authoritative):** edit at
  `/lifting/exercise/<name>` while signed in as the admin. Saves write
  `source='admin'` rows that permanently block reseeding for that exercise.

## 4. Drift check

Weights and coarse muscle tags can drift: the `/lifting/log` muscle facet
matches tags (13 coarse values), not weights. For any exercise you touched,
confirm its `muscle` tags (`taxonomy::exercise_tags`) still name the coarse
group of every significant weighted muscle (`muscle_taxonomy::coarse_tag_for`),
and fix the taxonomy rules if not.
