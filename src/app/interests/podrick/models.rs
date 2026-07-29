//! Podrick database models.
//!
//! Like the Spire and fitness models, the record identifier is projected to
//! its raw string key on load, so nothing outside the query layer ever sees a
//! SurrealDB record id.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// The Discord channel whose participant messages make up Pants Off history.
pub const PANTS_CHANNEL_ID: &str = "883473115085164544";
/// Where Podrick reports Pants Off infarctions.
pub const PANTS_INFARCTIONS_CHANNEL_ID: &str = "1049738190107451433";
/// The reaction used to mark a kwerm.
pub const KWERM_EMOJI: &str = "🪱";

/// Podrick's claim on one workout's announcement.
///
/// The row's existence means "this workout is mine to announce"; `message_id`
/// means "and I confirmed it landed". The two states are deliberately separate
/// so a process that dies between the claim and Discord's response leaves a
/// retryable row rather than a permanently lost announcement.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickAnnouncement {
    pub id: String,
    pub workout_id: String,
    /// The workout's canonical public path segment (`/lifting/{path}`).
    pub workout_path: String,
    pub channel_id: String,
    pub message_id: Option<String>,
    pub claimed_at: i64,
    pub posted_at: Option<i64>,
    pub attempts: i64,
}

/// A string-valued cursor row (`podrick_meta:<k>`).
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickMeta {
    pub k: String,
    pub v: String,
}

/// One Discord source message in `#no-pants-talk`.
///
/// Classification is deliberately not stored. The Eastern-time rules below
/// remain the single source of truth for both the worker and the heatmap.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickPantsMessage {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub author_id: String,
    /// Discord creation time as whole Unix seconds.
    pub posted_at: i64,
}

/// One claimed Discord side effect produced by Pants Off.
///
/// `action_kind` is `post` or `reaction`; `reason` is `infarction`,
/// `kwerm_am`, `kwerm_pm`, or `asynkwerm`. The database schema constrains
/// those strings while keeping this transport model compatible with direct
/// SurrealDB projections.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct PodrickPantsAction {
    pub id: String,
    pub action_kind: String,
    pub reason: String,
    pub target_channel_id: String,
    pub source_message_id: String,
    pub content: String,
    pub claimed_at: i64,
    pub completed_at: Option<i64>,
    pub output_message_id: Option<String>,
    pub attempts: i64,
}

/// A person whose messages count toward Pants Off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PantsParticipant {
    pub author_id: &'static str,
    pub display_name: &'static str,
}

/// Pants Off participants in their canonical heatmap order.
pub const PANTS_PARTICIPANTS: [PantsParticipant; 3] = [
    PantsParticipant {
        author_id: "284908269649002506",
        display_name: "Zack, Flipper of Kicks",
    },
    PantsParticipant {
        author_id: "224178544379428864",
        display_name: "Dr. Angor Zoidigsberg MD PI",
    },
    PantsParticipant {
        author_id: "129076065074151424",
        display_name: "Captain Beyond Beefheart",
    },
];

/// Find a Pants Off participant by Discord user id.
pub fn pants_participant(author_id: &str) -> Option<&'static PantsParticipant> {
    PANTS_PARTICIPANTS
        .iter()
        .find(|participant| participant.author_id == author_id)
}

fn pants_participant_index(author_id: &str) -> Option<usize> {
    PANTS_PARTICIPANTS
        .iter()
        .position(|participant| participant.author_id == author_id)
}

/// One of the two claim-bearing Pants Off times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PantsSlot {
    Am,
    Pm,
}

impl PantsSlot {
    pub const fn label(self) -> &'static str {
        match self {
            PantsSlot::Am => "6:07 AM",
            PantsSlot::Pm => "6:07 PM",
        }
    }
}

/// How a participant message counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PantsMomentKind {
    /// A claim at 6:07 AM or PM.
    Slot(PantsSlot),
    /// Minute 07 at an hour other than 6 AM or 6 PM.
    OutOfTown,
    /// Any minute other than 07.
    Infarction,
}

