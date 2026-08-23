//! Meal-to-crop conversion recipes.
//!
//! Every calculator denominator is evaluated from these factors. The same
//! values drive the serialized `cropKg` consumed by JavaScript and the
//! server-rendered dimensional analysis shown beside the result.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FactorOperator {
    Start,
    Multiply,
    Divide,
}

impl FactorOperator {
    pub(super) const fn symbol(self) -> &'static str {
        match self {
            Self::Start => "",
            Self::Multiply => "×",
            Self::Divide => "÷",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceKind {
    MeasuredProductData,
    IndustryBenchmark,
    ModelingChoice,
}

impl SourceKind {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::MeasuredProductData => "measured product data",
            Self::IndustryBenchmark => "industry benchmark",
            Self::ModelingChoice => "explicit modeling choice",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RecipeSource {
    pub(super) label: &'static str,
    pub(super) url: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RecipeCitation {
    pub(super) id: &'static str,
    pub(super) kind: SourceKind,
    pub(super) detail: &'static str,
    pub(super) sources: &'static [RecipeSource],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RecipeFactor {
    pub(super) operator: FactorOperator,
    /// Scalar used when the recipe is evaluated. Units cancel as displayed.
    pub(super) value: f64,
    pub(super) display: &'static str,
    pub(super) unit: &'static str,
    pub(super) approximate: bool,
    pub(super) citation: RecipeCitation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RecipeComponent {
    pub(super) label: &'static str,
    pub(super) factors: &'static [RecipeFactor],
    pub(super) result_unit: &'static str,
    pub(super) approximate: bool,
    pub(super) decimals: usize,
    pub(super) copy: &'static str,
}

impl RecipeComponent {
    pub(super) fn grams(self) -> f64 {
        self.factors
            .iter()
            .fold(1.0, |result, factor| match factor.operator {
                FactorOperator::Start | FactorOperator::Multiply => result * factor.value,
                FactorOperator::Divide => result / factor.value,
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ConversionRecipe {
    pub(super) components: &'static [RecipeComponent],
    pub(super) total_unit: &'static str,
    pub(super) approximate: bool,
    pub(super) decimals: usize,
    pub(super) copy: &'static str,
}

impl ConversionRecipe {
    pub(super) fn grams(self) -> f64 {
        self.components
            .iter()
            .map(|component| component.grams())
            .sum()
    }

    pub(super) const fn composite(self) -> bool {
        self.components.len() > 1
    }
}

const CAC_AVOCADO: RecipeSource = RecipeSource {
    label: "California Avocado Commission sizes ↗",
    url: "https://californiaavocado.com/wp-content/uploads/2023/02/CAC-Foodservice-Sizes-Yields-2-2-23.pdf",
};
const FDA_SERVINGS: RecipeSource = RecipeSource {
    label: "FDA serving references ↗",
    url: "https://www.ecfr.gov/current/title-21/chapter-I/subchapter-B/part-101/subpart-A/section-101.12",
};
const USDA_EXTRACTION: RecipeSource = RecipeSource {
    label: "USDA extraction benchmark ↗",
    url: "https://ers.usda.gov/sites/default/files/_laserfiche/outlooks/39633/48594_whs04k01.pdf?v=29137",
};
const MCDONALDS_PATTY: RecipeSource = RecipeSource {
    label: "McDonald’s 10:1 patty ↗",
    url: "https://www.mcdonalds.co.jp/campaign/thankyou50th/people/everyone03/",
};
const MOTTET_FEED: RecipeSource = RecipeSource {
    label: "Mottet et al. ↗",
    url: "https://doi.org/10.1016/j.gfs.2017.01.001",
};
const USDA_WHEAT_MOISTURE: RecipeSource = RecipeSource {
    label: "USDA wheat moisture basis ↗",
    url: "https://data.nass.usda.gov/Surveys/Guide_to_NASS_Surveys/Prices/Price_Program_Methodology_v11_03092015.pdf",
};
const MCDONALDS_BUN: RecipeSource = RecipeSource {
    label: "McDonald’s regular bun ↗",
    url: "https://fe2.mcdonalds.ro/produse/vita/cheeseburger",
};
const ASB_BUN: RecipeSource = RecipeSource {
    label: "ASB hamburger-bun formula ↗",
    url: "https://asbe.org/article/hamburger-bun/",
};
const FRENCH_BAGUETTE_WEIGHT: RecipeSource = RecipeSource {
    label: "French bakers’ confederation ↗",
    url: "https://boulangerie.org/reglementation/etiquetage-poids-des-pains/",
};
const ASB_BAGUETTE: RecipeSource = RecipeSource {
    label: "ASB baguette formula ↗",
    url: "https://asbe.org/article/baguette/",
};
const BAKING_LOSS: RecipeSource = RecipeSource {
    label: "bread baking-loss study ↗",
    url: "https://pmc.ncbi.nlm.nih.gov/articles/PMC7918450/",
};
const SCA_BREW: RecipeSource = RecipeSource {
    label: "SCA brewer requirements ↗",
    url: "https://sca.coffee/s/2017-SCA-CHB-Program-Requirements-ba6g.pdf",
};
const SCA_ESPRESSO: RecipeSource = RecipeSource {
    label: "SCA espresso research ↗",
    url: "https://sca.coffee/sca-news/25-magazine/issue-3/defining-ever-changing-espresso-25-magazine-issue-3-zyx36",
};
const COFFEE_ROAST_LOSS: RecipeSource = RecipeSource {
    label: "medium-roast study ↗",
    url: "https://academic.oup.com/ijfst/article/60/2/vvaf189/8262800",
};
const COCA_COLA_SUGAR: RecipeSource = RecipeSource {
    label: "Coca-Cola product answer ↗",
    url: "https://www.coca-cola.com/us/en/about-us/faq/how-much-sugar-is-in-coca-cola",
};
const DUNKIN_NUTRITION: RecipeSource = RecipeSource {
    label: "Dunkin’ nutrition guide ↗",
    url: "https://www.dunkindonuts.com/content/dam/dd/pdf/nutrition.pdf?affiliateid=4",
};
const FDA_TEASPOON: RecipeSource = RecipeSource {
    label: "FDA teaspoon conversion ↗",
    url: "https://www.fda.gov/downloads/Food/FoodScienceResearch/ToolsMaterials/UCM586423.pdf",
};

const AVOCADO_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "one avocado",
    factors: &[RecipeFactor {
        operator: FactorOperator::Start,
        value: 200.0,
        display: "200 g avocado",
        unit: "rounded market-size fruit",
        approximate: true,
        citation: RecipeCitation {
            id: "crop-recipe-avocado-size",
            kind: SourceKind::IndustryBenchmark,
            detail: "California’s predominant food-service size is about 7 oz (198 g). The calculator rounds that benchmark to 200 g; individual fruit varies.",
            sources: &[CAC_AVOCADO],
        },
    }],
    result_unit: "avocado / avocado",
    approximate: true,
    decimals: 0,
    copy: "A rounded produce-size ruler, not a claim that every avocado weighs exactly 200 g.",
}];

pub(super) const AVOCADO_RECIPE: ConversionRecipe = ConversionRecipe {
    components: AVOCADO_COMPONENTS,
    total_unit: "avocado / avocado",
    approximate: true,
    decimals: 0,
    copy: "California’s predominant food-service size is about 7 oz (198 g); this model rounds it to 200 g.",
};

const TOAST_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "avocado on one toast",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 200.0,
            display: "200 g avocado",
            unit: "rounded market-size fruit",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-toast-avocado-size",
                kind: SourceKind::IndustryBenchmark,
                detail: "California’s predominant food-service size is about 7 oz (198 g). The calculator rounds it to 200 g.",
                sources: &[CAC_AVOCADO],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Multiply,
            value: 0.5,
            display: "½ avocado",
            unit: "per toast",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-toast-half",
                kind: SourceKind::ModelingChoice,
                detail: "The calculator assigns half of its rounded 200 g avocado to one toast. This is a serving choice, not measured restaurant data.",
                sources: &[CAC_AVOCADO],
            },
        },
    ],
    result_unit: "avocado / avocado toast",
    approximate: true,
    decimals: 0,
    copy: "Half of the rounded avocado benchmark is assigned to one toast.",
}];

pub(super) const TOAST_RECIPE: ConversionRecipe = ConversionRecipe {
    components: TOAST_COMPONENTS,
    total_unit: "avocado / avocado toast",
    approximate: true,
    decimals: 0,
    copy: "This is an explicit half-avocado serving choice, not a restaurant recipe.",
};

const GUAC_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "two-avocado bowl",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 2.0,
            display: "2 avocados",
            unit: "model bowl",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-guac-count",
                kind: SourceKind::ModelingChoice,
                detail: "The bowl is deliberately defined as two avocados so the comparison remains concrete. It is not a restaurant serving claim.",
                sources: &[CAC_AVOCADO],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Multiply,
            value: 200.0,
            display: "200 g avocado",
            unit: "rounded per fruit",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-guac-avocado-size",
                kind: SourceKind::IndustryBenchmark,
                detail: "California’s predominant food-service size is about 7 oz (198 g). The calculator rounds that benchmark to 200 g.",
                sources: &[CAC_AVOCADO],
            },
        },
    ],
    result_unit: "avocado / guac bowl",
    approximate: true,
    decimals: 0,
    copy: "Two rounded market-size avocados make this deliberately named model bowl.",
}];

