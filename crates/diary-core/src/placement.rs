//! Collision placement shared by the server store and the device store.
//!
//! The adapters supply key projection plus read/create operations. This
//! module owns the check-create-recheck walk and content equality, so adding
//! a business field changes neither persistence implementation's control
//! flow. Futures deliberately have no `Send` bounds: the wasm adapters wrap
//! browser and IndexedDB work that is `!Send`.

use std::future::Future;

use crate::entry::{ComposedEntry, DiaryEntry};

/// Same-second key collisions probe forward at most this many seconds.
pub const COLLISION_PROBES: i64 = 5;

/// What an adapter knows about one occupied candidate key. Newer local row
/// generations are deliberately opaque to an older worker: they block the
/// key but can never compare equal through a truncated content projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Occupant {
    Known(DiaryEntry),
    Opaque,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    Placed(DiaryEntry),
    Deduped(DiaryEntry),
    Exhausted,
}

impl Placement {
    pub fn entry(self) -> Option<DiaryEntry> {
        match self {
            Self::Placed(entry) | Self::Deduped(entry) => Some(entry),
            Self::Exhausted => None,
        }
    }

    pub fn deduped(&self) -> bool {
        matches!(self, Self::Deduped(_))
    }
}

/// Place one composed entry under the first available projected second.
///
/// A row with identical [`crate::entry::EntryContent`] is a replay even at
/// a later probe. A failed create is always re-read: a racing writer may
/// have installed either the replay twin (dedupe) or a different occupant
/// (continue probing). Only a create error followed by no row escapes.
pub async fn place<E, Key, Read, ReadFuture, Create, CreateFuture>(
    requested: &ComposedEntry,
    mut key_for: Key,
    mut read: Read,
    mut create: Create,
) -> Result<Placement, E>
where
    Key: FnMut(i64) -> Result<String, E>,
    Read: FnMut(String) -> ReadFuture,
    ReadFuture: Future<Output = Result<Option<Occupant>, E>>,
    Create: FnMut(DiaryEntry) -> CreateFuture,
    CreateFuture: Future<Output = Result<(), E>>,
{
    for offset in 0..COLLISION_PROBES {
        let written_at = requested.written_at + offset;
        let id = key_for(written_at)?;
        match read(id.clone()).await? {
            Some(Occupant::Known(existing)) if existing.has_content(&requested.content) => {
                return Ok(Placement::Deduped(existing));
            }
            Some(_) => continue,
            None => {}
        }

        let candidate = DiaryEntry::new(id.clone(), requested.placed_at(written_at));
        match create(candidate.clone()).await {
            Ok(()) => return Ok(Placement::Placed(candidate)),
            Err(create_error) => match read(id).await? {
                Some(Occupant::Known(existing)) if existing.has_content(&requested.content) => {
                    return Ok(Placement::Deduped(existing));
                }
                Some(_) => continue,
                None => return Err(create_error),
            },
        }
    }
    Ok(Placement::Exhausted)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[tokio::test]
    async fn killer_replay_walk_dedupes_at_the_bumped_probe() {
        let rows = RefCell::new(HashMap::<String, DiaryEntry>::new());
        rows.borrow_mut().insert(
            "100".to_string(),
            DiaryEntry::from_parts("100", 100, "occupant"),
        );
        rows.borrow_mut().insert(
            "101".to_string(),
            DiaryEntry::from_parts("101", 101, "mine"),
        );
        let placement = place(
            &ComposedEntry::new(100, "mine"),
            |epoch| Ok::<_, ()>(epoch.to_string()),
            |id| std::future::ready(Ok(rows.borrow().get(&id).cloned().map(Occupant::Known))),
            |_| std::future::ready(Ok(())),
        )
        .await
        .unwrap();
        assert!(matches!(placement, Placement::Deduped(entry) if entry.id == "101"));
    }

    #[tokio::test]
    async fn a_lost_create_race_is_re_read_as_a_dedupe() {
        let reads = RefCell::new(0);
        let candidate = DiaryEntry::from_parts("100", 100, "mine");
        let placement = place(
            &candidate.composed,
            |_| Ok::<_, &'static str>("100".to_string()),
            |_| {
                let mut count = reads.borrow_mut();
                *count += 1;
                std::future::ready(Ok(if *count == 1 {
                    None
                } else {
                    Some(Occupant::Known(candidate.clone()))
                }))
            },
            |_| std::future::ready(Err("collision")),
        )
        .await
        .unwrap();
        assert!(matches!(placement, Placement::Deduped(_)));
    }
}