impl PantsMomentKind {
    pub const fn label(self) -> &'static str {
        match self {
            PantsMomentKind::Slot(slot) => slot.label(),
            PantsMomentKind::OutOfTown => "out of town",
            PantsMomentKind::Infarction => "infarction",
        }
    }
}

/// Eastern wall-clock classification of an epoch second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PantsLocalMinute {
    pub date: Date,
    pub hour: i8,
    pub minute: i8,
    pub kind: PantsMomentKind,
}

impl PantsLocalMinute {
    /// A compact twelve-hour clock label, such as `12:07 PM`.
    pub fn clock_label(self) -> String {
        let suffix = if self.hour < 12 { "AM" } else { "PM" };
        let hour = match self.hour % 12 {
            0 => 12,
            hour => hour,
        };
        format!("{hour}:{:02} {suffix}", self.minute)
    }
}

fn eastern_time_zone() -> &'static TimeZone {
    static EASTERN: OnceLock<TimeZone> = OnceLock::new();
    EASTERN.get_or_init(|| TimeZone::get("America/New_York").expect("bundled tzdb has New York"))
}

/// Project an epoch second into the Eastern day/minute and apply Pants Off's
/// time rules. Seconds within the minute are intentionally ignored.
///
/// Returns `None` only when `posted_at` is outside jiff's supported timestamp
/// range; Discord creation times are always in range.
pub fn classify_pants_time(posted_at: i64) -> Option<PantsLocalMinute> {
    let zoned = Timestamp::from_second(posted_at)
        .ok()?
        .to_zoned(eastern_time_zone().clone());
    let hour = zoned.hour();
    let minute = zoned.minute();
    let kind = match (hour, minute) {
        (6, 7) => PantsMomentKind::Slot(PantsSlot::Am),
        (18, 7) => PantsMomentKind::Slot(PantsSlot::Pm),
        (_, 7) => PantsMomentKind::OutOfTown,
        _ => PantsMomentKind::Infarction,
    };
    Some(PantsLocalMinute {
        date: zoned.date(),
        hour,
        minute,
        kind,
    })
}

/// The Eastern calendar date containing an epoch second.
pub fn pants_eastern_date(epoch_seconds: i64) -> Option<Date> {
    classify_pants_time(epoch_seconds).map(|local| local.date)
}

/// A recognized participant message with its derived Eastern classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedPantsMessage {
    pub id: String,
    pub message_id: String,
    pub channel_id: String,
    pub author_id: String,
    pub posted_at: i64,
    pub participant: PantsParticipant,
    pub date: Date,
    pub hour: i8,
    pub minute: i8,
    pub kind: PantsMomentKind,
}

impl ClassifiedPantsMessage {
    /// A compact twelve-hour clock label, such as `6:07 PM`.
    pub fn clock_label(&self) -> String {
        PantsLocalMinute {
            date: self.date,
            hour: self.hour,
            minute: self.minute,
            kind: self.kind,
        }
        .clock_label()
    }
}

/// Classify a stored source message.
///
/// Unknown authors and timestamps outside jiff's range are ignored. Channel
/// scoping happens when the worker fetches and stores source messages; the
/// channel id remains on this value so reaction actions can target it.
pub fn classify_pants_message(message: &PodrickPantsMessage) -> Option<ClassifiedPantsMessage> {
    let participant = *pants_participant(&message.author_id)?;
    let local = classify_pants_time(message.posted_at)?;
    Some(ClassifiedPantsMessage {
        id: message.id.clone(),
        message_id: message.message_id.clone(),
        channel_id: message.channel_id.clone(),
        author_id: message.author_id.clone(),
        posted_at: message.posted_at,
        participant,
        date: local.date,
        hour: local.hour,
        minute: local.minute,
        kind: local.kind,
    })
}

/// One participant's result on one Eastern calendar day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PantsParticipantDay {
    pub participant: PantsParticipant,
    /// Earliest source message in the AM 6:07 minute.
    pub first_am: Option<ClassifiedPantsMessage>,
    /// Earliest source message in the PM 6:07 minute.
    pub first_pm: Option<ClassifiedPantsMessage>,
    /// Every minute-07 message outside the two home slots.
    pub out_of_town: Vec<ClassifiedPantsMessage>,
    /// Every message outside minute 07.
    pub infarctions: Vec<ClassifiedPantsMessage>,
}