pub(super) const GUAC_RECIPE: ConversionRecipe = ConversionRecipe {
    components: GUAC_COMPONENTS,
    total_unit: "avocado / two-avocado guac bowl",
    approximate: true,
    decimals: 0,
    copy: "A two-avocado model bowl, not a claim about any restaurant’s guacamole portion.",
};

const PASTA_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "dry pasta back to farm-gate wheat",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 56.0,
            display: "56 g dry pasta",
            unit: "FDA bulk-serving reference",
            approximate: false,
            citation: RecipeCitation {
                id: "crop-recipe-pasta-serving",
                kind: SourceKind::IndustryBenchmark,
                detail: "FDA’s reference table gives 55 g for dry pasta. It also permits a 2 oz (56 g) visual unit for dry bulk pasta such as spaghetti. This model uses 56 g.",
                sources: &[FDA_SERVINGS],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Divide,
            value: 0.74,
            display: "0.74",
            unit: "flour / harvested wheat",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-pasta-extraction",
                kind: SourceKind::IndustryBenchmark,
                detail: "USDA uses a 74% flour-extraction benchmark. The model treats dry pasta mass as flour-equivalent, then scales it back to harvested wheat.",
                sources: &[USDA_EXTRACTION],
            },
        },
    ],
    result_unit: "farm-gate wheat / bowl",
    approximate: true,
    decimals: 0,
    copy: "The dry serving is treated as flour-equivalent and divided by a 74% milling extraction rate.",
}];

