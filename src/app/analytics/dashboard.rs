//! Public aggregates built from a bounded analytics-event snapshot.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use benjisponge::data::{
    Db, analytics_facts,
    analytics_models::{AnalyticsEvent, AnalyticsVisitorDay},
};
use tokio::time::timeout;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Week,
    Month,
    Quarter,
    Year,
}

impl Window {
    pub const ALL: [Self; 4] = [Self::Week, Self::Month, Self::Quarter, Self::Year];

    #[cfg(test)]
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("7d") => Self::Week,
            Some("90d") => Self::Quarter,
            Some("365d") => Self::Year,
            _ => Self::Month,
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
            Self::Year => "365d",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Week => "7 days",
            Self::Month => "30 days",
            Self::Quarter => "90 days",
            Self::Year => "1 year",
        }
    }

    pub const fn days(self) -> i64 {
        match self {
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
            Self::Year => 365,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Dashboard {
    pub overview: Overview,
    pub performance: Performance,
    pub days: Vec<Day>,
    pub pages: Vec<Page>,
    pub channels: Vec<Count>,
    pub referrers: Vec<Cohort>,
    pub countries: Vec<Cohort>,
    pub technology: Vec<Technology>,
    pub hourly: [[i64; 24]; 7],
    pub journeys: Vec<Journey>,
    pub outbound: Vec<Cohort>,
    pub campaigns: Vec<Campaign>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Overview {
    pub pageviews: i64,
    pub visitors: i64,
    pub sessions: i64,
    pub engaged_seconds: i64,
    pub outbound_clicks: i64,
    pub returning_percent: i64,
    pub single_page_percent: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Performance {
    pub attention_seconds: i64,
    pub scroll_percent: i64,
    pub finish_percent: i64,
    pub lcp_milliseconds: i64,
    pub cls_thousandths: i64,
    pub navigation_milliseconds: i64,
    pub samples: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Day {
    pub date: String,
    pub views: i64,
    pub visitors: i64,
    pub engaged_seconds: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Page {
    pub path: String,
    pub views: i64,
    pub visitors: i64,
    pub engaged_seconds: i64,
    pub scroll_percent: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Count {
    pub label: String,
    pub count: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cohort {
    pub label: String,
    pub views: i64,
    pub visitors: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Technology {
    pub dimension: String,
    pub label: String,
    pub views: i64,
    pub visitors: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Journey {
    pub from: String,
    pub to: String,
    pub trips: i64,
    pub visitors: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Campaign {
    pub source: String,
    pub campaign: String,
    pub views: i64,
    pub visitors: i64,
}

pub async fn load(db: &Db, cutoff: i64) -> anyhow::Result<Dashboard> {
    let current = current_timestamp()?;
    match analytics_facts::load(db, cutoff).await {
        Ok(Some(facts)) => return aggregate_facts(&facts, cutoff, current),
        Ok(None) => {}
        Err(error) => eprintln!("analytics facts: ready load failed; using raw snapshot: {error}"),
    }

    let raw = load_raw(db, cutoff, current).await?;
    if analytics_facts::ready_for_parity(db).await.unwrap_or(false) {
        match analytics_facts::load_for_parity(db, cutoff)
            .await
            .and_then(|facts| aggregate_facts(&facts, cutoff, current))
        {
            Ok(fact) if fact == raw => {
                let days = current.div_euclid(86_400) - cutoff.div_euclid(86_400) + 1;
                let bit = match days {
                    7 => 1,
                    30 => 2,
                    90 => 4,
                    365 => 8,
                    _ => 0,
                };
                if bit != 0 {
                    match analytics_facts::record_parity(db, bit).await {
                        Ok(mask) => eprintln!(
                            "analytics facts: exact parity confirmed for {days} days (mask {mask}/15)"
                        ),
                        Err(error) => {
                            eprintln!("analytics facts: could not persist parity: {error}")
                        }
                    }
                }
            }
            Ok(_) => eprintln!(
                "analytics facts: parity mismatch for cutoff day {}",
                cutoff.div_euclid(86_400)
            ),
            Err(error) => {
                eprintln!("analytics facts: parity load failed; using raw snapshot: {error}")
            }
        }
    }
    Ok(raw)
}

async fn load_raw(db: &Db, cutoff: i64, current: i64) -> anyhow::Result<Dashboard> {
    // Prior markers only need the idle window before `$cutoff`. Intersect that
    // tiny candidate set with in-window pageview sessions in Rust.
    //
    // Page through events with a hard row limit: even a single busy UTC day was
    // large enough to reset the shared SurrealDB websocket, so day chunks alone
    // were not enough. Explicit projections keep NONE option fields intact.
    const PAGE: i64 = 200;
    let prior_floor = cutoff.saturating_sub(super::db::SESSION_IDLE_SECONDS);
    let first_day = cutoff.div_euclid(86_400);
    let last_day = current.div_euclid(86_400);
    let day_count = last_day.saturating_sub(first_day).saturating_add(1);
    let budget =
        Duration::from_secs(u64::try_from(day_count.saturating_mul(5).max(15)).unwrap_or(15));
    let (events, prior_sessions) = timeout(budget, async {
        let mut events = Vec::new();
        for day in first_day..=last_day {
            let floor = day.saturating_mul(86_400).max(cutoff);
            let ceiling = (day + 1).saturating_mul(86_400);
            let mut cursor_at = floor.saturating_sub(1);
            let mut cursor_id = String::new();
            loop {
                let mut response = db
                    .query(
                        "SELECT record::id(id) AS id,
                                visitor_id, session_id, occurred_at, kind, page_path,
                                referrer_kind, referrer_host, referrer_path, country_code,
                                timezone, language, device_kind, browser, operating_system,
                                viewport_kind, navigation_kind, local_hour, local_weekday,
                                engagement_seconds, scroll_percent, lcp_milliseconds,
                                cls_thousandths, navigation_milliseconds, target_host,
                                utm_source, utm_medium, utm_campaign
                         FROM analytics_events
                         WHERE occurred_at >= $floor AND occurred_at < $ceiling
                           AND (occurred_at > $cursor_at
                                OR (occurred_at = $cursor_at AND record::id(id) > $cursor_id))
                         ORDER BY occurred_at ASC, id ASC
                         LIMIT $limit",
                    )
                    .bind(("floor", floor))
                    .bind(("ceiling", ceiling))
                    .bind(("cursor_at", cursor_at))
                    .bind(("cursor_id", cursor_id.clone()))
                    .bind(("limit", PAGE))
                    .await
                    .context("analytics snapshot query failed")?
                    .check()
                    .context("analytics snapshot query failed")?;
                let mut chunk: Vec<AnalyticsEvent> = response
                    .take(0)
                    .context("analytics snapshot decoding failed")?;
                let count = chunk.len();
                if let Some(last) = chunk.last() {
                    cursor_at = last.occurred_at;
                    cursor_id = last.id.clone();
                }
                events.append(&mut chunk);
                if count < PAGE as usize {
                    break;
                }
            }
        }

        let mut response = db
            .query(
                "SELECT VALUE session_id
                 FROM analytics_events
                 WHERE kind = 'pageview'
                     AND occurred_at < $cutoff
                     AND occurred_at >= $prior_floor
                 GROUP BY session_id",
            )
            .bind(("cutoff", cutoff))
            .bind(("prior_floor", prior_floor))
            .await
            .context("analytics prior-session query failed")?
            .check()
            .context("analytics prior-session query failed")?;
        let candidates: Vec<String> = response
            .take(0)
            .context("analytics prior-session decoding failed")?;

        let window_sessions: HashSet<&str> = events
            .iter()
            .filter(|event| event.kind == "pageview")
            .map(|event| event.session_id.as_str())
            .collect();
        let prior_sessions = candidates
            .into_iter()
            .filter(|session_id| window_sessions.contains(session_id.as_str()))
            .collect::<HashSet<_>>();
        Ok::<_, anyhow::Error>((events, prior_sessions))
    })
    .await
    .with_context(|| {
        format!(
            "analytics snapshot exceeded {} seconds across {day_count} day pages",
            budget.as_secs()
        )
    })??;

    Ok(aggregate(&events, &prior_sessions, cutoff, current))
}

fn current_timestamp() -> anyhow::Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch")?
            .as_secs(),
    )
    .context("current timestamp exceeds i64")
}

fn aggregate_facts(
    facts: &[AnalyticsVisitorDay],
    cutoff: i64,
    current: i64,
) -> anyhow::Result<Dashboard> {
    let cutoff_day = cutoff.div_euclid(86_400);
    let prior_floor = cutoff.saturating_sub(super::db::SESSION_IDLE_SECONDS);
    let mut prior_sessions = HashSet::new();
    let mut events = Vec::new();
    for fact in facts {
        for grouped in &fact.events {
            if grouped.event.visitor_id != fact.visitor_id {
                anyhow::bail!("analytics visitor-day payload has a mismatched visitor");
            }
            if fact.utc_day < cutoff_day {
                if grouped.event.kind == "pageview" && grouped.last_occurred_at >= prior_floor {
                    prior_sessions.insert(grouped.event.session_id.clone());
                }
                continue;
            }
            let count =
                usize::try_from(grouped.count).context("analytics fact count exceeds usize")?;
            for ordinal in 0..count {
                let mut event = grouped.event.clone();
                if ordinal > 0 {
                    event.id = format!("{}~{ordinal}", event.id);
                }
                events.push(event);
            }
        }
    }
    Ok(aggregate(&events, &prior_sessions, cutoff, current))
}

fn aggregate(
    events: &[AnalyticsEvent],
    prior_sessions: &HashSet<String>,
    cutoff: i64,
    current: i64,
) -> Dashboard {
    let loaded: Vec<&AnalyticsEvent> = events
        .iter()
        .filter(|event| event.occurred_at >= cutoff)
        .collect();
    let arrivals = earliest_pageviews(&loaded, prior_sessions);

    Dashboard {
        overview: aggregate_overview(&loaded),
        performance: aggregate_performance(&loaded),
        days: aggregate_days(&loaded, cutoff, current),
        pages: aggregate_pages(&loaded),
        channels: aggregate_channels(&arrivals),
        referrers: aggregate_referrers(&arrivals),
        countries: aggregate_countries(&loaded),
        technology: aggregate_technology(&loaded),
        hourly: aggregate_hourly(&loaded),
        journeys: aggregate_journeys(&loaded),
        outbound: aggregate_outbound(&loaded),
        campaigns: aggregate_campaigns(&arrivals),
    }
}

fn aggregate_overview(events: &[&AnalyticsEvent]) -> Overview {
    let mut pageviews = 0;
    let mut visitors = HashSet::new();
    let mut sessions: HashMap<(&str, &str), i64> = HashMap::new();
    let mut visitor_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut engaged_seconds = 0;
    let mut outbound_clicks = 0;

    for event in events {
        match event.kind.as_str() {
            "pageview" => {
                pageviews += 1;
                visitors.insert(event.visitor_id.as_str());
                *sessions
                    .entry((event.visitor_id.as_str(), event.session_id.as_str()))
                    .or_default() += 1;
                visitor_sessions
                    .entry(event.visitor_id.as_str())
                    .or_default()
                    .insert(event.session_id.as_str());
            }
            "engagement" => engaged_seconds += event.engagement_seconds.unwrap_or(0),
            "outbound" => outbound_clicks += 1,
            _ => {}
        }
    }

    let returning = visitor_sessions
        .values()
        .filter(|sessions| sessions.len() > 1)
        .count();
    let single_page = sessions.values().filter(|views| **views == 1).count();
    Overview {
        pageviews,
        visitors: usize_to_i64(visitors.len()),
        sessions: usize_to_i64(sessions.len()),
        engaged_seconds,
        outbound_clicks,
        returning_percent: percent(returning, visitor_sessions.len()),
        single_page_percent: percent(single_page, sessions.len()),
    }
}

#[derive(Default)]
struct Average {
    sum: i128,
    samples: usize,
}

impl Average {
    fn push(&mut self, value: Option<i64>) {
        if let Some(value) = value {
            self.sum += i128::from(value);
            self.samples += 1;
        }
    }

    fn rounded(&self) -> i64 {
        positive_half_up(self.sum, self.samples)
    }
}

fn aggregate_performance(events: &[&AnalyticsEvent]) -> Performance {
    let mut attention = Average::default();
    let mut scroll = Average::default();
    let mut lcp = Average::default();
    let mut cls = Average::default();
    let mut navigation = Average::default();
    let mut finishes = 0;
    let mut scroll_samples = 0;
    let mut samples = 0;

    for event in events.iter().filter(|event| event.kind == "engagement") {
        samples += 1;
        attention.push(event.engagement_seconds);
        scroll.push(event.scroll_percent);
        lcp.push(event.lcp_milliseconds);
        cls.push(event.cls_thousandths);
        navigation.push(event.navigation_milliseconds);
        if let Some(value) = event.scroll_percent {
            scroll_samples += 1;
            finishes += usize::from(value >= 90);
        }
    }

    Performance {
        attention_seconds: attention.rounded(),
        scroll_percent: scroll.rounded(),
        finish_percent: percent(finishes, scroll_samples),
        lcp_milliseconds: lcp.rounded(),
        cls_thousandths: cls.rounded(),
        navigation_milliseconds: navigation.rounded(),
        samples: usize_to_i64(samples),
    }
}

#[derive(Default)]
struct Daily<'a> {
    views: i64,
    visitors: HashSet<&'a str>,
    engaged_seconds: i64,
}

fn aggregate_days(events: &[&AnalyticsEvent], cutoff: i64, current: i64) -> Vec<Day> {
    const SECONDS_PER_DAY: i64 = 86_400;

    let mut daily: BTreeMap<i64, Daily<'_>> = BTreeMap::new();
    for event in events {
        let day = event.occurred_at.div_euclid(SECONDS_PER_DAY);
        let aggregate = daily.entry(day).or_default();
        match event.kind.as_str() {
            "pageview" => {
                aggregate.views += 1;
                aggregate.visitors.insert(event.visitor_id.as_str());
            }
            "engagement" => {
                aggregate.engaged_seconds += event.engagement_seconds.unwrap_or(0);
            }
            _ => {}
        }
    }

    let first = cutoff.div_euclid(SECONDS_PER_DAY);
    let last = current.div_euclid(SECONDS_PER_DAY);
    if first > last {
        return Vec::new();
    }
    (first..=last)
        .map(|day| {
            let aggregate = daily.get(&day);
            Day {
                date: format_utc_day(day),
                views: aggregate.map_or(0, |value| value.views),
                visitors: aggregate.map_or(0, |value| usize_to_i64(value.visitors.len())),
                engaged_seconds: aggregate.map_or(0, |value| value.engaged_seconds),
            }
        })
        .collect()
}

#[derive(Default)]
struct PageAggregate<'a> {
    views: i64,
    visitors: HashSet<&'a str>,
    engaged_seconds: i64,
    scroll: Average,
}

fn aggregate_pages(events: &[&AnalyticsEvent]) -> Vec<Page> {
    let mut aggregates: BTreeMap<&str, PageAggregate<'_>> = BTreeMap::new();
    for event in events {
        let aggregate = aggregates.entry(event.page_path.as_str()).or_default();
        match event.kind.as_str() {
            "pageview" => {
                aggregate.views += 1;
                aggregate.visitors.insert(event.visitor_id.as_str());
            }
            "engagement" => {
                aggregate.engaged_seconds += event.engagement_seconds.unwrap_or(0);
                aggregate.scroll.push(event.scroll_percent);
            }
            _ => {}
        }
    }

    let mut pages: Vec<Page> = aggregates
        .into_iter()
        .filter(|(_, aggregate)| aggregate.views > 0)
        .map(|(path, aggregate)| Page {
            path: path.to_owned(),
            views: aggregate.views,
            visitors: usize_to_i64(aggregate.visitors.len()),
            engaged_seconds: aggregate.engaged_seconds,
            scroll_percent: aggregate.scroll.rounded(),
        })
        .collect();
    pages.sort_by(|left, right| {
        right
            .views
            .cmp(&left.views)
            .then_with(|| left.path.cmp(&right.path))
    });

    let fixed_routes: HashSet<String> = crate::content::routes::site_routes().into_iter().collect();
    pages.retain(|page| fixed_routes.contains(&page.path) || page.visitors >= 3);
    pages.truncate(12);
    pages
}

fn earliest_pageviews<'a>(
    events: &[&'a AnalyticsEvent],
    prior_sessions: &HashSet<String>,
) -> Vec<&'a AnalyticsEvent> {
    let mut arrivals: HashMap<(&str, &str), &AnalyticsEvent> = HashMap::new();
    for event in events.iter().filter(|event| event.kind == "pageview") {
        if prior_sessions.contains(event.session_id.as_str()) {
            continue;
        }
        let key = (event.visitor_id.as_str(), event.session_id.as_str());
        arrivals
            .entry(key)
            .and_modify(|earlier| {
                if (event.occurred_at, event.id.as_str())
                    < (earlier.occurred_at, earlier.id.as_str())
                {
                    *earlier = event;
                }
            })
            .or_insert(event);
    }
    arrivals.into_values().collect()
}

fn aggregate_channels(arrivals: &[&AnalyticsEvent]) -> Vec<Count> {
    let mut aggregates: BTreeMap<&str, i64> = BTreeMap::new();
    for event in arrivals {
        *aggregates.entry(event.referrer_kind.as_str()).or_default() += 1;
    }
    let mut channels: Vec<Count> = aggregates
        .into_iter()
        .map(|(label, count)| Count {
            label: label.to_owned(),
            count,
        })
        .collect();
    channels.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.label.cmp(&right.label))
    });
    channels
}

#[derive(Default)]
struct CohortAggregate<'a> {
    views: i64,
    visitors: HashSet<&'a str>,
}

fn add_cohort<'a>(
    aggregates: &mut BTreeMap<&'a str, CohortAggregate<'a>>,
    label: &'a str,
    visitor: &'a str,
) {
    let aggregate = aggregates.entry(label).or_default();
    aggregate.views += 1;
    aggregate.visitors.insert(visitor);
}

fn collect_cohorts(aggregates: BTreeMap<&str, CohortAggregate<'_>>, limit: usize) -> Vec<Cohort> {
    let mut cohorts: Vec<Cohort> = aggregates
        .into_iter()
        .filter(|(_, aggregate)| aggregate.visitors.len() >= 3)
        .map(|(label, aggregate)| Cohort {
            label: label.to_owned(),
            views: aggregate.views,
            visitors: usize_to_i64(aggregate.visitors.len()),
        })
        .collect();
    cohorts.sort_by(|left, right| {
        right
            .views
            .cmp(&left.views)
            .then_with(|| left.label.cmp(&right.label))
    });
    cohorts.truncate(limit);
    cohorts
}

fn aggregate_referrers(arrivals: &[&AnalyticsEvent]) -> Vec<Cohort> {
    let mut aggregates = BTreeMap::new();
    for event in arrivals {
        if let Some(host) = event.referrer_host.as_deref() {
            add_cohort(&mut aggregates, host, event.visitor_id.as_str());
        }
    }
    collect_cohorts(aggregates, 12)
}

fn aggregate_countries(events: &[&AnalyticsEvent]) -> Vec<Cohort> {
    let mut aggregates = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "pageview") {
        if let Some(country) = event.country_code.as_deref() {
            add_cohort(&mut aggregates, country, event.visitor_id.as_str());
        }
    }
    collect_cohorts(aggregates, 30)
}

