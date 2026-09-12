//! Recent choices survive edits/deletes without retaining old entry content.
use crate::{Db, entry::ComposedEntry};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, SurrealValue)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub emoji: String,
    pub used_at_ms: i64,
}

pub async fn remember(db: &Db, usage: &Usage) -> Result<(), String> {
    if !crate::entry::valid_emoji(&usage.emoji) || usage.used_at_ms < 0 {
        return Err("invalid emoji usage".into());
    }
    db.query(
        "UPSERT type::record('diary_emojis', $emoji)
         SET emoji = $emoji, used_at_ms = math::max([used_at_ms ?? 0, $at])",
    )
    .bind(("emoji", usage.emoji.clone()))
    .bind(("at", usage.used_at_ms))
    .await
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn remember_entry(db: &Db, entry: &ComposedEntry) -> Result<(), String> {
    if let Some(emoji) = &entry.emoji {
        remember(
            db,
            &Usage {
                emoji: emoji.clone(),
                used_at_ms: entry
                    .saved_at_ms
                    .unwrap_or(entry.written_at.saturating_mul(1000)),
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn recent(db: &Db) -> Result<Vec<Usage>, String> {
    let mut result = db.query(
        "SELECT emoji, used_at_ms FROM diary_emojis ORDER BY used_at_ms DESC, emoji ASC LIMIT 10",
    ).await.map_err(|e| e.to_string())?.check().map_err(|e| e.to_string())?;
    result.take(0).map_err(|e| e.to_string())
}

pub async fn choices(db: &Db) -> Result<Vec<String>, String> {
    Ok(recent(db)
        .await?
        .into_iter()
        .map(|usage| usage.emoji)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn choices_are_distinct_capped_and_ordered_by_save_not_backdate() {
        let db = crate::outbox::open("mem://").await.unwrap();
        for (index, emoji) in [
            "🙂", "😊", "🥰", "😌", "😂", "🥹", "😢", "😔", "😟", "😤", "😡",
        ]
        .iter()
        .enumerate()
        {
            remember(
                &db,
                &Usage {
                    emoji: emoji.to_string(),
                    used_at_ms: index as i64,
                },
            )
            .await
            .unwrap();
        }
        let mut reused = ComposedEntry::new(100, "");
        reused.emoji = Some("🙂".into());
        reused.saved_at_ms = Some(1000);
        reused.occurred_at = Some(1);
        remember_entry(&db, &reused).await.unwrap();
        let recent = choices(&db).await.unwrap();
        assert_eq!(recent.len(), 10);
        assert_eq!(recent[0], "🙂");
        assert_eq!(recent[1], "😡");
        assert!(!recent.contains(&"😊".to_string()));
    }
    #[tokio::test]
    async fn recency_is_monotonic_and_survives_editing_and_deleting() {
        let db = crate::outbox::open("mem://").await.unwrap();
        let mut entry = ComposedEntry::new(1_700_000_000, "");
        entry.emoji = Some("😌".into());
        entry.saved_at_ms = Some(1000);
        let row = crate::outbox::enqueue(&db, entry, 1000).await.unwrap();
        crate::outbox::revise(
            &db,
            &row.id,
            "".into(),
            Some("🙂".into()),
            row.written_at,
            false,
            2000,
        )
        .await
        .unwrap();
        crate::outbox::revise(&db, &row.id, "".into(), None, row.written_at, true, 3000)
            .await
            .unwrap();
        remember(
            &db,
            &Usage {
                emoji: "🙂".into(),
                used_at_ms: 500,
            },
        )
        .await
        .unwrap();
        assert_eq!(choices(&db).await.unwrap(), ["🙂", "😌"]);
    }
}