pub(super) const PASTA_RECIPE: ConversionRecipe = ConversionRecipe {
    components: PASTA_COMPONENTS,
    total_unit: "farm-gate wheat / bowl of pasta",
    approximate: true,
    decimals: 0,
    copy: "A wheat-yield proxy: 56 g dry pasta is treated as flour-equivalent and scaled back through milling.",
};

const BURGER_COMPONENTS: &[RecipeComponent] = &[
    RecipeComponent {
        label: "beef · feed-crop equivalent",
        factors: &[
            RecipeFactor {
                operator: FactorOperator::Start,
                value: 45.0,
                display: "45 g beef",
                unit: "McDonald’s 10:1 patty",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-patty",
                    kind: SourceKind::MeasuredProductData,
                    detail: "McDonald’s Japan describes its regular 10:1 patty as one-tenth of a pound, approximately 45 g. The figure is pre-cooking product mass.",
                    sources: &[MCDONALDS_PATTY],
                },
            },
            RecipeFactor {
                operator: FactorOperator::Multiply,
                value: 9.4,
                display: "9.4 g feed DM",
                unit: "human-edible / g beef",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-feed",
                    kind: SourceKind::IndustryBenchmark,
                    detail: "A global livestock-feed study led by Anne Mottet reports 9.4 kg of human-edible feed dry matter per kg of deboned meat for OECD cattle feedlots. It is a production-system benchmark, not McDonald’s supplier data. Grass, residues, and other non-human-edible feed sit outside this factor; they are not assumed harmless.",
                    sources: &[MOTTET_FEED],
                },
            },
            RecipeFactor {
                operator: FactorOperator::Divide,
                value: 0.865,
                display: "0.865 g wheat DM",
                unit: "per g harvested wheat",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-moisture",
                    kind: SourceKind::ModelingChoice,
                    detail: "USDA’s standard wheat basis is 13.5% moisture, leaving 86.5% dry matter. Dividing by 0.865 expresses the aggregate feed dry matter on this calculator’s harvested-wheat yield basis.",
                    sources: &[USDA_WHEAT_MOISTURE],
                },
            },
        ],
        result_unit: "wheat-yield proxy",
        approximate: true,
        decimals: 0,
        copy: "The feed subtotal maps an OECD feedlot’s human-edible feed dry matter onto harvested wheat mass so it can share the calculator’s wheat-yield denominator. Non-human-edible feed has no single defensible wheat conversion here, so it is omitted—not counted as impact-free.",
    },
    RecipeComponent {
        label: "bun · farm-gate wheat",
        factors: &[
            RecipeFactor {
                operator: FactorOperator::Start,
                value: 50.0,
                display: "50 g bun",
                unit: "McDonald’s regular bun",
                approximate: false,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-bun-mass",
                    kind: SourceKind::MeasuredProductData,
                    detail: "McDonald’s Romania lists the regular cheeseburger bun at 50 g. Product specifications can vary by market; this supplies a concrete bun mass.",
                    sources: &[MCDONALDS_BUN],
                },
            },
            RecipeFactor {
                operator: FactorOperator::Divide,
                value: 0.75,
                display: "0.75",
                unit: "baked mass / dough mass",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-bake-yield",
                    kind: SourceKind::IndustryBenchmark,
                    detail: "Bread research describes average baking weight loss around 25%. The recipe therefore uses 75% retained mass to estimate dough from the finished bun.",
                    sources: &[BAKING_LOSS],
                },
            },
            RecipeFactor {
                operator: FactorOperator::Multiply,
                value: 100.0 / 187.5,
                display: "100 / 187.5",
                unit: "flour share of dough",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-bun-formula",
                    kind: SourceKind::IndustryBenchmark,
                    detail: "The American Society of Baking’s straight-dough hamburger-bun formula totals about 187.5 baker’s-percent units for every 100 units of flour.",
                    sources: &[ASB_BUN],
                },
            },
            RecipeFactor {
                operator: FactorOperator::Divide,
                value: 0.74,
                display: "0.74",
                unit: "flour / harvested wheat",
                approximate: true,
                citation: RecipeCitation {
                    id: "crop-recipe-burger-bun-extraction",
                    kind: SourceKind::IndustryBenchmark,
                    detail: "USDA uses a 74% flour-extraction benchmark. Dividing estimated flour by 0.74 expresses the bun on a farm-gate wheat basis.",
                    sources: &[USDA_EXTRACTION],
                },
            },
        ],
        result_unit: "farm-gate wheat",
        approximate: true,
        decimals: 0,
        copy: "Finished bun mass is expanded to dough, reduced to its formula’s flour share, then scaled back through milling.",
    },
];