impl PantsParticipantDay {
    fn new(participant: PantsParticipant) -> Self {
        Self {
            participant,
            first_am: None,
            first_pm: None,
            out_of_town: Vec::new(),
            infarctions: Vec::new(),
        }
    }

    /// The first source message in a claim-bearing slot.
    pub fn first(&self, slot: PantsSlot) -> Option<&ClassifiedPantsMessage> {
        match slot {
            PantsSlot::Am => self.first_am.as_ref(),
            PantsSlot::Pm => self.first_pm.as_ref(),
        }
    }

    /// This participant's earliest claim-bearing source message that day.
    pub fn first_claim(&self) -> Option<&ClassifiedPantsMessage> {
        match (&self.first_am, &self.first_pm) {
            (Some(am), Some(pm)) if message_order(am, pm).is_gt() => Some(pm),
            (Some(am), _) => Some(am),
            (_, Some(pm)) => Some(pm),
            (None, None) => None,
        }
    }

    /// Distinct AM/PM claims, from zero through two.
    pub fn claims(&self) -> u8 {
        u8::from(self.first_am.is_some()) + u8::from(self.first_pm.is_some())
    }

    pub fn is_double(&self) -> bool {
        self.claims() == 2
    }

    pub fn out_of_town_count(&self) -> usize {
        self.out_of_town.len()
    }

    pub fn infarction_count(&self) -> usize {
        self.infarctions.len()
    }

    fn record(&mut self, message: ClassifiedPantsMessage) {
        match message.kind {
            PantsMomentKind::Slot(PantsSlot::Am) => {
                keep_first(&mut self.first_am, message);
            }
            PantsMomentKind::Slot(PantsSlot::Pm) => {
                keep_first(&mut self.first_pm, message);
            }
            PantsMomentKind::OutOfTown => self.out_of_town.push(message),
            PantsMomentKind::Infarction => self.infarctions.push(message),
        }
    }

    fn sort_moments(&mut self) {
        self.out_of_town.sort_by(message_order);
        self.infarctions.sort_by(message_order);
    }
}

/// Pants Off's aggregate state for one Eastern calendar day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PantsDay {
    pub date: Date,
    /// Always ordered exactly like [`PANTS_PARTICIPANTS`].
    pub participants: [PantsParticipantDay; 3],
    pub kwerm_am: bool,
    pub kwerm_pm: bool,
    pub asynkwerm: bool,
}

impl PantsDay {
    fn new(date: Date) -> Self {
        Self {
            date,
            participants: [
                PantsParticipantDay::new(PANTS_PARTICIPANTS[0]),
                PantsParticipantDay::new(PANTS_PARTICIPANTS[1]),
                PantsParticipantDay::new(PANTS_PARTICIPANTS[2]),
            ],
            kwerm_am: false,
            kwerm_pm: false,
            asynkwerm: false,
        }
    }

    /// `YYYY-MM-DD`, suitable for heatmap keys and stable labels.
    pub fn date_label(&self) -> String {
        self.date.strftime("%Y-%m-%d").to_string()
    }

    pub fn participant(&self, author_id: &str) -> Option<&PantsParticipantDay> {
        pants_participant_index(author_id).map(|index| &self.participants[index])
    }

    /// Number of synchronous kwerms on this day (zero through two).
    pub fn kwerm_count(&self) -> u8 {
        u8::from(self.kwerm_am) + u8::from(self.kwerm_pm)
    }

    /// The three first source messages that make up a synchronous kwerm.
    ///
    /// This is directly usable for the worker's worm reactions. It returns
    /// `None` unless all three participants claimed the requested slot.
    pub fn kwerm_messages(&self, slot: PantsSlot) -> Option<[&ClassifiedPantsMessage; 3]> {
        Some([
            self.participants[0].first(slot)?,
            self.participants[1].first(slot)?,
            self.participants[2].first(slot)?,
        ])
    }

