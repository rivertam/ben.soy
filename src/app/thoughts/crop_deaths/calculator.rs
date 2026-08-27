//! The crop-death claim calculator.
//!
//! Yield is a sourced farm-gate input. Deaths per hectare per crop year are an
//! illustrated range, with a few labeled samples showing the assumptions behind
//! different points. The receipt translates one represented animal death into
//! lifestyle meals (bowls of pasta, a feed-grain cheeseburger, cans of Coke)—not
//! a land-area story.

use topcoat::{
    Result,
    view::{component, view},
};

use crate::components::{ext_link, inline_popover};

mod recipes;

use recipes::{
    AVOCADO_RECIPE, BAGUETTE_RECIPE, BURGER_RECIPE, COKE_RECIPE, CUP_RECIPE, ConversionRecipe,
    DONUT_RECIPE, ESPRESSO_RECIPE, GUAC_RECIPE, PASTA_RECIPE, RecipeComponent, RecipeFactor,
    TEA_RECIPE, TOAST_RECIPE,
};

const MIN_VALUE: f64 = 0.001;
const MAX_RATE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RangeSample {
    rate: f64,
    label: &'static str,
    trail_prefix: &'static str,
    citation: Option<SourceCitation>,
    trail_suffix: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceCitation {
    id: &'static str,
    label: &'static str,
    paragraphs: &'static [&'static str],
    sources: &'static [SourceLink],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceLink {
    label: &'static str,
    url: &'static str,
}

/// One everyday eating unit. Its crop mass is evaluated from a visible recipe.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LifestyleMeal {
    key: &'static str,
    singular: &'static str,
    plural: &'static str,
    recipe: ConversionRecipe,
}

impl LifestyleMeal {
    fn crop_kg(self) -> f64 {
        self.recipe.grams() / 1_000.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scenario {
    key: &'static str,
    tab: &'static str,
    crop: &'static str,
    crop_unit: &'static str,
    animal_singular: &'static str,
    animal_plural: &'static str,
    region: &'static str,
    yield_kg_per_hectare: f64,
    meals: &'static [LifestyleMeal],
    yield_source: &'static str,
    yield_url: &'static str,
    range_min: f64,
    range_max: f64,
    samples: &'static [RangeSample],
}

impl Scenario {
    fn default_rate(self) -> f64 {
        self.samples[1].rate
    }

    fn matching_sample(self, rate: f64) -> Option<RangeSample> {
        self.samples
            .iter()
            .copied()
            .find(|sample| (sample.rate - rate).abs() < 1e-9)
    }

    fn meal(self, key: Option<&str>) -> LifestyleMeal {
        key.and_then(|wanted| self.meals.iter().copied().find(|meal| meal.key == wanted))
            .unwrap_or(self.meals[0])
    }

    fn meals_json(self) -> String {
        let mut out = String::from("[");
        for (index, meal) in self.meals.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                r#"{{"key":"{}","singular":"{}","plural":"{}","cropKg":{}}}"#,
                meal.key,
                json_escape(meal.singular),
                json_escape(meal.plural),
                meal.crop_kg(),
            ));
        }
        out.push(']');
        out
    }

    fn calculate(self, deaths_per_hectare: f64) -> Calculation {
        let hectares = 1.0 / deaths_per_hectare;
        let food_kg = hectares * self.yield_kg_per_hectare;
        Calculation { hectares, food_kg }
    }
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

const AVOCADO_MEALS: &[LifestyleMeal] = &[
    LifestyleMeal {
        key: "avocado",
        singular: "avocado",
        plural: "avocados",
        recipe: AVOCADO_RECIPE,
    },
    LifestyleMeal {
        key: "toast",
        singular: "avocado toast",
        plural: "avocado toasts",
        recipe: TOAST_RECIPE,
    },
    LifestyleMeal {
        key: "guac",
        singular: "two-avocado guac bowl",
        plural: "two-avocado guac bowls",
        recipe: GUAC_RECIPE,
    },
];

const WHEAT_MEALS: &[LifestyleMeal] = &[
    LifestyleMeal {
        key: "pasta",
        singular: "bowl of pasta",
        plural: "bowls of pasta",
        recipe: PASTA_RECIPE,
    },
    LifestyleMeal {
        key: "cheeseburger",
        singular: "McDonald’s cheeseburger",
        plural: "McDonald’s cheeseburgers",
        recipe: BURGER_RECIPE,
    },
    LifestyleMeal {
        key: "baguette",
        singular: "medium baguette",
        plural: "medium baguettes",
        recipe: BAGUETTE_RECIPE,
    },
];

const COFFEE_MEALS: &[LifestyleMeal] = &[
    LifestyleMeal {
        key: "cup",
        singular: "cup of coffee",
        plural: "cups of coffee",
        recipe: CUP_RECIPE,
    },
    LifestyleMeal {
        key: "espresso",
        singular: "double espresso",
        plural: "double espressos",
        recipe: ESPRESSO_RECIPE,
    },
];

const SUGAR_MEALS: &[LifestyleMeal] = &[
    LifestyleMeal {
        key: "coke",
        singular: "can of Coke",
        plural: "cans of Coke",
        recipe: COKE_RECIPE,
    },
    LifestyleMeal {
        key: "donut",
        singular: "Dunkin’ glazed donut",
        plural: "Dunkin’ glazed donuts",
        recipe: DONUT_RECIPE,
    },
    LifestyleMeal {
        key: "tea",
        singular: "tea with 3 tsp sugar",
        plural: "teas with 3 tsp sugar",
        recipe: TEA_RECIPE,
    },
];