pub(super) const BURGER_RECIPE: ConversionRecipe = ConversionRecipe {
    components: BURGER_COMPONENTS,
    total_unit: "wheat-yield proxy / cheeseburger",
    approximate: true,
    decimals: 0,
    copy: "This is an OECD feedlot benchmark expressed as a wheat-yield proxy. It counts the benchmark’s human-edible feed, not every feed input or land-management impact. It does not claim McDonald’s cattle literally ate this much wheat, or that McDonald’s supply chain was measured.",
};

const BAGUETTE_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "finished baguette back to farm-gate wheat",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 250.0,
            display: "250 g baguette",
            unit: "modeled finished loaf",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-baguette-mass",
                kind: SourceKind::IndustryBenchmark,
                detail: "France’s national bakers’ confederation says there is no single legal baguette weight. Trade usage around Paris is 250 g. Some regions use 200 g. This model chooses 250 g for its medium baguette.",
                sources: &[FRENCH_BAGUETTE_WEIGHT],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Divide,
            value: 0.75,
            display: "0.75",
            unit: "baked mass / dough mass",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-baguette-bake-yield",
                kind: SourceKind::IndustryBenchmark,
                detail: "Bread research describes average baking weight loss around 25%. The recipe uses 75% retained mass to estimate dough from the finished baguette.",
                sources: &[BAKING_LOSS],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Multiply,
            value: 100.0 / 173.4,
            display: "100 / 173.4",
            unit: "flour share of dough",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-baguette-formula",
                kind: SourceKind::ModelingChoice,
                detail: "The American Society of Baking gives a water range of 65–75% and a yeast range of 0.8–1.0%. It also gives 2% salt and 0.5% malt. This model chooses the range midpoints. The ingredients total 173.4 baker’s-percent units for every 100 units of flour.",
                sources: &[ASB_BAGUETTE],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Divide,
            value: 0.74,
            display: "0.74",
            unit: "flour / harvested wheat",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-baguette-extraction",
                kind: SourceKind::IndustryBenchmark,
                detail: "USDA uses a 74% flour-extraction benchmark. Dividing the estimated flour by 0.74 expresses it as farm-gate wheat.",
                sources: &[USDA_EXTRACTION],
            },
        },
    ],
    result_unit: "farm-gate wheat / baguette",
    approximate: true,
    decimals: 0,
    copy: "The 250 g finished-loaf model is expanded through baking loss, reduced to the baguette formula’s flour share, then scaled back through milling.",
}];