fn aggregate_technology(events: &[&AnalyticsEvent]) -> Vec<Technology> {
    let mut aggregates: BTreeMap<(&str, &str), CohortAggregate<'_>> = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "pageview") {
        for (dimension, label) in [
            ("device", event.device_kind.as_str()),
            ("browser", event.browser.as_str()),
            ("os", event.operating_system.as_str()),
        ] {
            let aggregate = aggregates.entry((dimension, label)).or_default();
            aggregate.views += 1;
            aggregate.visitors.insert(event.visitor_id.as_str());
        }
    }

    let mut technology: Vec<Technology> = aggregates
        .into_iter()
        .filter(|(_, aggregate)| aggregate.visitors.len() >= 3)
        .map(|((dimension, label), aggregate)| Technology {
            dimension: dimension.to_owned(),
            label: label.to_owned(),
            views: aggregate.views,
            visitors: usize_to_i64(aggregate.visitors.len()),
        })
        .collect();
    technology.sort_by(|left, right| {
        left.dimension
            .cmp(&right.dimension)
            .then_with(|| right.views.cmp(&left.views))
            .then_with(|| left.label.cmp(&right.label))
    });
    technology
}

fn aggregate_hourly(events: &[&AnalyticsEvent]) -> [[i64; 24]; 7] {
    let mut grid = [[0; 24]; 7];
    for event in events.iter().filter(|event| event.kind == "pageview") {
        let (Some(weekday), Some(hour)) = (event.local_weekday, event.local_hour) else {
            continue;
        };
        let (Ok(weekday), Ok(hour)) = (usize::try_from(weekday), usize::try_from(hour)) else {
            continue;
        };
        if let Some(cell) = grid.get_mut(weekday).and_then(|day| day.get_mut(hour)) {
            *cell += 1;
        }
    }
    grid
}