const DAVIS_CITATION: SourceCitation = SourceCitation {
    id: "crop-davis-citation",
    label: "Steven Davis’s estimate of 15 deaths",
    paragraphs: &[
        "Steven L. Davis was an Oregon State animal-science professor who worked on animal bioethics. His 2003 ethics paper asked whether a least-harm diet might include pasture-fed ruminants.",
        "The 15-death figure is Davis’s calculation, not a field count. He borrowed 25 mice per hectare from an English wood-mouse study. He then chose a 60% mortality rate between that study’s 52% and a Hawaiian sugarcane study’s 77%.",
        "The English study found little direct effect from the combine. Most recorded deaths followed the loss of crop cover, when mice became vulnerable to predators.",
        "Davis’s calculation combines species, crops, countries, and studies. This calculator shows it as a landmark in the debate, not as a measurement of U.S. wheat fields.",
    ],
    sources: &[
        SourceLink {
            label: "Davis paper ↗",
            url: "https://doi.org/10.1023/A:1025638030686",
        },
        SourceLink {
            label: "OSU profile ↗",
            url: "https://news.oregonstate.edu/news/unconventional-animal-scientist-retires-sort",
        },
        SourceLink {
            label: "wood-mouse study ↗",
            url: "https://doi.org/10.1016/0006-3207(93)90060-E",
        },
        SourceLink {
            label: "sugarcane study ↗",
            url: "https://doi.org/10.2307/3799611",
        },
    ],
};

const ARCHER_CITATION: SourceCitation = SourceCitation {
    id: "crop-archer-citation",
    label: "Michael Archer’s Australian mouse-plague model",
    paragraphs: &[
        "Michael Archer is an Australian zoologist. In a 2011 essay, he used mouse-plague control in eastern Australian grain fields to argue about the animal costs of different diets.",
        "His starting point was 500 mice per hectare during a plague. He assumed one plague every four years. He also assumed poison bait kills 80% of the mice present.",
        "The arithmetic is 500 ÷ 4 × 0.8 = 100 deaths per hectare per year. It is a high-end Australian model, not a measurement of ordinary U.S. wheat production.",
    ],
    sources: &[SourceLink {
        label: "Archer paper ↗",
        url: "https://doi.org/10.7882/AZ.2011.051",
    }],
};

const NASS_CITATION: SourceCitation = SourceCitation {
    id: "crop-nass-citation",
    label: "a 1971 Hawaiian rat-tracking study",
    paragraphs: &[
        "R. D. Nass, G. A. Hood, and G. D. Lindsey used radio telemetry to follow Polynesian rats through a mechanical sugarcane harvest in Hawaii.",
        "About 77% of the tracked rats died during that harvest. Machinery, compacted burrows, later injuries, and predation all contributed.",
        "This is direct evidence that the harvest mechanism can kill rats. It is not a rat-density estimate for Brazilian cane, and the calculator’s 80 deaths per hectare remains a modeling choice.",
    ],
    sources: &[SourceLink {
        label: "Nass, Hood & Lindsey paper ↗",
        url: "https://doi.org/10.2307/3799611",
    }],
};

#[component]
async fn range_sample(
    scenario_key: &str,
    sample_index: usize,
    sample: RangeSample,
    visible: bool,
    active: bool,
) -> Result {
    view! {
        <article
            class="crop-range-sample"
            data-crop-range-sample=""
            data-crop-scenario=(scenario_key)
            data-crop-sample-index=(sample_index)
            hidden=(!visible)
        >
            <button
                type="button"
                class="crop-range-sample-pick"
                data-crop-sample-pick=""
                data-crop-sample-rate=(sample.rate)
                data-current=(active.then_some("true"))
            >
                <span>(sample.label)</span>
                <strong data-crop-sample-rate-label="">(format_number(sample.rate))</strong>
                <small>"deaths / hectare / crop year"</small>
            </button>
            <p class="crop-range-sample-copy">
                (sample.trail_prefix)
                if let Some(citation) = sample.citation {
                    inline_popover(
                        id: citation.id,
                        label: citation.label,
                        <span class="inline-popover-copy">
                            for paragraph in citation.paragraphs {
                                <span class="inline-popover-paragraph">(paragraph)</span>
                            }
                        </span>
                        for source in citation.sources {
                            ext_link(
                                class: "quiet-link crop-inline-source-link",
                                href: source.url,
                                label: source.label
                            )
                        }
                    )
                }
                (sample.trail_suffix)
            </p>
        </article>
    }
}

#[component]
async fn recipe_factor(factor: RecipeFactor) -> Result {
    let operator = factor.operator.symbol();
    view! {
        <span
            class="crop-recipe-factor"
            data-approximate=(factor.approximate.then_some("true"))
        >
            if !operator.is_empty() {
                <span class="crop-recipe-operator" aria-hidden="true">(operator)</span>
            }
            <span class="crop-recipe-factor-term">
                inline_popover(
                    id: factor.citation.id,
                    label: factor.display,
                    <span class="inline-popover-preview crop-recipe-citation">
                        <span class="crop-recipe-source-kind">(factor.citation.kind.label())</span>
                        <span>(factor.citation.detail)</span>
                    </span>
                    for source in factor.citation.sources {
                        ext_link(
                            class: "quiet-link crop-recipe-source-link",
                            href: source.url,
                            label: source.label
                        )
                    }
                )
                <small>(factor.unit)</small>
            </span>
        </span>
    }
}

#[component]
async fn recipe_component(component: RecipeComponent, show_copy: bool) -> Result {
    let result = format_recipe_grams(component.grams(), component.decimals);
    let relation = if component.approximate { "≈" } else { "=" };
    view! {
        <section class="crop-recipe-component">
            <p class="crop-recipe-component-label">(component.label)</p>
            <div class="crop-recipe-chain">
                for factor in component.factors {
                    recipe_factor(factor: *factor)
                }
                <span class="crop-recipe-operator" aria-hidden="true">(relation)</span>
                <span class="crop-recipe-result">
                    <strong>(format!("{result} g"))</strong>
                    <small>(component.result_unit)</small>
                </span>
            </div>
            if show_copy {
                <p class="crop-recipe-component-copy">(component.copy)</p>
            }
        </section>
    }
}

