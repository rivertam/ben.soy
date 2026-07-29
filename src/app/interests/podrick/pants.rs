//! Job 2: sync and respond to Pants Off messages.
//!
//! Source history and the live tail deliberately use the same backwards
//! Discord message walker. The one-time seed stores source facts without
//! producing side effects; only messages beyond its snowflake head can create
//! infarction posts or worm reactions. Every side effect is claimed in the
//! database before Discord is called and retried until confirmed.

use std::collections::BTreeMap;
use std::time::Duration;

use benjisponge::data::{
    Db,
    podrick_models::{
        KWERM_EMOJI, PantsDay, PantsMomentKind, PantsSlot, PodrickPantsAction, PodrickPantsMessage,
        aggregate_pants_messages, classify_pants_message, classify_pants_time, pants_participant,
    },
};
use jiff::Timestamp;

use crate::db;
use crate::discord::{ChannelMessage, Discord, DiscordError};

const DISCORD_PAGE_SIZE: usize = 100;
const BACKFILL_PAGES_PER_TICK: usize = 5;
const MAX_ACTIONS_PER_TICK: usize = 24;

const ACTION_POST: &str = "post";
const ACTION_REACTION: &str = "reaction";
const REASON_INFARCTION: &str = "infarction";
const REASON_KWERM_AM: &str = "kwerm_am";
const REASON_KWERM_PM: &str = "kwerm_pm";
const REASON_ASYNKWERM: &str = "asynkwerm";

#[derive(Debug)]
pub enum PantsError {
    Database(String),
    Discord(DiscordError),
    InvalidDiscordMessage(String),
    SourceChannelChanged { stored: String, configured: String },
}

impl std::fmt::Display for PantsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PantsError::Database(error) => write!(f, "database: {error}"),
            PantsError::Discord(error) => write!(f, "{error}"),
            PantsError::InvalidDiscordMessage(error) => {
                write!(f, "invalid Discord message: {error}")
            }
            PantsError::SourceChannelChanged { stored, configured } => write!(
                f,
                "Pants Off source channel changed from {stored} to {configured}; \
                 restore the original channel or perform an intentional data migration"
            ),
        }
    }
}

impl std::error::Error for PantsError {}

/// What one Pants Off pass did. IDs and action labels are intended for the
/// worker's structured logs, not the web page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PantsTickReport {
    pub history_scanned: usize,
    pub history_stored: usize,
    pub history_complete: bool,
    pub live_scanned: usize,
    pub live_stored: usize,
    pub infarctions: Vec<String>,
    pub worms: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub retry_after: Option<Duration>,
}

impl PantsTickReport {
    pub fn is_quiet(&self) -> bool {
        self.history_scanned == 0
            && !self.history_complete
            && self.live_stored == 0
            && self.infarctions.is_empty()
            && self.worms.is_empty()
            && self.skipped.is_empty()
            && self.failed.is_empty()
    }

    fn record_retryable(&mut self, label: &str, error: &DiscordError) {
        if let Some(after) = error.retry_after() {
            self.retry_after = Some(self.retry_after.map_or(after, |current| current.max(after)));
        }
        self.failed.push(format!("{label}: {error}"));
    }
}

pub struct PantsWorker {
    pub discord: Discord,
    pub channel_id: String,
    pub infarctions_channel_id: String,
    pub dry_run: bool,
}

impl PantsWorker {
    pub async fn tick(&self, db: &Db, now: i64) -> Result<PantsTickReport, PantsError> {
        let mut report = PantsTickReport::default();
        let result = self.tick_inner(db, now, &mut report).await;
        match result {
            Ok(()) => Ok(report),
            Err(PantsError::Discord(error)) if error.is_retryable() => {
                report.record_retryable("Discord request", &error);
                Ok(report)
            }
            Err(error) => Err(error),
        }
    }