fn aggregate_journeys(events: &[&AnalyticsEvent]) -> Vec<Journey> {
    let mut aggregates: BTreeMap<(&str, &str), CohortAggregate<'_>> = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "pageview") {
        let Some(from) = event.referrer_path.as_deref() else {
            continue;
        };
        if event.referrer_kind != "internal" || from == event.page_path {
            continue;
        }
        let aggregate = aggregates
            .entry((from, event.page_path.as_str()))
            .or_default();
        aggregate.views += 1;
        aggregate.visitors.insert(event.visitor_id.as_str());
    }

    let mut journeys: Vec<Journey> = aggregates
        .into_iter()
        .filter(|(_, aggregate)| aggregate.visitors.len() >= 3)
        .map(|((from, to), aggregate)| Journey {
            from: from.to_owned(),
            to: to.to_owned(),
            trips: aggregate.views,
            visitors: usize_to_i64(aggregate.visitors.len()),
        })
        .collect();
    journeys.sort_by(|left, right| {
        right
            .trips
            .cmp(&left.trips)
            .then_with(|| left.from.cmp(&right.from))
            .then_with(|| left.to.cmp(&right.to))
    });
    journeys.truncate(10);
    journeys
}

fn aggregate_outbound(events: &[&AnalyticsEvent]) -> Vec<Cohort> {
    let mut aggregates = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "outbound") {
        if let Some(host) = event.target_host.as_deref() {
            add_cohort(&mut aggregates, host, event.visitor_id.as_str());
        }
    }
    collect_cohorts(aggregates, 10)
}