#[component]
async fn meal_recipe_panel(scenario_key: &str, meal: LifestyleMeal, visible: bool) -> Result {
    let recipe = meal.recipe;
    let total = format_recipe_grams(recipe.grams(), recipe.decimals);
    let relation = if recipe.approximate { "≈" } else { "=" };
    view! {
        <article
            class="crop-recipe-panel"
            data-crop-recipe-panel=""
            data-crop-scenario=(scenario_key)
            data-crop-meal=(meal.key)
            data-crop-kg=(meal.crop_kg())
            data-display-grams=(total.as_str())
            data-total-unit=(recipe.total_unit)
            data-composite=(recipe.composite().then_some("true"))
            hidden=(!visible)
        >
            <div class="crop-recipe-head">
                <p>(meal.singular)</p>
                <span>"meal → crop recipe"</span>
            </div>
            <div class="crop-recipe-components">
                for component in recipe.components {
                    recipe_component(component: *component, show_copy: recipe.composite())
                }
            </div>
            if recipe.composite() {
                <div class="crop-recipe-total">
                    <span class="crop-recipe-total-parts">
                        for (index, component) in recipe.components.iter().enumerate() {
                            if index > 0 {
                                <span aria-hidden="true">" + "</span>
                            }
                            <span>(format!("{} g", format_recipe_grams(component.grams(), component.decimals)))</span>
                        }
                    </span>
                    <span class="crop-recipe-operator" aria-hidden="true">(relation)</span>
                    <span class="crop-recipe-result crop-recipe-grand-total">
                        <strong>(format!("{total} g"))</strong>
                        <small>(recipe.total_unit)</small>
                    </span>
                </div>
            }
            <p class="crop-recipe-copy">(recipe.copy)</p>
        </article>
    }
}

const AVOCADO_SAMPLES: &[RangeSample] = &[
    RangeSample {
        rate: 1.0,
        label: "one squirrel triggers control",
        trail_prefix: "A ground squirrel in or beside a hectare is enough to trigger control guidance; this treats that decision as one attributable ground-squirrel death per crop year.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 5.0,
        label: "a few squirrels are baited",
        trail_prefix: "A few ground squirrels occupy the hectare and the pressured colony turns over during the crop year.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 25.0,
        label: "dense squirrel colony is baited",
        trail_prefix: "A dense ground-squirrel colony is repeatedly baited, and most of that colony-scale population is counted as deaths over one crop year.",
        citation: None,
        trail_suffix: "",
    },
];

const WHEAT_SAMPLES: &[RangeSample] = &[
    RangeSample {
        rate: 2.0,
        label: "few mice hit by harvest",
        trail_prefix: "A sparse, non-plague reading where harvesting machinery directly hits only a couple of wood mice per hectare per crop year.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 15.0,
        label: "harvest removes cover",
        trail_prefix: "This middle scenario uses ",
        citation: Some(DAVIS_CITATION),
        trail_suffix: ". It starts with 25 mice per hectare. It applies a 60% mortality rate. That yields 15 deaths per hectare per crop year.",
    },
    RangeSample {
        rate: 100.0,
        label: "mouse plague + poison bait",
        trail_prefix: "This high-end scenario uses ",
        citation: Some(ARCHER_CITATION),
        trail_suffix: ". It starts with 500 mice per hectare during a plague. It spreads one plague across four years, then assumes an 80% bait kill. That yields 100 deaths per hectare per crop year.",
    },
];

const COFFEE_SAMPLES: &[RangeSample] = &[
    RangeSample {
        rate: 1_000.0,
        label: "low insecticide kill",
        trail_prefix: "Low insecticide exposure or well-timed spraying: about 1,000 larger insects are assumed killed per hectare per crop year.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 100_000.0,
        label: "routine insecticide kill",
        trail_prefix: "Routine insecticide use: about 100,000 insect deaths are assumed per hectare per crop year if a meaningful fraction of local arthropods are hit.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 1_000_000.0,
        label: "widespread insecticide kill",
        trail_prefix: "Heavy or widespread insecticide exposure: about 1 million insect deaths are assumed per hectare per crop year when spray mortality is broad—still not every insect on the farm.",
        citation: None,
        trail_suffix: "",
    },
];

const SUGAR_SAMPLES: &[RangeSample] = &[
    RangeSample {
        rate: 20.0,
        label: "moderate harvest kill",
        trail_prefix: "A moderate cane-rat population meets a harvest that kills a real fraction over one crop year, without counting every survivor’s later predation as an extra farm death.",
        citation: None,
        trail_suffix: "",
    },
    RangeSample {
        rate: 80.0,
        label: "harvest machinery kills rats",
        trail_prefix: "This middle scenario borrows its harvest mechanism from ",
        citation: Some(NASS_CITATION),
        trail_suffix: ". The study supplies a mortality fraction, not a Brazilian population density. The calculator models a denser cane-rat hectare and rounds the result to 80 deaths per hectare per crop year.",
    },
    RangeSample {
        rate: 250.0,
        label: "dense rats + harvest kill",
        trail_prefix: "A dense cane-rat population plus harvest kill in a high-pressure crop year, read generously.",
        citation: None,
        trail_suffix: "",
    },
];

const AVOCADO: Scenario = Scenario {
    key: "avocado",
    tab: "avocados / squirrels",
    crop: "California avocados",
    crop_unit: "avocado",
    animal_singular: "ground squirrel",
    animal_plural: "ground squirrels",
    region: "California · 2024 crop",
    // USDA NASS: 3.82 US short tons / acre.
    yield_kg_per_hectare: 8_563.302_833,
    meals: AVOCADO_MEALS,
    yield_source: "USDA NASS · California 2024",
    yield_url: "https://www.nass.usda.gov/Quick_Stats/Ag_Overview/stateOverview.php?state=California&year=2024",
    range_min: 1.0,
    range_max: 25.0,
    samples: AVOCADO_SAMPLES,
};

const WHEAT: Scenario = Scenario {
    key: "wheat",
    tab: "wheat / mice",
    crop: "U.S. wheat",
    crop_unit: "wheat",
    animal_singular: "wood mouse",
    animal_plural: "wood mice",
    region: "United States · 2025 crop",
    // USDA NASS: 53.3 bushels / acre; a wheat bushel is 60 lb.
    yield_kg_per_hectare: 3_584.481_998,
    meals: WHEAT_MEALS,
    yield_source: "USDA NASS · 2025 wheat summary",
    yield_url: "https://www.nass.usda.gov/Newsroom/archive/2025/09-30-2025.php",
    range_min: 2.0,
    range_max: 100.0,
    samples: WHEAT_SAMPLES,
};