    async fn tick_inner(
        &self,
        db: &Db,
        now: i64,
        report: &mut PantsTickReport,
    ) -> Result<(), PantsError> {
        let source_bound = self.source_channel_is_bound(db).await?;

        let Some(cursor) = db::meta(db, db::PANTS_CURSOR).await.map_err(database)? else {
            self.backfill(db, report, source_bound).await?;
            // A seed-completing pass still returns here. Reconciliation on
            // the same (or immediately following) pass must not react to
            // historical kwerms; the immutable backfill head below is the
            // action floor that enforces that on later passes.
            return Ok(());
        };

        let live = self.messages_after(&cursor, report).await?;
        if !source_bound && !self.dry_run {
            self.bind_source_channel(db).await?;
        }
        let mut recognized = Vec::new();
        for source in &live {
            if let Some(message) = self.recognized_message(source)? {
                if !self.dry_run {
                    db::store_pants_message(db, &message)
                        .await
                        .map_err(database)?;
                }
                recognized.push(message);
            }
        }
        report.live_stored += recognized.len();

        self.claim_infarctions(db, now, &recognized, report).await?;

        let floor = db::meta(db, db::PANTS_BACKFILL_HEAD)
            .await
            .map_err(database)?
            .unwrap_or_else(|| cursor.clone());
        self.claim_worms(db, now, &floor, &recognized, report)
            .await?;

        // Claims are durable before the cursor advances: a crash after this
        // point can only retry an idempotent action, never lose one.
        if !self.dry_run
            && let Some(newest) = live
                .iter()
                .max_by(|left, right| snowflake_cmp(&left.id, &right.id))
        {
            db::set_meta(db, db::PANTS_CURSOR, &newest.id)
                .await
                .map_err(database)?;
        }

        self.deliver_actions(db, now, report).await
    }

    async fn source_channel_is_bound(&self, db: &Db) -> Result<bool, PantsError> {
        let Some(stored) = db::meta(db, db::PANTS_SOURCE_CHANNEL)
            .await
            .map_err(database)?
        else {
            return Ok(false);
        };
        if stored == self.channel_id {
            Ok(true)
        } else {
            Err(PantsError::SourceChannelChanged {
                stored,
                configured: self.channel_id.clone(),
            })
        }
    }

    async fn bind_source_channel(&self, db: &Db) -> Result<(), PantsError> {
        db::init_meta(db, db::PANTS_SOURCE_CHANNEL, &self.channel_id)
            .await
            .map_err(database)?;
        // Verify the row exists as well as checking the returned winner.
        // Binding happens only after Discord accepted a source-channel read,
        // so a typo that returns 404 cannot become an immutable configuration.
        let stored = db::meta(db, db::PANTS_SOURCE_CHANNEL)
            .await
            .map_err(database)?
            .ok_or_else(|| {
                PantsError::Database(
                    "Pants Off source-channel binding was not persisted".to_string(),
                )
            })?;
        if stored == self.channel_id {
            Ok(())
        } else {
            Err(PantsError::SourceChannelChanged {
                stored,
                configured: self.channel_id.clone(),
            })
        }
    }

