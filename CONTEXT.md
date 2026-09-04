# benjisponge.com

Ben's personal site includes public writing and private, local-first tools.
This glossary records domain language whose meaning must stay stable across
the server, browser, and stores.

## Language

### Diary

**Entry Content**:
The business fields that determine replay equality for a Diary Entry.
_Avoid_: Payload, wire fields

**Reply Target**:
An optional, permalink-shaped Entry Reference carried by Entry Content. It is
a soft reference: its shape is validated, but persistence does not require a
matching Diary Entry to exist.
_Avoid_: Parent record, foreign key

**Composed Entry**:
A piece of Entry Content paired with the UTC second proposed to placement;
a device collision may re-anchor it to a later probed second.
_Avoid_: Wire entry, request entry

**Diary Entry**:
A piece of Entry Content paired with its placement-selected UTC second and
record key.
_Avoid_: Snapshot entry, store row

**Entry Key**:
The Eastern path-shaped record identifier selected by bounded collision
probing.
_Avoid_: Permanent id

**Saved Reference**:
The server-confirmed Entry Key and placement second returned when a Composed
Entry is saved.
_Avoid_: Saved entry, write response

**Entry Reference**:
A server-confirmed Entry Key safe to expose as a permalink.
_Avoid_: Predicted id, pending permalink

**Recovery Key**:
A device-only synthetic record key that preserves an unprojectable legacy
entry as failed text.
_Avoid_: Entry Key, permalink

**Diary Schema Epoch**:
The exact compatibility epoch shared by one diary build and its completed
server/device migrations. Mismatched clients pause sync until updated; Diary
Entries themselves carry no version.
_Avoid_: Per-entry schema marker

**Device Entry**:
A Diary Entry paired with device-local synchronization state, failure reason,
and enqueue order.
_Avoid_: Outbox row, queue item

**Sync State**:
The `pending`, `synced`, or `failed` status attached to a Device Entry.
_Avoid_: Delivery flag

### Fitness entry

**Workout Draft**:
The one mutable, device-local workout currently being composed. It may be
rebased when restored, but it has no server identity and never enters the
archive.
_Avoid_: Unsaved workout, pending workout

**Queued Workout**:
An immutable finalized workout stored in the device outbox with `pending`,
`failed`, or `saved` delivery state. Finalization creates it atomically with a
fresh Workout Draft before publication begins.
_Avoid_: Draft retry, uploaded workout

**Predicted Workout Link**:
The deterministic Eastern-path URL derived from a Queued Workout's frozen UTC
start. It is display-only and explicitly not live; rejection removes it.
_Avoid_: Permalink, hosted link

**Workout Receipt**:
The server-confirmed canonical location and exact share text attached to a
saved Queued Workout. It remains on the device until explicitly dismissed.
_Avoid_: Predicted link, approximate share

## Relationships

- A **Composed Entry** contains exactly one **Entry Content**
- **Entry Content** may carry one **Reply Target**
- Placement may re-anchor a **Composed Entry** before producing one **Diary Entry**
- A successfully placed **Diary Entry** has exactly one **Entry Key**
- Successfully saving a **Composed Entry** yields exactly one **Saved Reference**
- A **Device Entry** wraps exactly one **Diary Entry** and one **Sync State**
- A pending **Device Entry** has a predicted **Entry Key**
- A synced **Device Entry** exposes its key as an **Entry Reference**
- Only a synced **Device Entry** exposes its **Entry Reference** for selection
  as a **Reply Target**
- Sync proceeds only when the client and server agree on one **Diary Schema
  Epoch**; migrations put both stores in that epoch's canonical shape first
- A direct-sync token carries the exact **Diary Schema Epoch** admitted by the
  migration-owned table permission
- An unprojectable legacy entry is failed under a **Recovery Key**, which can
  never become an **Entry Reference**
- Finalizing a **Workout Draft** atomically creates one **Queued Workout** and
  one fresh Workout Draft
- A pending **Queued Workout** may show a **Predicted Workout Link**, but only a
  saved Queued Workout carries a **Workout Receipt**
- A failed **Queued Workout** can become the current Workout Draft only while
  that draft is empty

## Example dialogue

> **Dev:** "The pending Device Entry already has an Entry Key. Can I treat it
> as its final permalink?"
>
> **Domain expert:** "No. It becomes an Entry Reference only after a Saved
> Reference or snapshot confirms that it is synced."

## Flagged ambiguities

- "id" and "permalink" previously meant both a predicted local key and a
  server-confirmed reference — resolved: use **Entry Key** before confirmation
  and **Entry Reference** after it
- "wire entry", "snapshot entry", and "store entry" implied separate domain
  values — resolved: they are serialized or persisted forms of **Composed
  Entry** and **Diary Entry**
- "written at" previously meant both the client-proposed second and the
  collision-selected second — resolved: a **Composed Entry** proposes the
  starting second and a **Diary Entry** records the selected second
- "reply parent" implied a database relationship — resolved: a **Reply
  Target** is an optional soft reference to an Entry Reference, not a foreign
  key or existence constraint