const COFFEE: Scenario = Scenario {
    key: "coffee",
    tab: "coffee / insects",
    crop: "Brazilian coffee",
    crop_unit: "green coffee",
    animal_singular: "insect",
    animal_plural: "insects",
    region: "Brazil · 2025 crop",
    // CONAB: 29.7 60 kg bags of green coffee / hectare.
    yield_kg_per_hectare: 1_782.0,
    meals: COFFEE_MEALS,
    yield_source: "CONAB · Brazil 2025",
    yield_url: "https://cast.conab.gov.br/post/2025-09-04_lev_3_cafe/",
    range_min: 1_000.0,
    range_max: 1_000_000.0,
    samples: COFFEE_SAMPLES,
};

const SUGAR: Scenario = Scenario {
    key: "sugar",
    tab: "sugar / rats",
    crop: "Brazilian cane sugar",
    crop_unit: "sugar",
    animal_singular: "rat",
    animal_plural: "rats",
    region: "Brazil · 2024/25 cane · Hawaiian harvest evidence",
    // USDA FAS Brazil: ~79 t cane/ha × ~143 kg recoverable sugar / t cane.
    yield_kg_per_hectare: 79_000.0 * 0.143,
    meals: SUGAR_MEALS,
    yield_source: "USDA FAS · Brazil sugar semi-annual",
    yield_url: "https://apps.fas.usda.gov/newgainapi/api/Report/DownloadReportByFileName?fileName=Sugar+Semi-annual_Brasilia_Brazil_BR2024-0026.pdf",
    range_min: 20.0,
    range_max: 250.0,
    samples: SUGAR_SAMPLES,
};

const SCENARIOS: [Scenario; 4] = [AVOCADO, WHEAT, COFFEE, SUGAR];