    /// The three source messages that make up an asynkwerm.
    ///
    /// A participant with a double contributes only their earlier valid slot,
    /// so every asynkwerm produces exactly one worm reaction per person.
    pub fn asynkwerm_messages(&self) -> Option<[&ClassifiedPantsMessage; 3]> {
        if !self.asynkwerm {
            return None;
        }
        Some([
            self.participants[0].first_claim()?,
            self.participants[1].first_claim()?,
            self.participants[2].first_claim()?,
        ])
    }

    /// Every distinct claim-bearing source message, in participant order and
    /// AM-before-PM order. Useful when selecting reactions for an asynkwerm.
    pub fn first_messages(&self) -> impl Iterator<Item = &ClassifiedPantsMessage> {
        self.participants
            .iter()
            .flat_map(|participant| [&participant.first_am, &participant.first_pm])
            .filter_map(Option::as_ref)
    }

    fn finish(&mut self) {
        for participant in &mut self.participants {
            participant.sort_moments();
        }
        self.kwerm_am = self
            .participants
            .iter()
            .all(|participant| participant.first_am.is_some());
        self.kwerm_pm = self
            .participants
            .iter()
            .all(|participant| participant.first_pm.is_some());
        let everyone_has_a_claim = self
            .participants
            .iter()
            .all(|participant| participant.claims() > 0);
        self.asynkwerm = everyone_has_a_claim && !self.kwerm_am && !self.kwerm_pm;
    }
}

/// Aggregate source messages into Eastern calendar days.
///
/// Unknown authors and out-of-range timestamps are ignored. Duplicate posts
/// in a 6:07 slot yield one claim and preserve only the earliest message for
/// reactions. Out-of-town posts and infarctions never affect claims or kwerm
/// state and are retained as separate moments.
pub fn aggregate_pants_messages(messages: &[PodrickPantsMessage]) -> Vec<PantsDay> {
    let mut days = BTreeMap::<Date, PantsDay>::new();
    for message in messages {
        let Some(classified) = classify_pants_message(message) else {
            continue;
        };
        let Some(participant_index) = pants_participant_index(&classified.author_id) else {
            // `classify_pants_message` already established this; keeping the
            // aggregation total makes future participant-model edits safe.
            continue;
        };
        let date = classified.date;
        days.entry(date)
            .or_insert_with(|| PantsDay::new(date))
            .participants[participant_index]
            .record(classified);
    }
    days.into_values()
        .map(|mut day| {
            day.finish();
            day
        })
        .collect()
}

fn keep_first(current: &mut Option<ClassifiedPantsMessage>, candidate: ClassifiedPantsMessage) {
    if current
        .as_ref()
        .is_none_or(|first| message_order(&candidate, first).is_lt())
    {
        *current = Some(candidate);
    }
}

fn message_order(left: &ClassifiedPantsMessage, right: &ClassifiedPantsMessage) -> Ordering {
    left.posted_at
        .cmp(&right.posted_at)
        .then_with(|| snowflake_order(&left.message_id, &right.message_id))
}