    /// Resume (or begin) the one-time newest-to-oldest source history walk.
    /// Pages are checkpointed independently, so a 429 or restart loses at most
    /// one page of work and replaying that page is an idempotent UPSERT.
    async fn backfill(
        &self,
        db: &Db,
        report: &mut PantsTickReport,
        mut source_bound: bool,
    ) -> Result<(), PantsError> {
        let mut head = db::meta(db, db::PANTS_BACKFILL_HEAD)
            .await
            .map_err(database)?;
        let mut before = db::meta(db, db::PANTS_BACKFILL_BEFORE)
            .await
            .map_err(database)?
            .or_else(|| head.clone());

        for _ in 0..BACKFILL_PAGES_PER_TICK {
            let page = self
                .discord
                .messages(&self.channel_id, before.as_deref())
                .await
                .map_err(PantsError::Discord)?;
            if !source_bound && !self.dry_run {
                self.bind_source_channel(db).await?;
                source_bound = true;
            }
            report.history_scanned += page.len();

            if page.is_empty() {
                let boundary = head.clone().unwrap_or_else(|| "0".to_string());
                if !self.dry_run {
                    if head.is_none() {
                        db::set_meta(db, db::PANTS_BACKFILL_HEAD, &boundary)
                            .await
                            .map_err(database)?;
                    }
                    db::set_meta(db, db::PANTS_CURSOR, &boundary)
                        .await
                        .map_err(database)?;
                }
                report.history_complete = true;
                return Ok(());
            }

            validate_page(&page)?;
            let newest = page
                .iter()
                .max_by(|left, right| snowflake_cmp(&left.id, &right.id))
                .expect("nonempty page");
            let oldest = page
                .iter()
                .min_by(|left, right| snowflake_cmp(&left.id, &right.id))
                .expect("nonempty page");
            if head.is_none() {
                head = Some(newest.id.clone());
            }

            let mut stored = 0;
            for source in &page {
                if let Some(message) = self.recognized_message(source)? {
                    if !self.dry_run {
                        db::store_pants_message(db, &message)
                            .await
                            .map_err(database)?;
                    }
                    stored += 1;
                }
            }
            report.history_stored += stored;

            if !self.dry_run {
                let boundary = head.as_deref().expect("page established head");
                db::set_meta(db, db::PANTS_BACKFILL_HEAD, boundary)
                    .await
                    .map_err(database)?;
            }

            if page.len() < DISCORD_PAGE_SIZE {
                if !self.dry_run {
                    db::set_meta(
                        db,
                        db::PANTS_CURSOR,
                        head.as_deref().expect("page established head"),
                    )
                    .await
                    .map_err(database)?;
                }
                report.history_complete = true;
                return Ok(());
            }

            before = Some(oldest.id.clone());
            if !self.dry_run {
                db::set_meta(
                    db,
                    db::PANTS_BACKFILL_BEFORE,
                    before.as_deref().expect("just set"),
                )
                .await
                .map_err(database)?;
            }
        }
        Ok(())
    }

    /// Read every source message newer than `cursor`, even if more than one
    /// Discord page accumulated between polls. The API returns newest first,
    /// so this walks backwards until it crosses the numeric snowflake cursor,
    /// then sorts the result oldest first for deterministic processing.
    async fn messages_after(
        &self,
        cursor: &str,
        report: &mut PantsTickReport,
    ) -> Result<Vec<ChannelMessage>, PantsError> {
        let cursor = snowflake(cursor)?;
        let mut before = None;
        let mut messages = Vec::new();
        loop {
            let page = self
                .discord
                .messages(&self.channel_id, before.as_deref())
                .await
                .map_err(PantsError::Discord)?;
            report.live_scanned += page.len();
            if page.is_empty() {
                break;
            }
            validate_page(&page)?;
            let oldest = page
                .iter()
                .min_by(|left, right| snowflake_cmp(&left.id, &right.id))
                .expect("nonempty page");
            let mut crossed_cursor = false;
            for message in &page {
                if snowflake(&message.id)? > cursor {
                    messages.push(message.clone());
                } else {
                    crossed_cursor = true;
                }
            }
            if crossed_cursor || page.len() < DISCORD_PAGE_SIZE {
                break;
            }
            before = Some(oldest.id.clone());
        }
        messages.sort_by(|left, right| snowflake_cmp(&left.id, &right.id));
        messages.dedup_by(|left, right| left.id == right.id);
        Ok(messages)
    }

    fn recognized_message(
        &self,
        source: &ChannelMessage,
    ) -> Result<Option<PodrickPantsMessage>, PantsError> {
        if pants_participant(&source.author.id).is_none() {
            return Ok(None);
        }
        let posted_at = channel_message_second(source)?;
        Ok(Some(PodrickPantsMessage {
            id: source.id.clone(),
            message_id: source.id.clone(),
            channel_id: self.channel_id.clone(),
            author_id: source.author.id.clone(),
            posted_at,
        }))
    }