pub(super) const BAGUETTE_RECIPE: ConversionRecipe = ConversionRecipe {
    components: BAGUETTE_COMPONENTS,
    total_unit: "farm-gate wheat / medium baguette",
    approximate: true,
    decimals: 0,
    copy: "A 250 g model based on Parisian trade usage, not a universal baguette weight; regional bakery conventions vary.",
};

const CUP_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "brewed cup back to green coffee",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 0.240,
            display: "240 mL cup",
            unit: "0.240 L brewed water",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-coffee-cup-volume",
                kind: SourceKind::ModelingChoice,
                detail: "The calculator defines one familiar cup as 240 mL. Brew vessels and actual poured volumes vary.",
                sources: &[SCA_BREW],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Multiply,
            value: 55.0,
            display: "55 g/L",
            unit: "roasted coffee / water",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-coffee-brew-ratio",
                kind: SourceKind::IndustryBenchmark,
                detail: "SCA home-brewer requirements use approximately 55 g of roasted coffee per litre at full capacity. It is a brewing benchmark, not every drinker’s recipe.",
                sources: &[SCA_BREW],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Divide,
            value: 0.8374,
            display: "0.8374",
            unit: "roasted mass / green mass",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-coffee-roast-yield",
                kind: SourceKind::IndustryBenchmark,
                detail: "A roasting study measured 16.26% mass loss for its medium roast. The retained share is 1 − 0.1626 = 0.8374.",
                sources: &[COFFEE_ROAST_LOSS],
            },
        },
    ],
    result_unit: "green coffee / cup",
    approximate: true,
    decimals: 1,
    copy: "The SCA brew ratio supplies roasted coffee mass; medium-roast loss scales it back to green coffee.",
}];

pub(super) const CUP_RECIPE: ConversionRecipe = ConversionRecipe {
    components: CUP_COMPONENTS,
    total_unit: "green coffee / cup",
    approximate: true,
    decimals: 1,
    copy: "A 240 mL cup at the SCA brewing benchmark, translated through one measured medium-roast loss.",
};

const ESPRESSO_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "roasted espresso dose back to green coffee",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 18.0,
            display: "18 g dose",
            unit: "roasted coffee",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-espresso-dose",
                kind: SourceKind::IndustryBenchmark,
                detail: "SCA survey research found that the average espresso dose clustered at 18–20 g. Most respondents used 18 g baskets. This model takes 18 g.",
                sources: &[SCA_ESPRESSO],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Divide,
            value: 0.8374,
            display: "0.8374",
            unit: "roasted mass / green mass",
            approximate: true,
            citation: RecipeCitation {
                id: "crop-recipe-espresso-roast-yield",
                kind: SourceKind::IndustryBenchmark,
                detail: "A roasting study measured 16.26% mass loss for its medium roast, leaving a 0.8374 retained-mass factor.",
                sources: &[COFFEE_ROAST_LOSS],
            },
        },
    ],
    result_unit: "green coffee / double espresso",
    approximate: true,
    decimals: 1,
    copy: "An 18 g roasted dose is scaled back through the study’s medium-roast mass loss.",
}];