fn aggregate_campaigns(arrivals: &[&AnalyticsEvent]) -> Vec<Campaign> {
    let mut aggregates: BTreeMap<(&str, &str), CohortAggregate<'_>> = BTreeMap::new();
    for event in arrivals {
        let Some(source) = event.utm_source.as_deref() else {
            continue;
        };
        let campaign = event.utm_campaign.as_deref().unwrap_or("(uncategorized)");
        let aggregate = aggregates.entry((source, campaign)).or_default();
        aggregate.views += 1;
        aggregate.visitors.insert(event.visitor_id.as_str());
    }

    let mut campaigns: Vec<Campaign> = aggregates
        .into_iter()
        .filter(|(_, aggregate)| aggregate.visitors.len() >= 3)
        .map(|((source, campaign), aggregate)| Campaign {
            source: source.to_owned(),
            campaign: campaign.to_owned(),
            views: aggregate.views,
            visitors: usize_to_i64(aggregate.visitors.len()),
        })
        .collect();
    campaigns.sort_by(|left, right| {
        right
            .views
            .cmp(&left.views)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.campaign.cmp(&right.campaign))
    });
    campaigns.truncate(10);
    campaigns
}

fn percent(numerator: usize, denominator: usize) -> i64 {
    positive_half_up(
        i128::try_from(numerator).expect("analytics count fits i128") * 100,
        denominator,
    )
}

