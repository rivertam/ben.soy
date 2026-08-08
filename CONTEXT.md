# benjisponge.com

Ben's personal site includes public writing and private, local-first tools.
This glossary records domain language whose meaning must stay stable across
the server, browser, and stores.

## Language

### Diary

**Entry Content**:
The business fields that determine replay equality for a Diary Entry.
_Avoid_: Payload, wire fields

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

**Device Entry**:
A Diary Entry paired with its semantic generation and device-local
synchronization state, failure reason, and enqueue order.
_Avoid_: Outbox row, queue item

**Sync State**:
The `pending`, `synced`, or `failed` status attached to a Device Entry.
_Avoid_: Delivery flag

## Relationships

- A **Composed Entry** contains exactly one **Entry Content**
- Placement may re-anchor a **Composed Entry** before producing one **Diary Entry**
- A successfully placed **Diary Entry** has exactly one **Entry Key**
- Successfully saving a **Composed Entry** yields exactly one **Saved Reference**
- A **Device Entry** wraps exactly one **Diary Entry** and one **Sync State**
- A pending **Device Entry** has a predicted **Entry Key**
- A synced **Device Entry** exposes its key as an **Entry Reference**
- An unprojectable legacy entry is failed under a **Recovery Key**, which can
  never become an **Entry Reference**

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