fn snowflake_order(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZACK: &str = "284908269649002506";
    const DRANGOR: &str = "224178544379428864";
    const CAPTAIN: &str = "129076065074151424";

    fn epoch(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_second()
    }

    fn message(message_id: &str, author_id: &str, instant: &str) -> PodrickPantsMessage {
        PodrickPantsMessage {
            id: message_id.to_string(),
            message_id: message_id.to_string(),
            channel_id: PANTS_CHANNEL_ID.to_string(),
            author_id: author_id.to_string(),
            posted_at: epoch(instant),
        }
    }

    fn one_day(messages: &[PodrickPantsMessage]) -> PantsDay {
        let days = aggregate_pants_messages(messages);
        assert_eq!(days.len(), 1);
        days.into_iter().next().unwrap()
    }

    #[test]
    fn participant_order_and_names_are_the_contract() {
        assert_eq!(
            PANTS_PARTICIPANTS,
            [
                PantsParticipant {
                    author_id: ZACK,
                    display_name: "Zack, Flipper of Kicks",
                },
                PantsParticipant {
                    author_id: DRANGOR,
                    display_name: "Dr. Angor Zoidigsberg MD PI",
                },
                PantsParticipant {
                    author_id: CAPTAIN,
                    display_name: "Captain Beyond Beefheart",
                },
            ]
        );
    }

    #[test]
    fn duplicate_posts_in_one_slot_claim_once_and_keep_the_true_first() {
        // Reverse input order, and put both messages in one epoch second. The
        // lower Discord snowflake is the earlier source message.
        let messages = [
            message("102", ZACK, "2026-07-28T10:07:12Z"),
            message("101", ZACK, "2026-07-28T10:07:12Z"),
        ];
        let day = one_day(&messages);
        let zack = day.participant(ZACK).unwrap();
        assert_eq!(zack.claims(), 1);
        assert!(!zack.is_double());
        assert_eq!(zack.first_am.as_ref().unwrap().message_id, "101");
        assert!(zack.first_pm.is_none());
        assert_eq!(day.first_messages().count(), 1);
    }

    #[test]
    fn am_and_pm_are_a_double() {
        let day = one_day(&[
            message("101", ZACK, "2026-07-28T10:07:59Z"),
            message("102", ZACK, "2026-07-28T22:07:00Z"),
        ]);
        let zack = day.participant(ZACK).unwrap();
        assert_eq!(zack.claims(), 2);
        assert!(zack.is_double());
        assert_eq!(zack.first(PantsSlot::Am).unwrap().clock_label(), "6:07 AM");
        assert_eq!(zack.first(PantsSlot::Pm).unwrap().clock_label(), "6:07 PM");
    }

    #[test]
    fn out_of_town_and_infarction_moments_are_orthogonal_to_claims() {
        let day = one_day(&[
            message("102", ZACK, "2026-07-28T16:07:45Z"), // 12:07 PM
            message("101", ZACK, "2026-07-28T10:08:00Z"), // 6:08 AM
            message("103", ZACK, "2026-07-28T17:07:00Z"), // 1:07 PM
        ]);
        let zack = day.participant(ZACK).unwrap();
        assert_eq!(zack.claims(), 0);
        assert_eq!(zack.out_of_town_count(), 2);
        assert_eq!(zack.infarction_count(), 1);
        assert_eq!(zack.out_of_town[0].clock_label(), "12:07 PM");
        assert_eq!(zack.out_of_town[1].clock_label(), "1:07 PM");
        assert_eq!(zack.infarctions[0].clock_label(), "6:08 AM");
        assert!(!day.kwerm_am);
        assert!(!day.kwerm_pm);
        assert!(!day.asynkwerm);
    }

    #[test]
    fn all_three_in_the_same_slot_make_synchronous_kwerms() {
        let messages = [
            message("101", ZACK, "2026-07-28T10:07:00Z"),
            message("102", DRANGOR, "2026-07-28T10:07:01Z"),
            message("103", CAPTAIN, "2026-07-28T10:07:02Z"),
            message("201", ZACK, "2026-07-28T22:07:00Z"),
            message("202", DRANGOR, "2026-07-28T22:07:01Z"),
            message("203", CAPTAIN, "2026-07-28T22:07:02Z"),
        ];
        let day = one_day(&messages);
        assert!(day.kwerm_am);
        assert!(day.kwerm_pm);
        assert_eq!(day.kwerm_count(), 2);
        assert!(!day.asynkwerm);
        assert_eq!(
            day.kwerm_messages(PantsSlot::Am)
                .unwrap()
                .map(|message| message.message_id.as_str()),
            ["101", "102", "103"]
        );
        assert_eq!(
            day.kwerm_messages(PantsSlot::Pm)
                .unwrap()
                .map(|message| message.message_id.as_str()),
            ["201", "202", "203"]
        );
    }

    #[test]
    fn split_slots_make_an_asynkwerm() {
        let day = one_day(&[
            message("101", DRANGOR, "2026-07-28T10:07:00Z"),
            message("102", ZACK, "2026-07-28T22:07:00Z"),
            message("103", CAPTAIN, "2026-07-28T22:07:01Z"),
        ]);
        assert!(!day.kwerm_am);
        assert!(!day.kwerm_pm);
        assert!(day.asynkwerm);
        assert!(day.kwerm_messages(PantsSlot::Am).is_none());
        assert!(day.kwerm_messages(PantsSlot::Pm).is_none());
        assert_eq!(
            day.asynkwerm_messages()
                .unwrap()
                .map(|message| message.message_id.as_str()),
            ["102", "101", "103"]
        );
    }

    #[test]
    fn an_asynkwerm_uses_one_earliest_message_for_a_double_participant() {
        let day = one_day(&[
            message("101", ZACK, "2026-07-28T10:07:00Z"),
            message("102", ZACK, "2026-07-28T22:07:00Z"),
            message("103", DRANGOR, "2026-07-28T10:07:01Z"),
            message("104", CAPTAIN, "2026-07-28T22:07:01Z"),
        ]);
        assert!(day.asynkwerm);
        assert_eq!(
            day.asynkwerm_messages()
                .unwrap()
                .map(|message| message.message_id.as_str()),
            ["101", "103", "104"]
        );
    }

    #[test]
    fn a_later_matching_post_promotes_async_to_synchronous_kwerm() {
        let mut messages = vec![
            message("101", DRANGOR, "2026-07-28T10:07:00Z"),
            message("102", ZACK, "2026-07-28T22:07:00Z"),
            message("103", CAPTAIN, "2026-07-28T22:07:01Z"),
        ];
        assert!(one_day(&messages).asynkwerm);

        messages.push(message("104", DRANGOR, "2026-07-28T22:07:30Z"));
        let promoted = one_day(&messages);
        assert!(promoted.kwerm_pm);
        assert!(!promoted.asynkwerm);
        assert_eq!(
            promoted
                .kwerm_messages(PantsSlot::Pm)
                .unwrap()
                .map(|message| message.message_id.as_str()),
            ["102", "104", "103"]
        );
    }

    #[test]
    fn eastern_projection_uses_dst_and_the_eastern_calendar_day() {
        // 6:07 is UTC-5 in January and UTC-4 in July.
        let winter = classify_pants_time(epoch("2026-01-15T11:07:59Z")).unwrap();
        let summer = classify_pants_time(epoch("2026-07-15T10:07:00Z")).unwrap();
        assert_eq!(winter.kind, PantsMomentKind::Slot(PantsSlot::Am));
        assert_eq!(summer.kind, PantsMomentKind::Slot(PantsSlot::Am));
        assert_eq!(winter.clock_label(), "6:07 AM");
        assert_eq!(summer.clock_label(), "6:07 AM");

        // A UTC-Wednesday instant still aggregates under Tuesday in New York.
        let late_tuesday = message("101", ZACK, "2026-07-29T02:07:00Z");
        let day = one_day(&[late_tuesday]);
        assert_eq!(day.date_label(), "2026-07-28");
        assert_eq!(
            day.participant(ZACK).unwrap().out_of_town[0].clock_label(),
            "10:07 PM"
        );
    }

    #[test]
    fn unknown_authors_and_out_of_range_instants_are_ignored() {
        let unknown = message("101", "999", "2026-07-28T10:07:00Z");
        let invalid = PodrickPantsMessage {
            posted_at: i64::MAX,
            ..message("102", ZACK, "2026-07-28T10:07:00Z")
        };
        assert!(aggregate_pants_messages(&[unknown, invalid]).is_empty());
    }

    #[test]
    fn days_and_non_claim_moments_are_chronological() {
        let messages = [
            message("204", ZACK, "2026-07-29T15:10:00Z"),
            message("103", ZACK, "2026-07-28T17:07:00Z"),
            message("101", ZACK, "2026-07-28T16:07:00Z"),
            message("102", ZACK, "2026-07-28T16:08:00Z"),
        ];
        let days = aggregate_pants_messages(&messages);
        assert_eq!(
            days.iter().map(PantsDay::date_label).collect::<Vec<_>>(),
            ["2026-07-28", "2026-07-29"]
        );
        assert_eq!(
            days[0].participants[0]
                .out_of_town
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            ["101", "103"]
        );
    }
}