fn positive_half_up(numerator: i128, denominator: usize) -> i64 {
    if denominator == 0 {
        return 0;
    }
    debug_assert!(numerator >= 0);
    let denominator = i128::try_from(denominator).expect("analytics count fits i128");
    let rounded = (numerator * 2 + denominator) / (denominator * 2);
    i64::try_from(rounded).expect("analytics aggregate fits i64")
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).expect("analytics count fits i64")
}

fn format_utc_day(day: i64) -> String {
    const SECONDS_PER_DAY: i64 = 86_400;

    jiff::Timestamp::from_second(
        day.checked_mul(SECONDS_PER_DAY)
            .expect("analytics day fits a timestamp"),
    )
    .expect("analytics day is a valid timestamp")
    .strftime("%Y-%m-%d")
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_parser_is_bounded_to_known_ranges() {
        assert_eq!(Window::parse(Some("7d")), Window::Week);
        assert_eq!(Window::parse(Some("365d")), Window::Year);
        assert_eq!(Window::parse(Some("all")), Window::Month);
        assert_eq!(Window::parse(None), Window::Month);
    }

    #[test]
    fn aggregation_preserves_rounding_nulls_arrivals_and_day_gaps() {
        let cutoff = 1_704_067_200;
        let mut events = vec![
            event("z", "v1", "s1", cutoff, "pageview", "/resume"),
            event("a", "v1", "s1", cutoff, "pageview", "/thoughts"),
            event("b", "v1", "s2", cutoff + 1, "pageview", "/resume"),
            event("c", "v2", "s3", cutoff + 2, "pageview", "/resume"),
            event("d", "v1", "s1", cutoff + 2, "engagement", "/resume"),
            event("e", "v1", "s1", cutoff + 2, "engagement", "/resume"),
            event("f", "v1", "s1", cutoff + 2, "engagement", "/resume"),
            event("g", "v1", "s1", cutoff + 2, "outbound", "/resume"),
            event("old", "v1", "s1", cutoff - 1, "pageview", "/resume"),
            event(
                "later",
                "v3",
                "s4",
                cutoff + 2 * 86_400,
                "pageview",
                "/resume",
            ),
        ];
        events[0].referrer_kind = "direct".to_string();
        events[1].referrer_kind = "search".to_string();
        events[8].referrer_kind = "social".to_string();
        events[4].engagement_seconds = Some(1);
        events[4].scroll_percent = Some(89);
        events[4].lcp_milliseconds = Some(100);
        events[5].engagement_seconds = Some(2);
        events[5].scroll_percent = Some(90);
        events[5].lcp_milliseconds = Some(101);
        events[6].engagement_seconds = None;
        events[6].scroll_percent = None;
        events[6].lcp_milliseconds = None;

        let dashboard = aggregate(&events, &HashSet::new(), cutoff, cutoff + 2 * 86_400);

        assert_eq!(dashboard.overview.pageviews, 5);
        assert_eq!(dashboard.overview.visitors, 3);
        assert_eq!(dashboard.overview.sessions, 4);
        assert_eq!(dashboard.overview.engaged_seconds, 3);
        assert_eq!(dashboard.overview.outbound_clicks, 1);
        assert_eq!(dashboard.overview.returning_percent, 33);
        assert_eq!(dashboard.overview.single_page_percent, 75);
        assert_eq!(dashboard.performance.attention_seconds, 2);
        assert_eq!(dashboard.performance.scroll_percent, 90);
        assert_eq!(dashboard.performance.finish_percent, 50);
        assert_eq!(dashboard.performance.lcp_milliseconds, 101);
        assert_eq!(dashboard.performance.samples, 3);
        assert_eq!(dashboard.channels.len(), 2);
        assert_eq!(dashboard.channels[0].label, "direct");
        assert_eq!(dashboard.channels[0].count, 3);
        assert_eq!(dashboard.channels[1].label, "search");
        assert_eq!(dashboard.channels[1].count, 1);
        assert_eq!(
            dashboard
                .days
                .iter()
                .map(|day| (day.date.as_str(), day.views))
                .collect::<Vec<_>>(),
            vec![("2024-01-01", 4), ("2024-01-02", 0), ("2024-01-03", 1)]
        );
    }

    #[test]
    fn public_aggregates_keep_fixed_pages_and_suppress_small_cohorts() {
        let cutoff = 1_704_067_200;
        let mut events = Vec::new();
        let cohorts = [
            ("v1", "s1", "/felix/public", "US", "good.example", "good"),
            ("v2", "s2", "/felix/public", "US", "good.example", "good"),
            ("v3", "s3", "/felix/public", "US", "good.example", "good"),
            ("v4", "s4", "/felix/private", "CA", "small.example", "small"),
            ("v5", "s5", "/felix/private", "CA", "small.example", "small"),
        ];
        for (index, (visitor, session, path, country, host, source)) in
            cohorts.into_iter().enumerate()
        {
            let mut pageview = event(
                &format!("p{index}"),
                visitor,
                session,
                cutoff + i64::try_from(index).unwrap(),
                "pageview",
                path,
            );
            pageview.referrer_kind = "internal".to_string();
            pageview.referrer_host = Some(host.to_string());
            pageview.referrer_path = Some("/thoughts".to_string());
            pageview.country_code = Some(country.to_string());
            pageview.device_kind = if source == "good" {
                "mobile"
            } else {
                "desktop"
            }
            .to_string();
            pageview.browser = if source == "good" {
                "Firefox"
            } else {
                "Chrome"
            }
            .to_string();
            pageview.operating_system =
                if source == "good" { "Linux" } else { "Windows" }.to_string();
            pageview.utm_source = Some(source.to_string());
            events.push(pageview);

            let mut outbound = event(
                &format!("o{index}"),
                visitor,
                session,
                cutoff + 10 + i64::try_from(index).unwrap(),
                "outbound",
                path,
            );
            outbound.target_host = Some(host.to_string());
            events.push(outbound);
        }
        events.push(event(
            "fixed",
            "solo",
            "solo",
            cutoff + 30,
            "pageview",
            "/resume",
        ));

        let dashboard = aggregate(&events, &HashSet::new(), cutoff, cutoff);

        assert!(dashboard.pages.iter().any(|page| page.path == "/resume"));
        assert!(
            dashboard
                .pages
                .iter()
                .any(|page| page.path == "/felix/public")
        );
        assert!(
            dashboard
                .pages
                .iter()
                .all(|page| page.path != "/felix/private")
        );
        assert_eq!(dashboard.referrers.len(), 1);
        assert_eq!(dashboard.referrers[0].label, "good.example");
        assert_eq!(dashboard.countries.len(), 1);
        assert_eq!(dashboard.countries[0].label, "US");
        assert!(
            dashboard
                .technology
                .iter()
                .all(|row| !["desktop", "Chrome", "Windows"].contains(&row.label.as_str()))
        );
        assert_eq!(dashboard.journeys.len(), 1);
        assert_eq!(dashboard.outbound.len(), 1);
        assert_eq!(dashboard.outbound[0].label, "good.example");
        assert_eq!(dashboard.campaigns.len(), 1);
        assert_eq!(dashboard.campaigns[0].source, "good");
    }

    #[test]
    fn a_session_that_started_before_the_window_is_not_an_arrival_again() {
        let cutoff = 1_704_067_200;
        let mut continued = event("a", "v1", "continued", cutoff, "pageview", "/resume");
        continued.referrer_kind = "search".to_string();
        let fresh = event("b", "v2", "fresh", cutoff + 1, "pageview", "/resume");

        let dashboard = aggregate(
            &[continued, fresh],
            &HashSet::from(["continued".to_string()]),
            cutoff,
            cutoff,
        );

        assert_eq!(dashboard.channels.len(), 1);
        assert_eq!(dashboard.channels[0].label, "direct");
        assert_eq!(dashboard.channels[0].count, 1);
    }

    #[test]
    fn visitor_day_facts_match_raw_for_every_dashboard_window() {
        let current = 1_735_776_000; // 2025-01-02 UTC
        let mut events = Vec::new();
        for offset in [0, 6, 29, 89, 364] {
            let at = current - offset * 86_400;
            for visitor in 0..3 {
                let mut pageview = event(
                    &format!("p-{offset}-{visitor}"),
                    &format!("v{visitor}"),
                    &format!("s-{offset}-{visitor}"),
                    at,
                    "pageview",
                    "/resume",
                );
                pageview.country_code = Some("US".into());
                pageview.local_weekday = Some(offset.rem_euclid(7));
                pageview.local_hour = Some(0);
                pageview.referrer_kind = "search".into();
                pageview.referrer_host = Some("search.example".into());
                pageview.utm_source = Some("test".into());
                pageview.utm_campaign = Some("exact".into());
                events.push(pageview);

                let mut engagement = event(
                    &format!("e-{offset}-{visitor}"),
                    &format!("v{visitor}"),
                    &format!("s-{offset}-{visitor}"),
                    at + 1,
                    "engagement",
                    "/resume",
                );
                engagement.engagement_seconds = Some(0);
                engagement.scroll_percent = Some(90 + visitor);
                engagement.lcp_milliseconds = Some(100 + visitor);
                events.push(engagement);
            }
        }

        // One session straddles midnight and two pageviews share a second;
        // event id is the deterministic acquisition tie-breaker.
        let boundary = current - 29 * 86_400;
        events.push(event(
            "z",
            "cross",
            "cross-session",
            boundary - 1,
            "pageview",
            "/",
        ));
        events.push(event(
            "b",
            "cross",
            "cross-session",
            boundary,
            "pageview",
            "/thoughts",
        ));
        events.push(event(
            "a",
            "cross",
            "cross-session",
            boundary,
            "pageview",
            "/resume",
        ));

        let mut by_visitor_day: BTreeMap<(String, i64), Vec<AnalyticsEvent>> = BTreeMap::new();
        for event in events.iter().cloned() {
            by_visitor_day
                .entry((
                    event.visitor_id.clone(),
                    event.occurred_at.div_euclid(86_400),
                ))
                .or_default()
                .push(event);
        }
        let facts: Vec<AnalyticsVisitorDay> = by_visitor_day
            .into_iter()
            .map(|((visitor, day), rows)| analytics_facts::compact(day, &visitor, rows).unwrap())
            .collect();

        for days in [7, 30, 90, 365] {
            let cutoff = current - (days - 1) * 86_400;
            let prior_floor = cutoff - super::super::db::SESSION_IDLE_SECONDS;
            let current_sessions: HashSet<&str> = events
                .iter()
                .filter(|event| event.kind == "pageview" && event.occurred_at >= cutoff)
                .map(|event| event.session_id.as_str())
                .collect();
            let prior: HashSet<String> = events
                .iter()
                .filter(|event| {
                    event.kind == "pageview"
                        && event.occurred_at < cutoff
                        && event.occurred_at >= prior_floor
                        && current_sessions.contains(event.session_id.as_str())
                })
                .map(|event| event.session_id.clone())
                .collect();
            let raw = aggregate(&events, &prior, cutoff, current);
            let from_facts = aggregate_facts(&facts, cutoff, current).unwrap();
            assert_eq!(from_facts, raw, "{days}-day differential");
        }
    }

    fn event(
        id: &str,
        visitor_id: &str,
        session_id: &str,
        occurred_at: i64,
        kind: &str,
        page_path: &str,
    ) -> AnalyticsEvent {
        AnalyticsEvent {
            id: id.to_string(),
            visitor_id: visitor_id.to_string(),
            session_id: session_id.to_string(),
            occurred_at,
            kind: kind.to_string(),
            page_path: page_path.to_string(),
            referrer_kind: "direct".to_string(),
            referrer_host: None,
            referrer_path: None,
            country_code: None,
            timezone: None,
            language: None,
            device_kind: "unknown".to_string(),
            browser: "Other".to_string(),
            operating_system: "Other".to_string(),
            viewport_kind: "unknown".to_string(),
            navigation_kind: None,
            local_hour: None,
            local_weekday: None,
            engagement_seconds: None,
            scroll_percent: None,
            lcp_milliseconds: None,
            cls_thousandths: None,
            navigation_milliseconds: None,
            target_host: None,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
        }
    }
}