pub(super) const ESPRESSO_RECIPE: ConversionRecipe = ConversionRecipe {
    components: ESPRESSO_COMPONENTS,
    total_unit: "green coffee / double espresso",
    approximate: true,
    decimals: 1,
    copy: "An SCA-observed espresso dose translated to green-coffee mass; cafés and roasts vary.",
};

const COKE_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "sugar in one can",
    factors: &[RecipeFactor {
        operator: FactorOperator::Start,
        value: 39.0,
        display: "39 g sugar",
        unit: "12 oz can",
        approximate: false,
        citation: RecipeCitation {
            id: "crop-recipe-coke-sugar",
            kind: SourceKind::MeasuredProductData,
            detail: "Coca-Cola reports 39 g of sugar in a 12 oz U.S. can. The calculator compares that mass with recoverable cane sugar; it does not trace the can’s sweetener to Brazil or to cane.",
            sources: &[COCA_COLA_SUGAR],
        },
    }],
    result_unit: "sugar / can",
    approximate: false,
    decimals: 0,
    copy: "The product’s sugar mass is used as an analogy for the field’s recoverable sugar mass.",
}];

pub(super) const COKE_RECIPE: ConversionRecipe = ConversionRecipe {
    components: COKE_COMPONENTS,
    total_unit: "sugar / can of Coke",
    approximate: false,
    decimals: 0,
    copy: "A sugar-mass analogy, not a claim that this Coke’s sweetener came from the modeled Brazilian cane field.",
};

const DONUT_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "added sugar in one donut",
    factors: &[RecipeFactor {
        operator: FactorOperator::Start,
        value: 12.0,
        display: "12 g added sugar",
        unit: "Dunkin’ glazed donut",
        approximate: false,
        citation: RecipeCitation {
            id: "crop-recipe-donut-sugar",
            kind: SourceKind::MeasuredProductData,
            detail: "Dunkin’s nutrition guide lists 12 g added sugar for one glazed donut. Menus and formulations can change; this is the cited product entry.",
            sources: &[DUNKIN_NUTRITION],
        },
    }],
    result_unit: "added sugar / donut",
    approximate: false,
    decimals: 0,
    copy: "The labeled added-sugar mass is compared with recoverable cane sugar; no sourcing chain is implied.",
}];

pub(super) const DONUT_RECIPE: ConversionRecipe = ConversionRecipe {
    components: DONUT_COMPONENTS,
    total_unit: "added sugar / Dunkin’ glazed donut",
    approximate: false,
    decimals: 0,
    copy: "A product-label sugar-mass analogy, not a claim about Dunkin’s sugar supplier.",
};

const TEA_COMPONENTS: &[RecipeComponent] = &[RecipeComponent {
    label: "three teaspoons of sugar",
    factors: &[
        RecipeFactor {
            operator: FactorOperator::Start,
            value: 3.0,
            display: "3 tsp",
            unit: "table sugar / tea",
            approximate: false,
            citation: RecipeCitation {
                id: "crop-recipe-tea-spoons",
                kind: SourceKind::ModelingChoice,
                detail: "The recipe explicitly chooses three level teaspoons of table sugar in one tea. It is not a survey of how people sweeten tea.",
                sources: &[FDA_TEASPOON],
            },
        },
        RecipeFactor {
            operator: FactorOperator::Multiply,
            value: 4.2,
            display: "4.2 g/tsp",
            unit: "FDA table-sugar conversion",
            approximate: false,
            citation: RecipeCitation {
                id: "crop-recipe-tea-teaspoon-mass",
                kind: SourceKind::IndustryBenchmark,
                detail: "FDA classroom material gives one teaspoon of sugar a mass of 4.2 g. Three teaspoons therefore equal 12.6 g.",
                sources: &[FDA_TEASPOON],
            },
        },
    ],
    result_unit: "sugar / tea",
    approximate: false,
    decimals: 1,
    copy: "The named recipe is three teaspoons, multiplied by FDA’s 4.2 g-per-teaspoon conversion.",
}];

pub(super) const TEA_RECIPE: ConversionRecipe = ConversionRecipe {
    components: TEA_COMPONENTS,
    total_unit: "sugar / tea with 3 tsp sugar",
    approximate: false,
    decimals: 1,
    copy: "An explicit three-teaspoon recipe, not a generic estimate for sweetened tea.",
};