    async fn claim_infarctions(
        &self,
        db: &Db,
        now: i64,
        messages: &[PodrickPantsMessage],
        report: &mut PantsTickReport,
    ) -> Result<(), PantsError> {
        for message in messages {
            let Some(classified) = classify_pants_message(message) else {
                continue;
            };
            if classified.kind != PantsMomentKind::Infarction {
                continue;
            }
            let label = format!(
                "{} at {}",
                classified.participant.display_name,
                classified.clock_label()
            );
            if self.dry_run {
                report.infarctions.push(format!("would post: {label}"));
                continue;
            }
            let clock = classified.clock_label();
            let action = PodrickPantsAction {
                id: format!("infarction:{}", classified.message_id),
                action_kind: ACTION_POST.to_string(),
                reason: REASON_INFARCTION.to_string(),
                target_channel_id: self.infarctions_channel_id.clone(),
                source_message_id: classified.message_id.clone(),
                content: format!(
                    "{} had a pants-off infarction at {}.",
                    classified.participant.display_name, clock
                ),
                claimed_at: now,
                completed_at: None,
                output_message_id: None,
                attempts: 0,
            };
            let _ = db::claim_pants_action(db, &action)
                .await
                .map_err(database)?;
        }
        Ok(())
    }

    async fn claim_worms(
        &self,
        db: &Db,
        now: i64,
        action_floor: &str,
        new_messages: &[PodrickPantsMessage],
        report: &mut PantsTickReport,
    ) -> Result<(), PantsError> {
        let mut messages = db::pants_messages(db).await.map_err(database)?;
        messages.extend_from_slice(new_messages);
        let mut by_id = BTreeMap::new();
        for message in messages {
            by_id.insert(message.message_id.clone(), message);
        }
        let messages: Vec<_> = by_id.into_values().collect();
        let floor = snowflake(action_floor)?;

        for day in aggregate_pants_messages(&messages) {
            if let Some(messages) = day.kwerm_messages(PantsSlot::Am) {
                self.claim_worm_set(db, now, &day, REASON_KWERM_AM, &messages, floor, report)
                    .await?;
            }
            if let Some(messages) = day.kwerm_messages(PantsSlot::Pm) {
                self.claim_worm_set(db, now, &day, REASON_KWERM_PM, &messages, floor, report)
                    .await?;
            }
            if day.asynkwerm
                && asynkwerm_is_final(&day, now)
                && let Some(messages) = day.asynkwerm_messages()
            {
                self.claim_worm_set(db, now, &day, REASON_ASYNKWERM, &messages, floor, report)
                    .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn claim_worm_set(
        &self,
        db: &Db,
        now: i64,
        day: &PantsDay,
        reason: &str,
        messages: &[&benjisponge::data::podrick_models::ClassifiedPantsMessage; 3],
        action_floor: u64,
        report: &mut PantsTickReport,
    ) -> Result<(), PantsError> {
        let is_live = messages
            .iter()
            .any(|message| snowflake(&message.message_id).is_ok_and(|id| id > action_floor));
        if !is_live {
            return Ok(());
        }
        let label = format!("{reason}:{}", day.date_label());
        if self.dry_run {
            report.worms.push(format!("would react: {label}"));
            return Ok(());
        }
        for message in messages {
            let action = PodrickPantsAction {
                id: format!("{label}:{}", message.message_id),
                action_kind: ACTION_REACTION.to_string(),
                reason: reason.to_string(),
                target_channel_id: self.channel_id.clone(),
                source_message_id: message.message_id.clone(),
                content: KWERM_EMOJI.to_string(),
                claimed_at: now,
                completed_at: None,
                output_message_id: None,
                attempts: 0,
            };
            let _ = db::claim_pants_action(db, &action)
                .await
                .map_err(database)?;
        }
        Ok(())
    }

    async fn deliver_actions(
        &self,
        db: &Db,
        now: i64,
        report: &mut PantsTickReport,
    ) -> Result<(), PantsError> {
        for action in db::uncompleted_pants_actions(db, MAX_ACTIONS_PER_TICK)
            .await
            .map_err(database)?
        {
            if self.dry_run {
                report
                    .failed
                    .push(format!("would retry pending action {}", action.id));
                continue;
            }
            let attempt: Result<Option<String>, PantsError> = match action.action_kind.as_str() {
                ACTION_POST => self.deliver_post(&action).await,
                ACTION_REACTION => self
                    .discord
                    .add_own_reaction(
                        &action.target_channel_id,
                        &action.source_message_id,
                        &action.content,
                    )
                    .await
                    .map(|()| None)
                    .map_err(PantsError::Discord),
                other => {
                    return Err(PantsError::Database(format!(
                        "unknown pants action kind {other:?} in {}",
                        action.id
                    )));
                }
            };
            match attempt {
                Ok(output_message_id) => {
                    db::mark_pants_action_completed(
                        db,
                        &action.id,
                        now,
                        output_message_id.as_deref(),
                    )
                    .await
                    .map_err(database)?;
                    match action.reason.as_str() {
                        REASON_INFARCTION => report.infarctions.push(action.source_message_id),
                        _ => report
                            .worms
                            .push(format!("{}:{}", action.reason, action.source_message_id)),
                    }
                }
                Err(error) => {
                    db::record_pants_action_attempt(db, &action.id)
                        .await
                        .map_err(database)?;
                    if action.action_kind == ACTION_REACTION
                        && matches!(error, PantsError::Discord(DiscordError::NotFound))
                    {
                        // The historical source fact remains useful even if
                        // its Discord message was deleted. A reaction can
                        // never succeed now, so complete it terminally instead
                        // of letting the oldest action poison every restart.
                        db::mark_pants_action_completed(db, &action.id, now, None)
                            .await
                            .map_err(database)?;
                        report
                            .skipped
                            .push(format!("{}: source message was deleted", action.id));
                        continue;
                    }
                    if let PantsError::Discord(discord) = &error
                        && discord.is_retryable()
                    {
                        report.record_retryable(&action.id, discord);
                        // Another request in the same pass is unlikely to fare
                        // better and can compound a 429. Leave the rest queued.
                        break;
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Reconcile the durable nonce against channel history before posting.
    ///
    /// Discord enforces nonce uniqueness only for the past few minutes. A
    /// process can lose an accepted response and stay down longer than that,
    /// so every retry first walks back to the action's claim time. Finding the
    /// nonce confirms the original post without creating another.
    async fn deliver_post(
        &self,
        action: &PodrickPantsAction,
    ) -> Result<Option<String>, PantsError> {
        let nonce = format!("i{}", action.source_message_id);
        if let Some(message_id) = self
            .find_message_by_nonce(&action.target_channel_id, &nonce, action.claimed_at)
            .await?
        {
            return Ok(Some(message_id));
        }
        self.discord
            .post_message_idempotent(&action.target_channel_id, &action.content, &nonce)
            .await
            .map(Some)
            .map_err(PantsError::Discord)
    }

    async fn find_message_by_nonce(
        &self,
        channel_id: &str,
        nonce: &str,
        not_before: i64,
    ) -> Result<Option<String>, PantsError> {
        let mut before = None;
        loop {
            let page = self
                .discord
                .messages(channel_id, before.as_deref())
                .await
                .map_err(PantsError::Discord)?;
            if page.is_empty() {
                return Ok(None);
            }
            validate_page(&page)?;
            if let Some(message) = page
                .iter()
                .find(|message| message.nonce.as_deref() == Some(nonce))
            {
                return Ok(Some(message.id.clone()));
            }
            let oldest = page
                .iter()
                .min_by(|left, right| snowflake_cmp(&left.id, &right.id))
                .expect("nonempty page");
            let oldest_second = page
                .iter()
                .map(channel_message_second)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .min()
                .expect("nonempty page");
            if page.len() < DISCORD_PAGE_SIZE || oldest_second < not_before {
                return Ok(None);
            }
            before = Some(oldest.id.clone());
        }
    }
}

fn asynkwerm_is_final(day: &PantsDay, now: i64) -> bool {
    let Some(local) = classify_pants_time(now) else {
        return false;
    };
    day.date < local.date || (day.date == local.date && (local.hour, local.minute) >= (18, 8))
}

fn validate_page(messages: &[ChannelMessage]) -> Result<(), PantsError> {
    for message in messages {
        snowflake(&message.id)?;
    }
    Ok(())
}

fn channel_message_second(message: &ChannelMessage) -> Result<i64, PantsError> {
    message
        .timestamp
        .parse::<Timestamp>()
        .map(|timestamp| timestamp.as_second())
        .map_err(|error| invalid_timestamp(message, error))
}

fn invalid_timestamp(message: &ChannelMessage, error: jiff::Error) -> PantsError {
    PantsError::InvalidDiscordMessage(format!(
        "{} has timestamp {:?}: {error}",
        message.id, message.timestamp
    ))
}

fn snowflake(value: &str) -> Result<u64, PantsError> {
    value.parse().map_err(|_| {
        PantsError::InvalidDiscordMessage(format!("snowflake {value:?} is not an unsigned integer"))
    })
}

fn snowflake_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn database(error: surrealdb::Error) -> PantsError {
    PantsError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::ChannelMessageAuthor;
    use benjisponge::data::podrick_models::{
        PANTS_CHANNEL_ID, PANTS_PARTICIPANTS, aggregate_pants_messages,
    };

    fn epoch(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_second()
    }

    fn message(id: &str, participant: usize, instant: &str) -> PodrickPantsMessage {
        PodrickPantsMessage {
            id: id.to_string(),
            message_id: id.to_string(),
            channel_id: PANTS_CHANNEL_ID.to_string(),
            author_id: PANTS_PARTICIPANTS[participant].author_id.to_string(),
            posted_at: epoch(instant),
        }
    }

    #[test]
    fn async_waits_until_the_pm_minute_has_closed() {
        let messages = [
            message("101", 0, "2026-07-28T10:07:00Z"),
            message("102", 1, "2026-07-28T22:07:00Z"),
            message("103", 2, "2026-07-28T22:07:01Z"),
        ];
        let day = aggregate_pants_messages(&messages).pop().unwrap();
        assert!(day.asynkwerm);
        assert!(!asynkwerm_is_final(&day, epoch("2026-07-28T22:07:59Z")));
        assert!(asynkwerm_is_final(&day, epoch("2026-07-28T22:08:00Z")));
        assert!(asynkwerm_is_final(&day, epoch("2026-07-29T04:00:00Z")));
    }

    #[test]
    fn snowflakes_compare_numerically_not_lexically() {
        assert!(snowflake_cmp("9", "10").is_lt());
        assert_eq!(snowflake("18446744073709551615").unwrap(), u64::MAX);
        assert!(snowflake("-1").is_err());
    }

    #[test]
    fn discord_rfc3339_timestamp_becomes_a_recognized_source_fact() {
        let worker = PantsWorker {
            discord: Discord::new(String::new()),
            channel_id: PANTS_CHANNEL_ID.to_string(),
            infarctions_channel_id: "1049738190107451433".to_string(),
            dry_run: true,
        };
        let source = ChannelMessage {
            id: "1234567890".to_string(),
            author: ChannelMessageAuthor {
                id: PANTS_PARTICIPANTS[2].author_id.to_string(),
            },
            timestamp: "2026-07-28T10:07:59.123456+00:00".to_string(),
            nonce: None,
        };
        let stored = worker.recognized_message(&source).unwrap().unwrap();
        assert_eq!(stored.message_id, source.id);
        assert_eq!(
            classify_pants_message(&stored).unwrap().kind,
            PantsMomentKind::Slot(PantsSlot::Am)
        );

        let stranger = ChannelMessage {
            author: ChannelMessageAuthor {
                id: "999".to_string(),
            },
            ..source
        };
        assert!(worker.recognized_message(&stranger).unwrap().is_none());
    }
}