#[derive(Clone, Debug)]
pub struct State {
    scenario: Scenario,
    meal: LifestyleMeal,
    deaths_per_hectare: f64,
    sample: Option<RangeSample>,
    warning: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Calculation {
    hectares: f64,
    food_kg: f64,
}

impl Calculation {
    fn meals(self, meal: LifestyleMeal) -> f64 {
        self.food_kg / meal.crop_kg()
    }
}

impl Scenario {
    fn from_key(key: Option<&str>) -> Option<Self> {
        SCENARIOS
            .iter()
            .copied()
            .find(|scenario| Some(scenario.key) == key)
    }
}

fn parse_positive(
    raw: Option<&str>,
    default: f64,
    maximum: f64,
    label: &str,
    warnings: &mut Vec<String>,
) -> f64 {
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().parse::<f64>() {
        Ok(value) if value.is_finite() && (MIN_VALUE..=maximum).contains(&value) => value,
        _ => {
            warnings.push(format!(
                "{label} must be between {} and {}; the calculator reset it to {}.",
                format_number(MIN_VALUE),
                format_number(maximum),
                format_number(default),
            ));
            default
        }
    }
}

pub fn state(food: Option<&str>, rate: Option<&str>, meal: Option<&str>) -> State {
    let mut warnings = Vec::new();
    let scenario = match food {
        None => AVOCADO,
        Some(key) => Scenario::from_key(Some(key)).unwrap_or_else(|| {
            warnings.push("That food is not in this draft; showing avocados instead.".to_string());
            AVOCADO
        }),
    };

    let selected_meal = scenario.meal(meal);
    if meal.is_some() && meal != Some(selected_meal.key) {
        warnings.push(format!(
            "That meal isn’t mapped for this claim; showing {} instead.",
            selected_meal.plural
        ));
    }

    let default_rate = scenario.default_rate();
    let deaths_per_hectare = match rate {
        None => default_rate,
        Some(raw) => parse_positive(
            Some(raw),
            default_rate,
            MAX_RATE,
            "Deaths per hectare per crop year",
            &mut warnings,
        ),
    };

    let sample = scenario.matching_sample(deaths_per_hectare);

    State {
        scenario,
        meal: selected_meal,
        deaths_per_hectare,
        sample,
        warning: warnings.join(" "),
    }
}

fn grouped_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

fn trim_decimal(mut value: String) -> String {
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

fn format_number(value: f64) -> String {
    if value >= 100.0 {
        grouped_integer(value.round() as u64)
    } else if value >= 10.0 {
        let rounded = (value * 10.0).round() / 10.0;
        trim_decimal(format!("{rounded:.1}"))
    } else if value >= 1.0 {
        let rounded = (value * 100.0).round() / 100.0;
        trim_decimal(format!("{rounded:.2}"))
    } else {
        let rounded = (value * 1_000.0).round() / 1_000.0;
        trim_decimal(format!("{rounded:.3}"))
    }
}

fn format_recipe_grams(value: f64, decimals: usize) -> String {
    if decimals == 0 {
        return grouped_integer(value.round() as u64);
    }
    let scale = 10_f64.powi(decimals as i32);
    trim_decimal(format!("{:.decimals$}", (value * scale).round() / scale))
}

fn plural<'a>(amount: f64, singular: &'a str, plural: &'a str) -> &'a str {
    if (amount - 1.0).abs() < f64::EPSILON {
        singular
    } else {
        plural
    }
}

fn range_position(rate: f64, minimum: f64, maximum: f64) -> f64 {
    if maximum <= minimum {
        return 50.0;
    }
    let rate = rate.clamp(minimum, maximum);
    ((rate.ln() - minimum.ln()) / (maximum.ln() - minimum.ln()) * 100.0).clamp(0.0, 100.0)
}

fn nearest_sample_index(rate: f64, samples: &[RangeSample]) -> usize {
    samples
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left_distance = (left.rate.ln() - rate.ln()).abs();
            let right_distance = (right.rate.ln() - rate.ln()).abs();
            left_distance.partial_cmp(&right_distance).unwrap()
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

#[component]
pub async fn calculator(state: State) -> Result {
    let scenario = state.scenario;
    let meal = state.meal;
    let calculation = scenario.calculate(state.deaths_per_hectare);
    let meal_count = calculation.meals(meal);
    let yield_kg = format_number(scenario.yield_kg_per_hectare);
    let meals_per_hectare = format_number(scenario.yield_kg_per_hectare / meal.crop_kg());
    let meal_grams = format_recipe_grams(meal.recipe.grams(), meal.recipe.decimals);
    let rate = format_number(state.deaths_per_hectare);
    let result_meals = format_number(meal_count);
    let meal_label = plural(meal_count, meal.singular, meal.plural);
    let yield_unit_label = format!("kg {} / hectare / crop year", scenario.crop_unit);
    let meal_grams_label = meal.recipe.total_unit.to_owned();
    let conversion_label = format!("{} / hectare / crop year", meal.plural);
    let rate_animal_label = plural(
        state.deaths_per_hectare,
        scenario.animal_singular,
        scenario.animal_plural,
    );
    let rate_label = format!("{} / hectare / crop year", rate_animal_label);
    let result_per_animal_label = format!("{} / {}", meal_label, scenario.animal_singular);
    let rate_value = state.deaths_per_hectare.to_string();
    let nearest_sample =
        scenario.samples[nearest_sample_index(state.deaths_per_hectare, scenario.samples)];
    let rate_status = match state.sample {
        Some(sample) => sample.label.to_owned(),
        None if state.deaths_per_hectare < scenario.range_min => {
            format!("below range · closest: {}", nearest_sample.label)
        }
        None if state.deaths_per_hectare > scenario.range_max => {
            format!("above range · closest: {}", nearest_sample.label)
        }
        None => format!("closest · {}", nearest_sample.label),
    };
    let range_min = format_number(scenario.range_min);
    let range_max = format_number(scenario.range_max);
    let range_slider_value = format!(
        "{:.0}",
        range_position(
            state.deaths_per_hectare,
            scenario.range_min,
            scenario.range_max
        ) * 10.0
    );
    let scenario_meal_attrs: Vec<String> = SCENARIOS.iter().map(|s| s.meals_json()).collect();
    let alt_meals: Vec<_> = scenario
        .meals
        .iter()
        .copied()
        .filter(|candidate| candidate.key != meal.key)
        .map(|candidate| {
            let count = calculation.meals(candidate);
            (
                candidate,
                format_number(count),
                plural(count, candidate.singular, candidate.plural),
            )
        })
        .collect();

    view! {
        <section
            class="crop-calculator"
            id="calculator"
            aria-labelledby="calculator-title"
            data-crop-calculator=""
        >
            <div class="crop-calc-heading">
                <p class="crop-section-number">"03 / the calculator"</p>
            </div>

            <form
                method="get"
                action="/thoughts/crop-deaths#calculator"
                class="crop-controls"
                data-crop-controls=""
            >
                <fieldset>
                    <legend>"1 · crop"</legend>
                    <div class="crop-scenario-tabs">
                        for (candidate, meals_attr) in SCENARIOS.iter().zip(scenario_meal_attrs.iter()) {
                            <label>
                                <input
                                    type="radio"
                                    name="food"
                                    value=(candidate.key)
                                    checked=(candidate.key == scenario.key)
                                    data-crop=(candidate.crop)
                                    data-animal-singular=(candidate.animal_singular)
                                    data-animal-plural=(candidate.animal_plural)
                                    data-region=(candidate.region)
                                    data-crop-unit=(candidate.crop_unit)
                                    data-yield-kg=(candidate.yield_kg_per_hectare)
                                    data-yield-source=(candidate.yield_source)
                                    data-yield-url=(candidate.yield_url)
                                    data-range-min=(candidate.range_min)
                                    data-range-max=(candidate.range_max)
                                    data-default-rate=(candidate.default_rate())
                                    data-meals=(meals_attr.as_str())
                                >
                                <span>(candidate.tab)</span>
                            </label>
                        }
                    </div>
                </fieldset>

                <fieldset class="crop-rate-fieldset">
                    <legend>"2 · choose a death rate"</legend>
                    <input
                        type="range"
                        min="0"
                        max="1000"
                        step="1"
                        value=(range_slider_value.as_str())
                        aria-label="Death rate within the illustrated range"
                        aria-valuetext=(format!("{} deaths per hectare per crop year", rate))
                        data-crop-rate-slider=""
                    >
                    <div class="crop-rate-scale" aria-hidden="true">
                        <span data-crop-rate-min="">(range_min.as_str())</span>
                        <span>"deaths / hectare / yr"</span>
                        <span data-crop-rate-max="">(range_max.as_str())</span>
                    </div>

                    <label class="crop-rate-entry">
                        <span>"your rate · deaths / hectare / crop year"</span>
                        <input
                            type="number"
                            name="rate"
                            value=(rate_value.as_str())
                            min="0.001"
                            max="1000000"
                            step="any"
                            inputmode="decimal"
                            required=""
                            data-crop-rate=""
                        >
                    </label>

                    <div class="crop-range-sample-groups" data-crop-range-sample-groups="">
                        for candidate in SCENARIOS {
                            <div
                                class="crop-range-sample-grid"
                                data-crop-range-samples=""
                                data-crop-scenario=(candidate.key)
                                hidden=(candidate.key != scenario.key)
                            >
                                for (sample_index, sample) in candidate.samples.iter().enumerate() {
                                    range_sample(
                                        scenario_key: candidate.key,
                                        sample_index: sample_index,
                                        sample: *sample,
                                        visible: sample_index
                                            == nearest_sample_index(
                                                if candidate.key == scenario.key {
                                                    state.deaths_per_hectare
                                                } else {
                                                    candidate.default_rate()
                                                },
                                                candidate.samples,
                                            ),
                                        active: candidate.key == scenario.key
                                            && sample_index
                                                == nearest_sample_index(
                                                    state.deaths_per_hectare,
                                                    candidate.samples,
                                                ),
                                    )
                                }
                            </div>
                        }
                    </div>
                </fieldset>

                <fieldset class="crop-meal-fieldset">
                    <legend>"3 · express as"</legend>
                    <div class="crop-meal-tabs" role="radiogroup" aria-label="Lifestyle meal" data-crop-meal-tabs="">
                        for candidate in scenario.meals.iter() {
                            <label>
                                <input
                                    type="radio"
                                    name="meal"
                                    value=(candidate.key)
                                    checked=(candidate.key == meal.key)
                                    data-crop-meal=(candidate.key)
                                >
                                <span>(candidate.singular)</span>
                            </label>
                        }
                    </div>
                </fieldset>
                <button type="submit">"run the assumption →"</button>
                if state.warning.is_empty() {
                    <p class="crop-calc-warning" role="alert" data-crop-warning="" hidden=""></p>
                } else {
                    <p class="crop-calc-warning" role="alert" data-crop-warning="">(state.warning.as_str())</p>
                }
            </form>

            <div class="crop-receipt" aria-live="polite">
                <div class="crop-receipt-head">
                    <div>
                        <h3 data-crop-name="">(scenario.crop)</h3>
                    </div>
                    <span data-crop-rate-status="">(rate_status.as_str())</span>
                </div>
                <p class="crop-receipt-region" data-crop-region="">(scenario.region)</p>

                <div class="crop-result">
                    <p data-crop-result-lede="">(format!("under your assumption, 1 {} corresponds to", scenario.animal_singular))</p>
                    <strong data-crop-result-units="">(result_meals.as_str())</strong>
                    <span data-crop-result-unit-label="">(meal_label)</span>
                    <small data-crop-result-detail="">
                        (format!("at {rate} {} / hectare / crop year · 1 {} represented", plural(state.deaths_per_hectare, scenario.animal_singular, scenario.animal_plural), scenario.animal_singular))
                    </small>
                </div>

                <div
                    class="crop-equation"
                    aria-label=(format!(
                        "The selected meal recipe evaluates to {} grams of {}. {} kg {} per hectare per crop year times 1,000 grams per kilogram divided by {} grams per {}, equals {} {} per hectare per crop year; {} {} per hectare per crop year divided by {} {}, equals {} {} per {}",
                        meal_grams,
                        meal.recipe.total_unit,
                        yield_kg,
                        scenario.crop_unit,
                        meal_grams,
                        meal.singular,
                        meals_per_hectare,
                        meal.plural,
                        meals_per_hectare,
                        meal.plural,
                        rate,
                        rate_animal_label,
                        result_meals,
                        meal_label,
                        scenario.animal_singular,
                    ))
                    data-crop-equation=""
                >
                    <div class="crop-recipe-panels" data-crop-recipe-panels="">
                        for candidate_scenario in SCENARIOS {
                            for candidate_meal in candidate_scenario.meals {
                                meal_recipe_panel(
                                    scenario_key: candidate_scenario.key,
                                    meal: *candidate_meal,
                                    visible: candidate_scenario.key == scenario.key
                                        && candidate_meal.key == meal.key,
                                )
                            }
                        }
                    </div>
                    <div class="crop-equation-divider" aria-hidden="true"></div>
                    <p class="crop-equation-heading crop-equation-heading-secondary">"then apply it to one hectare"</p>
                    <div class="crop-equation-line crop-conversion-line">
                        <span class="crop-equation-term">
                            <strong data-crop-yield-kg="">(yield_kg.as_str())</strong>
                            <small data-crop-yield-unit="">(yield_unit_label.as_str())</small>
                        </span>
                        <span class="crop-equation-operator" aria-hidden="true">"×"</span>
                        <span class="crop-equation-term">
                            <strong>"1,000 g"</strong>
                            <small>"per kg"</small>
                        </span>
                        <span class="crop-equation-operator" aria-hidden="true">"÷"</span>
                        <span class="crop-equation-term">
                            <strong data-crop-meal-grams="">(format!("{meal_grams} g"))</strong>
                            <small data-crop-meal-grams-label="">(meal_grams_label.as_str())</small>
                        </span>
                    </div>
                    <div class="crop-equation-answer">
                        <span class="crop-equation-operator" aria-hidden="true">"="</span>
                        <span class="crop-equation-term crop-equation-emphasis">
                            <strong data-crop-conversion-meals="">(meals_per_hectare.as_str())</strong>
                            <small data-crop-conversion-label="">(conversion_label.as_str())</small>
                        </span>
                    </div>
                    <p class="crop-equation-source">
                        <span>"yield input: "</span>
                        <span data-crop-yield-source="">(scenario.yield_source)</span>
                        <a data-crop-yield-link="" href=(scenario.yield_url) target="_blank" rel="noopener noreferrer">"source ↗"</a>
                    </p>
                    <div class="crop-equation-divider" aria-hidden="true"></div>
                    <p class="crop-equation-heading crop-equation-heading-secondary">"then divide by the death rate"</p>
                    <div class="crop-equation-line crop-division-line">
                        <span class="crop-equation-term">
                            <strong data-crop-division-meals="">(meals_per_hectare.as_str())</strong>
                            <small data-crop-division-meals-label="">(conversion_label.as_str())</small>
                        </span>
                        <span class="crop-equation-operator" aria-hidden="true">"÷"</span>
                        <span class="crop-equation-term">
                            <strong data-crop-death-rate="">(rate.as_str())</strong>
                            <small data-crop-death-rate-label="">(rate_label.as_str())</small>
                        </span>
                    </div>
                    <div class="crop-equation-answer">
                        <span class="crop-equation-operator" aria-hidden="true">"="</span>
                        <span class="crop-equation-term crop-equation-final">
                            <strong data-crop-division-result="">(result_meals.as_str())</strong>
                            <small data-crop-division-result-label="">(result_per_animal_label.as_str())</small>
                        </span>
                    </div>
                </div>

                <div class="crop-alt-meals" data-crop-alt-meals="">
                    <p>"same crop death, other meals"</p>
                    <ul>
                        for (candidate, count, label) in alt_meals.iter() {
                            <li data-alt-meal=(candidate.key)>
                                <strong data-alt-count="">(count.as_str())</strong>
                                <span data-alt-label="">(label)</span>
                            </li>
                        }
                    </ul>
                </div>

            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use topcoat::context::Cx;

    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn recipe_panel_opening<'a>(html: &'a str, scenario: &str, meal: &str) -> &'a str {
        let marker = format!("data-crop-scenario=\"{scenario}\" data-crop-meal=\"{meal}\"");
        let marker_start = html.find(&marker).expect("recipe panel marker");
        let tag_start = html[..marker_start].rfind("<article").expect("panel start");
        let tag_end = html[marker_start..].find('>').expect("panel end") + marker_start + 1;
        &html[tag_start..tag_end]
    }

    #[test]
    fn avocado_middle_sample_is_the_default_rate() {
        let parsed = state(None, None, None);
        assert_eq!(parsed.scenario, AVOCADO);
        assert_eq!(parsed.deaths_per_hectare, AVOCADO.default_rate());
        assert_eq!(parsed.sample, Some(AVOCADO.samples[1]));
        assert_eq!(parsed.meal.key, "avocado");
    }

    #[test]
    fn wheat_defaults_to_pasta_and_can_switch_to_cheeseburger() {
        let pasta = state(Some("wheat"), None, None);
        let burger = state(Some("wheat"), None, Some("cheeseburger"));
        assert_eq!(pasta.meal.key, "pasta");
        assert_eq!(burger.meal.key, "cheeseburger");
        let calc = WHEAT.calculate(WHEAT.default_rate());
        // Feed grain for the patty dwarfs a pasta bowl, so one death corresponds to fewer burgers.
        assert!(calc.meals(burger.meal) < calc.meals(pasta.meal));
        assert!((burger.meal.crop_kg() / pasta.meal.crop_kg()) > 5.0);
    }

    #[test]
    fn every_meal_mass_is_evaluated_from_its_recipe() {
        let expected_grams = [
            ("avocado", "avocado", 200.0),
            ("avocado", "toast", 100.0),
            ("avocado", "guac", 400.0),
            ("wheat", "pasta", 56.0 / 0.74),
            (
                "wheat",
                "cheeseburger",
                45.0 * 9.4 / 0.865 + 50.0 / 0.75 * (100.0 / 187.5) / 0.74,
            ),
            ("wheat", "baguette", 250.0 / 0.75 * (100.0 / 173.4) / 0.74),
            ("coffee", "cup", 0.240 * 55.0 / 0.8374),
            ("coffee", "espresso", 18.0 / 0.8374),
            ("sugar", "coke", 39.0),
            ("sugar", "donut", 12.0),
            ("sugar", "tea", 3.0 * 4.2),
        ];

        for (scenario_key, meal_key, expected) in expected_grams {
            let scenario = Scenario::from_key(Some(scenario_key)).unwrap();
            let meal = scenario.meal(Some(meal_key));
            assert_close(meal.recipe.grams(), expected);
            assert_close(meal.crop_kg() * 1_000.0, expected);
        }
    }

    #[test]
    fn burger_subtotals_and_moisture_conversion_stay_explicit() {
        use super::recipes::FactorOperator;

        let burger = WHEAT.meal(Some("cheeseburger"));
        let beef = burger.recipe.components[0];
        let bun = burger.recipe.components[1];

        assert_close(beef.grams(), 45.0 * 9.4 / 0.865);
        assert_close(bun.grams(), 50.0 / 0.75 * (100.0 / 187.5) / 0.74);
        assert_close(burger.recipe.grams(), 537.065_389_088_510_5);
        assert_eq!(beef.factors[2].operator, FactorOperator::Divide);
        assert_close(beef.factors[2].value, 1.0 - 0.135);
        assert_eq!(format_recipe_grams(beef.grams(), beef.decimals), "489");
        assert_eq!(format_recipe_grams(bun.grams(), bun.decimals), "48");
        assert_eq!(
            format_recipe_grams(burger.recipe.grams(), burger.recipe.decimals),
            "537"
        );
        assert!(
            beef.factors[1]
                .citation
                .detail
                .contains("outside this factor; they are not assumed harmless")
        );
        assert!(beef.copy.contains("omitted—not counted as impact-free"));
        assert!(
            burger
                .recipe
                .copy
                .contains("not every feed input or land-management impact")
        );
    }

    #[test]
    fn baguette_size_and_formula_choice_stay_explicit() {
        let baguette = WHEAT.meal(Some("baguette"));
        let component = baguette.recipe.components[0];

        assert_close(component.factors[0].value, 250.0);
        assert_close(component.factors[1].value, 0.75);
        assert_close(component.factors[2].value, 100.0 / 173.4);
        assert_close(component.factors[3].value, 0.74);
        assert_close(component.grams(), 259.775_346_280_536_97);
        assert_eq!(
            format_recipe_grams(component.grams(), component.decimals),
            "260"
        );
    }

    #[test]
    fn serialized_crop_kg_matches_each_recipe_result() {
        for scenario in SCENARIOS {
            let serialized: serde_json::Value =
                serde_json::from_str(&scenario.meals_json()).expect("meal JSON should be valid");
            let entries = serialized.as_array().expect("meal JSON array");
            assert_eq!(entries.len(), scenario.meals.len());

            for (entry, meal) in entries.iter().zip(scenario.meals) {
                assert_eq!(entry["key"], meal.key);
                assert_close(
                    entry["cropKg"].as_f64().expect("numeric cropKg"),
                    meal.recipe.grams() / 1_000.0,
                );
            }
        }
    }

    #[test]
    fn every_recipe_factor_has_sources_and_a_unique_popover_id() {
        let mut ids = HashSet::new();
        for scenario in SCENARIOS {
            for sample in scenario.samples {
                if let Some(citation) = sample.citation {
                    assert!(
                        !citation.paragraphs.is_empty(),
                        "{} has no explanatory copy",
                        citation.label
                    );
                    assert!(
                        !citation.sources.is_empty(),
                        "{} has no inline source",
                        citation.label
                    );
                    assert!(ids.insert(citation.id), "duplicate id: {}", citation.id);
                }
            }
            for meal in scenario.meals {
                for component in meal.recipe.components {
                    assert!(!component.factors.is_empty(), "{} has no factors", meal.key);
                    for factor in component.factors {
                        assert!(
                            !factor.citation.sources.is_empty(),
                            "{} factor {} has no source",
                            meal.key,
                            factor.display
                        );
                        assert!(
                            ids.insert(factor.citation.id),
                            "duplicate id: {}",
                            factor.citation.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn davis_landmark_identifies_the_author_and_exposes_its_construction() {
        let citation = WHEAT.samples[1].citation.expect("middle wheat citation");

        assert_eq!(citation.label, "Steven Davis’s estimate of 15 deaths");
        assert!(
            citation
                .paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("Oregon State animal-science professor"))
        );
        assert!(
            citation
                .paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("calculation, not a field count"))
        );
        assert!(
            citation
                .paragraphs
                .iter()
                .any(|paragraph| paragraph.contains("not as a measurement of U.S. wheat fields"))
        );
        assert!(
            citation
                .sources
                .iter()
                .any(|source| source.url.contains("1025638030686"))
        );
        assert!(!WHEAT.samples[1].trail_suffix.contains("Davis"));
        assert!(!SUGAR.samples[1].trail_suffix.contains("Davis"));
    }

    #[tokio::test]
    async fn ssr_contains_every_recipe_and_only_the_selected_panel_is_visible() {
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! {
            calculator(state: state(Some("wheat"), Some("15"), Some("cheeseburger")))
        };
        let html = result.unwrap().render(__cx);

        assert_eq!(
            html.matches("data-crop-recipe-panel=\"\"").count(),
            SCENARIOS
                .iter()
                .map(|scenario| scenario.meals.len())
                .sum::<usize>()
        );
        assert!(!recipe_panel_opening(&html, "wheat", "cheeseburger").contains(" hidden"));
        assert!(recipe_panel_opening(&html, "wheat", "pasta").contains(" hidden=\"\""));
        assert!(recipe_panel_opening(&html, "coffee", "cup").contains(" hidden=\"\""));
        assert!(html.contains(">489 g</strong><small>wheat-yield proxy</small>"));
        assert!(html.contains(">48 g</strong><small>farm-gate wheat</small>"));
        assert!(html.contains(">537 g</strong><small>wheat-yield proxy / cheeseburger</small>"));
        assert!(html.contains("OECD feedlot benchmark expressed as a wheat-yield proxy"));
        assert!(html.contains("Steven Davis’s estimate of 15 deaths"));
        assert!(html.contains("Oregon State animal-science professor"));
        assert!(html.contains("https://doi.org/10.1023/A:1025638030686"));
        assert!(!html.contains("crop-range-sample-output"));
        assert!(!html.contains("data-crop-sample-units"));
        assert!(html.contains("data-crop-division-result"));
        assert!(html.contains("data-crop-kg=\"0.5370653890885105\""));
        assert!(html.contains(
            "The selected meal recipe evaluates to 537 grams of wheat-yield proxy / cheeseburger."
        ));
        assert!(!html.contains("crop year per hectare / crop year"));
    }

    #[test]
    fn enhanced_switching_hides_stale_recipe_panels_without_building_markup() {
        const SCRIPT: &str = include_str!("crop-deaths.js");

        assert!(SCRIPT.contains("querySelectorAll(\"[data-crop-recipe-panel]\")"));
        assert!(SCRIPT.contains("panel.hidden = !current"));
        assert!(SCRIPT.contains("panel.setAttribute(\"aria-hidden\", \"true\")"));
        assert!(SCRIPT.contains("showRecipePanel(scenario.value, meal.key)"));
        assert!(SCRIPT.contains("recipePanel?.dataset.cropKg"));
        assert!(SCRIPT.contains("grams of ${recipeUnit}"));
        assert!(SCRIPT.contains("updateRange(scenario, rate)"));
        assert!(!SCRIPT.contains("data-crop-sample-units"));
        assert!(!SCRIPT.contains("data-crop-sample-unit-label"));
        assert!(!SCRIPT.contains("data-crop-conversion-copy"));
    }

    #[test]
    fn sample_rate_can_be_selected_directly() {
        let parsed = state(Some("wheat"), Some("100"), None);
        assert_eq!(parsed.scenario, WHEAT);
        assert_eq!(parsed.deaths_per_hectare, WHEAT.samples[2].rate);
        assert_eq!(parsed.sample, Some(WHEAT.samples[2]));
    }

    #[test]
    fn claim_change_uses_each_scenario_middle_sample() {
        let avocado = state(Some("avocado"), None, None);
        let wheat = state(Some("wheat"), None, None);
        assert_eq!(avocado.deaths_per_hectare, AVOCADO.default_rate());
        assert_eq!(wheat.deaths_per_hectare, WHEAT.default_rate());
        assert_eq!(wheat.sample, Some(WHEAT.samples[1]));
    }

    #[test]
    fn custom_rate_leaves_the_sample() {
        let parsed = state(Some("wheat"), Some("7"), None);
        assert_eq!(parsed.deaths_per_hectare, 7.0);
        assert_eq!(parsed.sample, None);
    }

    #[test]
    fn sugar_scenario_is_registered() {
        let parsed = state(Some("sugar"), None, Some("coke"));
        assert_eq!(parsed.scenario, SUGAR);
        assert_eq!(parsed.meal.key, "coke");
    }

    #[test]
    fn avocado_example_keeps_the_assumption_out_of_the_yield() {
        let result = AVOCADO.calculate(1.0);
        assert!((result.hectares - 1.0).abs() < 1e-9);
        assert!((result.food_kg - 8_563.302_833).abs() < 1e-6);
        assert!((result.meals(AVOCADO_MEALS[0]) - 42_816.514_165).abs() < 1e-6);
    }

    #[test]
    fn result_scales_inverse_to_the_assumed_death_rate() {
        let pasta = WHEAT_MEALS[0];
        let one = WHEAT.calculate(1.0).meals(pasta);
        let ten = WHEAT.calculate(10.0).meals(pasta);
        assert!((one / ten - 10.0).abs() < 1e-9);
    }

    #[test]
    fn nearest_landmark_follows_the_logarithmic_slider() {
        assert_eq!(
            WHEAT.samples[nearest_sample_index(7.0, WHEAT.samples)].rate,
            15.0
        );
        assert_eq!(
            COFFEE.samples[nearest_sample_index(700_000.0, COFFEE.samples)].rate,
            1_000_000.0
        );
    }

    #[test]
    fn invalid_query_values_fall_back_with_a_visible_warning() {
        let parsed = state(Some("dragonfruit"), Some("zero"), Some("milkshake"));
        assert_eq!(parsed.scenario, AVOCADO);
        assert_eq!(parsed.deaths_per_hectare, AVOCADO.default_rate());
        assert!(parsed.warning.contains("not in this draft"));
        assert!(parsed.warning.contains("Deaths per hectare per crop year"));
        assert!(parsed.warning.contains("meal isn’t mapped"));
    }

    #[test]
    fn number_formatting_is_compact_and_grouped() {
        assert_eq!(format_number(42_816.51), "42,817");
        assert_eq!(format_number(10.25), "10.3");
        assert_eq!(format_number(1.25), "1.25");
        assert_eq!(format_number(0.125), "0.125");
    }
}
