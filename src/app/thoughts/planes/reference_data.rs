use super::format::round_count;

// The sacrifice chart: each bar is a year of an ordinary thing, decomposed
// into slices. A slice is either a floor you keep no matter what (cut: None)
// or the piece a specific sacrifice carves off. Slices are ordered from the
// baseline outward: the free end of the bar is shaved first (the mildest
// cut), and each deeper cut takes its own slice plus everything outside it —
// so a bar's cuttable total is the sum of its non-floor slices.
//
// Colors are `--slice-*` tokens declared in styles/planes-tokens.css (so
// themes can restyle the bars); within a bar, darker = the deeper cut.

#[derive(Debug, Clone, Copy)]
pub struct FlightAnalogy {
    #[allow(dead_code)] // React list key in the original; kept for data parity
    pub id: &'static str,
    pub kg_per_unit: f64,
    /// Dashed-line label template; '{n}' becomes the friendly count, e.g. 'eating ≈{n} hamburgers'.
    pub tick: &'static str,
    /// Fuller unit for tooltips and the data table, e.g. "miles in a gas car".
    pub unit_label: &'static str,
    /// Largest count this unit reads naturally at ("130 months" should be years).
    pub max_count: Option<f64>,
    pub basis: &'static str,
    pub source_ids: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct CutSlice {
    pub id: &'static str,
    /// The sacrifice that removes this slice, or None for a floor you keep.
    pub cut: Option<&'static str>,
    /// Tooltip copy: what this slice is and what it takes to erase it.
    pub label: &'static str,
    pub kg: f64,
    pub color: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct CutOption {
    pub id: &'static str,
    /// Chip label with its price, e.g. "go vegan −1.0 t".
    pub label: &'static str,
    /// Ids of the slices this cut erases.
    pub slice_ids: &'static [&'static str],
    /// Independent ladders within one bar (e.g. 'diet' vs. 'waste'): one option
    /// can be active per group. None = the bar's single ladder.
    pub group: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct SacrificeBar {
    pub id: &'static str,
    /// The year being dissected, e.g. "A year of eating".
    pub noun: &'static str,
    pub detail: &'static str,
    /// Baseline-first; the last slice sits at the bar's free end.
    pub slices: &'static [CutSlice],
    /// The cuts on offer, mildest first within each group; one applies per group.
    pub options: &'static [CutOption],
    /// Ways to quote this flight in the bar's own currency — the dashed line's
    /// label picks one per flight via pick_analogy().
    pub analogies: &'static [FlightAnalogy],
    pub source_ids: &'static [&'static str],
}

/// The floor color — deliberately neutral, near the paper: not actionable,
/// not data you can cut.
pub const FLOOR_COLOR: &str = "var(--slice-floor)";

pub static SACRIFICE_BARS: &[SacrificeBar] = &[
    SacrificeBar {
        id: "climate",
        noun: "A year of indoor climate",
        detail: "a gas-furnace winter, then ≈800 hours of summer A/C",
        slices: &[
            CutSlice {
                id: "heat-pump-floor",
                label: "Modern heat pump keeping a home warm for the winter",
                cut: Some("no climate control"),
                kg: 1300.0,
                color: FLOOR_COLOR,
            },
            CutSlice {
                id: "gas-premium",
                cut: Some("switching to a heat pump"),
                label: "The gas premium: what the furnace burns beyond a heat pump’s share of the \
                        same warmth",
                kg: 840.0,
                color: "var(--slice-heat-deep)",
            },
            CutSlice {
                id: "thermostat",
                cut: Some("turning it down 3 °F"),
                label: "nudging the thermostat down ≈3°F (DOE’s rule of thumb: ≈1% per °F)",
                kg: 240.0,
                color: "var(--slice-heat-mild)",
            },
            CutSlice {
                id: "ac-all",
                cut: Some("sweating out the summer"),
                label: "800 A/C hours × ≈1.1 kg/hour (3 kWh at the 0.37 \
                        kg/kWh U.S. grid average)",
                kg: 800.0 * 1.1,
                color: "var(--slice-ac)",
            },
        ],
        options: &[
            CutOption {
                id: "thermostat",
                label: "turn it down 3 °F −0.2 t",
                slice_ids: &["thermostat"],
                group: Some("heat"),
            },
            CutOption {
                id: "heat-pump",
                label: "switch to a heat pump −1.1 t",
                slice_ids: &["gas-premium", "thermostat"],
                group: Some("heat"),
            },
            CutOption {
                id: "no-climate-control",
                label: "use no climate control at all −3.3 t",
                slice_ids: &["gas-premium", "thermostat", "heat-pump-floor", "ac-all"],
                group: Some("heat"),
            },
            CutOption {
                id: "sweat",
                label: "sweat out the summer −0.9 t",
                slice_ids: &["ac-all"],
                group: Some("cool"),
            },
        ],
        analogies: &[
            FlightAnalogy {
                id: "furnace-months",
                kg_per_unit: 400.0,
                tick: "keeping the furnace on for ≈{n} winter months",
                unit_label: "months of a typical gas-furnace season",
                max_count: Some(12.0),
                basis: "a ≈450-therm heating season spread over ≈6 cold months (US EIA RECS 2020)",
                source_ids: &["heating"],
            },
            FlightAnalogy {
                id: "winters",
                kg_per_unit: 2380.0,
                tick: "heating your home for ≈{n} winters",
                unit_label: "gas-furnace winters",
                max_count: None,
                basis: "a ≈450-therm season ≈ 2.4 t CO₂e (US EIA RECS 2020 average for gas-heated \
                        homes)",
                source_ids: &["heating"],
            },
            FlightAnalogy {
                id: "therms",
                kg_per_unit: 5.3,
                tick: "burning ≈{n} therms of natural gas",
                unit_label: "therms of natural gas, burned",
                max_count: None,
                basis: "≈5.3 kg CO₂ per therm of natural gas",
                source_ids: &["heating"],
            },
            FlightAnalogy {
                id: "showers",
                kg_per_unit: 1.7,
                tick: "taking ≈{n} long hot showers",
                unit_label: "≈15-minute electric power showers",
                max_count: None,
                basis: "≈1.7 kg CO₂e per 15-minute electric power shower (Berners-Lee)",
                source_ids: &["items"],
            },
            FlightAnalogy {
                id: "ac-days",
                kg_per_unit: 24.0 * 1.1,
                tick: "A/C running day and night for ≈{n} days",
                unit_label: "days of central A/C running around the clock",
                max_count: Some(60.0),
                basis: "24 h/day at ≈1.1 kg CO₂e/hour",
                source_ids: &["grid"],
            },
            FlightAnalogy {
                id: "ac-months",
                kg_per_unit: 30.0 * 24.0 * 1.1,
                tick: "A/C running day and night for ≈{n} months",
                unit_label: "months of central A/C running around the clock",
                max_count: Some(12.0),
                basis: "24 h/day at ≈1.1 kg CO₂e/hour",
                source_ids: &["grid"],
            },
            FlightAnalogy {
                id: "ac-summers",
                kg_per_unit: 800.0 * 1.1,
                tick: "running the A/C for ≈{n} summers",
                unit_label: "≈800-hour summers of central A/C",
                max_count: None,
                basis: "≈800 hours a season at ≈1.1 kg CO₂e/hour",
                source_ids: &["grid"],
            },
        ],
        source_ids: &["heating", "heatpump", "grid"],
    },
    SacrificeBar {
        id: "eating",
        noun: "A year's worth of food",
        detail: "assuming you eat a typical American diet, including typical food waste",
        slices: &[
            CutSlice {
                id: "vegan-floor",
                cut: None,
                label: "1,095 meals vegan meals × ≈1 kg (Scarborough et al. 2014). Broadly, the most efficient diet is a vegan one.",
                kg: 1095.0 * 0.96,
                color: FLOOR_COLOR,
            },
            CutSlice {
                id: "veg-to-vegan",
                cut: Some("going vegan"),
                label: "Vegetarian → vegan buys this middle slice: ≈0.3 kg × 1,095 meals \
                        (Scarborough et al. 2014)",
                kg: 1095.0 * (1.27 - 0.96),
                color: "var(--slice-diet-deep)",
            },
            CutSlice {
                id: "avg-to-veg",
                cut: Some("going vegetarian"),
                label: "Omnivore → vegetarian shaves this slice: ≈0.6 kg × 1,095 meals \
                        (Scarborough et al. 2014)",
                kg: 1095.0 * (1.88 - 1.27),
                color: "var(--slice-diet-mild)",
            },
            CutSlice {
                id: "farming",
                cut: Some("wasting nothing"),
                label: "The farming that grew the food you toss — ≈¾ of wasted food’s footprint.",
                kg: 1095.0 * 0.34 * 0.75,
                color: "var(--slice-waste-deep)",
            },
            CutSlice {
                id: "methane",
                cut: Some("composting"),
                label: "Landfill methane — ≈¼ of wasted food’s footprint (EPA). Composting \
                        removes the bulk of this, but farming emissions and all still spent",
                kg: 1095.0 * 0.34 * 0.25,
                color: "var(--slice-waste-mild)",
            },
        ],
        options: &[
            CutOption {
                id: "vegetarian",
                label: "go vegetarian −0.7 t",
                slice_ids: &["avg-to-veg"],
                group: Some("diet"),
            },
            CutOption {
                id: "vegan",
                label: "go vegan −1.0 t",
                slice_ids: &["veg-to-vegan", "avg-to-veg"],
                group: Some("diet"),
            },
            CutOption {
                id: "compost",
                label: "compost −0.09 t",
                slice_ids: &["methane"],
                group: Some("waste"),
            },
            CutOption {
                id: "zero-waste",
                label: "waste nothing −0.37 t",
                slice_ids: &["farming", "methane"],
                group: Some("waste"),
            },
        ],
        analogies: &[
            FlightAnalogy {
                id: "meal-months",
                kg_per_unit: (1095.0 * 1.88) / 12.0,
                tick: "every meal for ≈{n} months, average diet",
                unit_label: "months of meals on an average (medium-meat) diet",
                max_count: Some(21.0),
                basis: "Scarborough et al. 2014: ≈1.9 kg CO₂e/meal at 3 meals/day",
                source_ids: &["diets"],
            },
            FlightAnalogy {
                id: "burgers",
                kg_per_unit: 3.6,
                tick: "eating ≈{n} hamburgers",
                unit_label: "quarter-pound hamburgers",
                max_count: None,
                basis: "≈3.6 kg CO₂e each — a 113 g patty at ≈33 kg/kg, the U.S.-typical (feedlot \
                        + dairy-herd) beef intensity; Poore & Nemecek’s global beef-herd mean \
                        would read ≈11 kg",
                source_ids: &["meat"],
            },
            FlightAnalogy {
                id: "chickens",
                kg_per_unit: 6.0,
                tick: "eating ≈{n} rotisserie chickens",
                unit_label: "whole rotisserie chickens",
                max_count: None,
                basis: "≈6 kg CO₂e per bird — ≈0.65 kg of meat at poultry’s ≈10 kg/kg (Poore & \
                        Nemecek 2018, per-kg data via Our World in Data)",
                source_ids: &["meat"],
            },
            FlightAnalogy {
                id: "burger-daily",
                kg_per_unit: 365.0 * 3.6,
                tick: "a hamburger a day for ≈{n} years",
                unit_label: "years of a hamburger-a-day habit",
                max_count: Some(20.0),
                basis: "365 quarter-pounders a year at ≈3.6 kg CO₂e each (U.S.-typical beef \
                        intensity, Poore & Nemecek 2018)",
                source_ids: &["meat"],
            },
            FlightAnalogy {
                id: "binned-meals",
                kg_per_unit: 0.34,
                tick: "scraping ≈{n} meals into the bin",
                unit_label: "meals’ worth of typical food waste",
                max_count: None,
                basis: "≈0.34 kg CO₂e wasted per meal — the consumer-stage share of North \
                        America’s FAO food-wastage footprint",
                source_ids: &["foodwaste"],
            },
            FlightAnalogy {
                id: "waste-years",
                kg_per_unit: 1095.0 * 0.34,
                tick: "a year of typical food waste, ≈{n} times over",
                unit_label: "years of one person’s typical food waste",
                max_count: None,
                basis: "1,095 meals a year at ≈0.34 kg CO₂e wasted each (consumer stage, \
                        FAO-based)",
                source_ids: &["foodwaste"],
            },
        ],
        source_ids: &["diets", "foodwaste", "compost"],
    },
    SacrificeBar {
        id: "fashion",
        noun: "Constantly buying fast fashion for a year",
        detail: "≈2 new garments a week; the U.S. average is ≈1",
        slices: &[
            CutSlice {
                id: "basics",
                cut: None,
                label: "New socks and underwear: call it ≈30 kg a year — a few unavoidable new \
                        basics at the assumed ≈10 kg each — even a full-thrift wardrobe buys \
                        these new",
                kg: 30.0,
                color: FLOOR_COLOR,
            },
            CutSlice {
                id: "average-wardrobe",
                cut: Some("thrifting nearly everything"),
                label: "The average shopper’s year: ≈53 new garments (US PIRG) at ≈10 kg each \
                        (our assumed average — between a ≈7 kg tee and ≈33 kg jeans; sector \
                        totals imply more) — going almost all-secondhand erases this",
                kg: 530.0,
                color: "var(--slice-fashion-deep)",
            },
            CutSlice {
                id: "fast-premium",
                cut: Some("dropping to the average pace"),
                label: "The fast-fashion premium: the second new garment every week — ≈51 more \
                        pieces at ≈10 kg each",
                kg: 510.0,
                color: "var(--slice-fashion-mild)",
            },
        ],
        options: &[
            CutOption {
                id: "slow-down",
                label: "drop fast fashion −0.5 t",
                slice_ids: &["fast-premium"],
                group: None,
            },
            CutOption {
                id: "thrift",
                label: "thrift nearly everything −1.0 t",
                slice_ids: &["average-wardrobe", "fast-premium"],
                group: None,
            },
        ],
        analogies: &[
            FlightAnalogy {
                id: "jeans",
                kg_per_unit: 33.0,
                tick: "buying ≈{n} new pairs of jeans",
                unit_label: "new pairs of jeans",
                max_count: None,
                basis: "Levi’s lifecycle assessment: ≈33 kg CO₂e per pair of 501s",
                source_ids: &["jeans"],
            },
            FlightAnalogy {
                id: "tees",
                kg_per_unit: 7.0,
                tick: "buying ≈{n} new cotton T-shirts",
                unit_label: "new cotton T-shirts",
                max_count: None,
                basis: "≈7 kg CO₂e per cotton tee (typical of published industry LCAs)",
                source_ids: &["fashion"],
            },
            FlightAnalogy {
                id: "wardrobe-years",
                kg_per_unit: 530.0,
                tick: "an average wardrobe year, ≈{n} times over",
                unit_label: "years of average-pace clothes shopping",
                max_count: Some(20.0),
                basis: "≈53 new garments a year (US PIRG 2024) at an assumed ≈10 kg CO₂e each",
                source_ids: &["pirg", "fashion"],
            },
        ],
        source_ids: &["fashion", "pirg", "jeans"],
    },
];

/// The rounding errors: the four famous little habits, one bar each. At the
/// main chart's scale all four together are a sliver; the zoomed panel
/// redraws them at their own scale so they can be seen — and cut — one by one.
pub static HABIT_BARS: &[SacrificeBar] = &[
    SacrificeBar {
        id: "coffee",
        noun: "A latte every morning",
        detail: "365 large lattes a year",
        slices: &[
            CutSlice {
                id: "beans",
                cut: Some("quitting coffee outright"),
                label: "The coffee itself: 365 black coffees ≈ 8 kg (Berners-Lee: ≈21 g each) — \
                        only quitting clears it",
                kg: 365.0 * 0.021,
                color: "var(--slice-habit-deep)",
            },
            CutSlice {
                id: "milk",
                cut: Some("taking it black"),
                label: "The milk: a large latte runs ≈340 g against black coffee’s ≈21 g \
                        (Berners-Lee) — switch to black and this slice is gone",
                kg: 365.0 * (0.34 - 0.021),
                color: "var(--slice-habit-mild)",
            },
        ],
        options: &[
            CutOption {
                id: "black",
                label: "take it black −116 kg",
                slice_ids: &["milk"],
                group: None,
            },
            CutOption {
                id: "quit",
                label: "quit coffee −124 kg",
                slice_ids: &["beans", "milk"],
                group: None,
            },
        ],
        analogies: &[
            FlightAnalogy {
                id: "lattes",
                kg_per_unit: 0.34,
                tick: "drinking ≈{n} large lattes",
                unit_label: "large lattes",
                max_count: None,
                basis: "Berners-Lee: ≈340 g CO₂e per large latte, mostly the milk",
                source_ids: &["items"],
            },
            FlightAnalogy {
                id: "latte-years",
                kg_per_unit: 365.0 * 0.34,
                tick: "a daily latte for ≈{n} years",
                unit_label: "years of a latte-a-morning habit",
                max_count: None,
                basis: "365 large lattes a year at ≈340 g CO₂e each (Berners-Lee)",
                source_ids: &["items"],
            },
            FlightAnalogy {
                id: "black-years",
                kg_per_unit: 365.0 * 0.021,
                tick: "a daily black coffee for ≈{n} years",
                unit_label: "years of a black-coffee-a-day habit",
                max_count: None,
                basis: "365 black coffees a year at ≈21 g CO₂e each (Berners-Lee)",
                source_ids: &["items"],
            },
        ],
        source_ids: &["items"],
    },
    SacrificeBar {
        id: "phone",
        noun: "A new iPhone every year",
        detail: "one flagship upgrade, annually",
        slices: &[CutSlice {
            id: "handset",
            cut: Some("keeping the old phone"),
            label: "One flagship phone ≈70 kg CO₂e, ≈80% of it manufacturing (Apple’s own \
                    lifecycle reports)",
            kg: 70.0,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "keep",
            label: "keep the old phone −70 kg",
            slice_ids: &["handset"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "iphones",
                kg_per_unit: 70.0,
                tick: "buying ≈{n} brand-new iPhones",
                unit_label: "brand-new flagship phones",
                max_count: None,
                basis: "Apple lifecycle reports: ≈70 kg CO₂e per flagship phone",
                source_ids: &["phone"],
            },
            FlightAnalogy {
                id: "upgrade-years",
                kg_per_unit: 70.0,
                tick: "a yearly phone upgrade for ≈{n} years",
                unit_label: "years of an annual phone upgrade",
                max_count: None,
                basis: "one ≈70 kg CO₂e flagship per year (Apple lifecycle reports)",
                source_ids: &["phone"],
            },
        ],
        source_ids: &["phone"],
    },
    SacrificeBar {
        id: "soda",
        noun: "A daily Diet Coke",
        detail: "365 cans a year",
        slices: &[CutSlice {
            id: "cans",
            cut: Some("quitting the daily can"),
            label: "365 cans of Diet Coke × ≈150 g (Carbon Trust LCA with Coca-Cola)",
            kg: 365.0 * 0.15,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "quit",
            label: "quit the can −55 kg",
            slice_ids: &["cans"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "cans",
                kg_per_unit: 0.15,
                tick: "drinking ≈{n} cans of Diet Coke",
                unit_label: "cans of Diet Coke",
                max_count: None,
                basis: "Carbon Trust LCA: ≈150 g CO₂e per 330 ml can",
                source_ids: &["soda"],
            },
            FlightAnalogy {
                id: "habit-years",
                kg_per_unit: 365.0 * 0.15,
                tick: "a Diet Coke every day for ≈{n} years",
                unit_label: "years of a can-a-day Diet Coke habit",
                max_count: None,
                basis: "365 cans a year at ≈150 g CO₂e each (Carbon Trust LCA)",
                source_ids: &["soda"],
            },
        ],
        source_ids: &["soda"],
    },
    SacrificeBar {
        id: "bottled-water",
        noun: "Bottled water, daily",
        detail: "365 half-liter bottles a year",
        slices: &[CutSlice {
            id: "bottles",
            cut: Some("quitting bottled water"),
            label: "365 half-liter bottles × ≈160 g (Berners-Lee)",
            kg: 365.0 * 0.16,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "quit",
            label: "carry a flask −58 kg",
            slice_ids: &["bottles"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "bottles",
                kg_per_unit: 0.16,
                tick: "drinking ≈{n} bottles of water",
                unit_label: "half-liter plastic bottles",
                max_count: None,
                basis: "≈160 g CO₂e each, Berners-Lee",
                source_ids: &["items"],
            },
            FlightAnalogy {
                id: "habit-years",
                kg_per_unit: 365.0 * 0.16,
                tick: "bottled water daily for ≈{n} years",
                unit_label: "years of a bottle-a-day habit",
                max_count: None,
                basis: "365 bottles a year at ≈160 g CO₂e each (Berners-Lee)",
                source_ids: &["items"],
            },
        ],
        source_ids: &["items"],
    },
    SacrificeBar {
        id: "streaming",
        noun: "Streaming, two hours a night",
        detail: "≈730 hours of video a year",
        slices: &[CutSlice {
            id: "hours",
            cut: Some("reading a book instead"),
            label: "730 hours × ≈55 g CO₂e per streaming hour (Carbon Trust, European grid \
                    average)",
            kg: 730.0 * 0.055,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "quit",
            label: "read a book instead −40 kg",
            slice_ids: &["hours"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "hours",
                kg_per_unit: 0.055,
                tick: "streaming ≈{n} hours of video",
                unit_label: "hours of streamed video",
                max_count: None,
                basis: "Carbon Trust: ≈55 g CO₂e per streaming hour, European average",
                source_ids: &["streaming"],
            },
            FlightAnalogy {
                id: "habit-years",
                kg_per_unit: 730.0 * 0.055,
                tick: "two hours of streaming a night for ≈{n} years",
                unit_label: "years of two-hours-a-night streaming",
                max_count: None,
                basis: "730 hours a year at ≈55 g CO₂e each (Carbon Trust)",
                source_ids: &["streaming"],
            },
        ],
        source_ids: &["streaming"],
    },
    SacrificeBar {
        id: "chatgpt",
        noun: "ChatGPT, 30 queries a day",
        detail: "10,950 queries a year",
        slices: &[CutSlice {
            id: "queries",
            cut: Some("logging off"),
            label: "10,950 queries × ≈0.13 g (OpenAI’s own figure: ≈0.34 Wh per query at the U.S. \
                    grid average)",
            kg: 10950.0 * 0.00013,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "quit",
            label: "log off −1.4 kg",
            slice_ids: &["queries"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "vibe-coding",
                kg_per_unit: 8.8,
                tick: "vibe coding this site ≈{n} times",
                unit_label: "vibe-codings of this site",
                max_count: None,
                basis: "one build ≈60 h of Claude (Fable 5, max effort): ≈1.5M tokens generated + \
                        ≈80M re-read, nearly all prompt-cache hits ≈6 kWh ≈2.2 kg CO₂e at the \
                        U.S. grid average",
                source_ids: &["ai-agent", "grid"],
            },
            FlightAnalogy {
                id: "habit-years",
                kg_per_unit: 10950.0 * 0.00013,
                tick: "thirty ChatGPT queries a day for ≈{n} years",
                unit_label: "years of thirty ChatGPT queries a day",
                max_count: None,
                basis: "10,950 queries a year at ≈0.13 g CO₂e each (OpenAI’s own figure)",
                source_ids: &["ai-openai"],
            },
        ],
        source_ids: &["ai-openai"],
    },
    SacrificeBar {
        id: "straws",
        noun: "A plastic straw a day",
        detail: "365 straws a year",
        slices: &[CutSlice {
            id: "straws",
            cut: Some("sipping from the glass"),
            label: "365 plastic straws × ≈1.5 g (polypropylene-straw LCAs)",
            kg: 365.0 * 0.0015,
            color: "var(--slice-habit-deep)",
        }],
        options: &[CutOption {
            id: "quit",
            label: "skip the straw −0.5 kg",
            slice_ids: &["straws"],
            group: None,
        }],
        analogies: &[
            FlightAnalogy {
                id: "straws",
                kg_per_unit: 0.0015,
                tick: "sipping through ≈{n} plastic straws",
                unit_label: "plastic straws",
                max_count: None,
                basis: "≈1.5 g CO₂e each, per lifecycle studies of polypropylene straws",
                source_ids: &["straw-lca"],
            },
            FlightAnalogy {
                id: "habit-years",
                kg_per_unit: 365.0 * 0.0015,
                tick: "a straw a day for ≈{n} years",
                unit_label: "years of a straw-a-day habit",
                max_count: None,
                basis: "365 straws a year at ≈1.5 g CO₂e each (straw LCAs)",
                source_ids: &["straw-lca"],
            },
        ],
        source_ids: &["straw-lca"],
    },
];

/// Quote the flight in one of a bar's currencies. Prefers the analogy whose
/// friendly rounding lands closest to the truth, with a bonus for
/// one-significant-figure counts ("7,000 gas miles" beats "6,900"); near-ties
/// break pseudo-randomly, seeded by the flight, so different flights get
/// quoted in different units.
pub fn pick_analogy(
    analogies: &[FlightAnalogy],
    flight_kg: f64,
    seed: i64,
) -> (&FlightAnalogy, f64) {
    // A quote under 2 units ("≈ 1 summer of A/C" for half a summer) misleads,
    // and one past the unit's natural range ("130 months of waste") reads odd.
    let pool: Vec<&FlightAnalogy> = analogies
        .iter()
        .filter(|a| {
            let count = flight_kg / a.kg_per_unit;
            count >= 2.0 && count <= a.max_count.unwrap_or(f64::INFINITY)
        })
        .collect();
    let candidates: Vec<&FlightAnalogy> = if pool.is_empty() {
        analogies.iter().collect()
    } else {
        pool
    };
    let mut scored: Vec<(&FlightAnalogy, f64, f64)> = candidates
        .into_iter()
        .map(|analogy| {
            let count = flight_kg / analogy.kg_per_unit;
            let rounded = round_count(count);
            let magnitude = 10f64.powi(rounded.log10().floor() as i32);
            let one_sig = (rounded / magnitude).fract() == 0.0;
            let score = (rounded - count).abs() / count - if one_sig { 0.03 } else { 0.0 };
            (analogy, count, score)
        })
        .collect();
    scored.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    let ties: Vec<&(&FlightAnalogy, f64, f64)> = scored
        .iter()
        .filter(|c| c.2 <= scored[0].2 + 0.02)
        .collect();
    let &(analogy, count, _) = ties[(seed.unsigned_abs() as usize) % ties.len()];
    (analogy, count)
}

/// Everything a bar's sacrifices can remove — its deepest cut.
pub fn cuttable_kg(bar: &SacrificeBar) -> f64 {
    bar.slices
        .iter()
        .fold(0.0, |sum, s| sum + if s.cut.is_some() { s.kg } else { 0.0 })
}

/// The bar's full height: floor plus everything cuttable.
pub fn total_kg(bar: &SacrificeBar) -> f64 {
    bar.slices.iter().fold(0.0, |sum, s| sum + s.kg)
}

#[cfg(test)]
mod tests {
    use super::super::sources::source;
    use super::*;

    fn every_bar() -> impl Iterator<Item = &'static SacrificeBar> {
        SACRIFICE_BARS.iter().chain(HABIT_BARS.iter())
    }

    #[test]
    fn every_source_id_resolves() {
        // cite() panics at render on an unknown id; catch it here instead.
        let mut ids: Vec<(&str, &str)> = Vec::new();
        for bar in every_bar() {
            ids.extend(bar.source_ids.iter().map(|s| (bar.id, *s)));
            for an in bar.analogies {
                ids.extend(an.source_ids.iter().map(|s| (an.id, *s)));
            }
        }
        for (owner, id) in ids {
            assert!(source(id).is_some(), "{owner} cites unknown source {id:?}");
        }
    }

    #[test]
    fn options_name_real_cuttable_slices() {
        // charts::option_kg sums the named slices; an empty or dangling
        // slice_ids list renders a zero-priced or panicking chip.
        for bar in every_bar() {
            for opt in bar.options {
                assert!(
                    !opt.slice_ids.is_empty(),
                    "{}:{} has no slices",
                    bar.id,
                    opt.id
                );
                for sid in opt.slice_ids {
                    let slice = bar.slices.iter().find(|s| s.id == *sid);
                    let slice = slice.unwrap_or_else(|| {
                        panic!("{}:{} names unknown slice {sid:?}", bar.id, opt.id)
                    });
                    assert!(
                        slice.cut.is_some(),
                        "{}:{} names uncuttable slice {sid:?}",
                        bar.id,
                        opt.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_bar_has_analogies_and_positive_slices() {
        // pick_analogy indexes [0]; slices divide by kg in width math.
        for bar in every_bar() {
            assert!(!bar.analogies.is_empty(), "{} has no analogies", bar.id);
            for an in bar.analogies {
                assert!(an.kg_per_unit > 0.0, "{}:{} kg_per_unit", bar.id, an.id);
            }
            for slice in bar.slices {
                assert!(slice.kg > 0.0, "{}:{} kg", bar.id, slice.id);
            }
        }
    }

    #[test]
    fn wardrobe_slice_matches_its_own_label_and_analogy() {
        // "≈53 garments at ≈10 kg each" = 530; the wardrobe-years analogy
        // quotes the same year. These drifted once (500 vs 530).
        let fashion = every_bar().find(|b| b.id == "fashion").unwrap();
        let slice = fashion
            .slices
            .iter()
            .find(|s| s.id == "average-wardrobe")
            .unwrap();
        let analogy = fashion
            .analogies
            .iter()
            .find(|a| a.id == "wardrobe-years")
            .unwrap();
        assert_eq!(slice.kg, analogy.kg_per_unit);
    }

    #[test]
    fn bar_and_slice_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for bar in every_bar() {
            assert!(seen.insert(bar.id), "duplicate bar id {:?}", bar.id);
            let mut slice_ids = std::collections::HashSet::new();
            for s in bar.slices {
                assert!(
                    slice_ids.insert(s.id),
                    "{}: duplicate slice {:?}",
                    bar.id,
                    s.id
                );
            }
        }
    }
}
