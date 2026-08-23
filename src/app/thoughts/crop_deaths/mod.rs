//! A deliberately draft-like argument about crop deaths. The prose is a set
//! of numbered handles to rewrite; the calculator is the finished claim-audit
//! mechanism underneath it.

mod calculator;

crate::register_post!(
    essay,
    slug: "crop-deaths",
    shortlink: "crops",
    title: "So you think you care about crop deaths…",
    date: "2026-08-23",
    teaser: "I've spent too much time arguing about this online",
    read_label: "audit the claim →",
    tags: &["animals", "food", "veganism"],
);

use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{page, query_params},
    view::view,
};

use crate::components::{doc_head, ext_link, full_bleed, inline_popover, shell};

const CROP_DEATHS_JS: Asset = asset!("./crop-deaths.js");
const SOME_GUY_AVATAR: Asset = asset!("./assets/some-guy-avatar.webp");

#[query_params(error = redirect("?"))]
struct CropDeathsQuery {
    food: Option<String>,
    rate: Option<String>,
    meal: Option<String>,
}

#[page("/thoughts/crop-deaths")]
async fn crop_deaths(cx: &Cx) -> Result {
    let query = query_params::<CropDeathsQuery>(cx)?;
    let calculator_state = calculator::state(
        query.food.as_deref(),
        query.rate.as_deref(),
        query.meal.as_deref(),
    );

    view! {
        shell(title: POST.title, active: "", hide_nav: true, runtime: false,
            <article class="crop-deaths">
                full_bleed(class: "crop-hero-band",
                    <div class="crop-page-wrap crop-hero">
                        doc_head(stamp: POST.date, title: POST.title)
                        <div class="crop-hero-copy">
                            <p>
                                "\"Vegans kill more animals by eating plants than meat eaters do by eating animals\""
                            </p>
                        </div>
                    </div>
                )

                <section class="crop-page-wrap crop-section crop-opening" aria-labelledby="comment-title">
                    <div class="crop-section-heading">
                        <p id="comment-title" class="crop-section-number">"01 / the comment"</p>
                    </div>

                    <div class="crop-comment-card">
                        <div class="crop-comment-meta">
                            <img
                                class="crop-comment-avatar"
                                src=(SOME_GUY_AVATAR)
                                alt="A man in sunglasses looking down at the camera from inside a car"
                            >
                            <p><strong>"Some guy"</strong><small>"13m"</small></p>
                        </div>
                        <blockquote>
                            <p>
                                "Vegans kill more animals than meat eaters. Combines shred mice. Pesticides poison insects. Your avocados kill squirrels and your coffee kills thousands of bugs. One grass-fed cow can feed a family for months."
                            </p>
                        </blockquote>
                    </div>

                    <div class="crop-argument-card">
                        <p>"Things I agree with"</p>
                        <ol data-navigable="">
                            <li>
                                inline_popover(
                                          id: "crop-premise-one",
                                          label: "Growing plants harms and kills sentient animals.",
                                          <span class="inline-popover-copy">
                                              <span class="inline-popover-paragraph">
                                                  "Pretty much all human use of land harms animals."
                                              </span>
                                              <span class="inline-popover-paragraph">
                                                  "Farming repeats that harm. Animals deemed pests may be killed again and again to keep a field productive."
                                              </span>
                                              <span class="inline-popover-paragraph">
                                                  "A building also displaces animals, but it is less likely to require the same recurring \"de-pesting\"."
                                              </span>
                                          </span>
                                )
                            </li>
                            <li>"It is effectively impossible to completely eliminate harm to animals, especially not simply by avoiding direct consumption of animal products."</li>
                            <li>"It seems likely that the greatest reduction of animal harm involves opting out of the system entirely and becoming a homesteader, which most vegans are not."</li>
                        </ol>
                        <p class="crop-argument-verdict">
                            "These are, in fact, great points, but the conclusions people typically draw from them are not great."
                        </p>
                    </div>
                </section>

                <section class="crop-page-wrap crop-section" aria-labelledby="notes-title">
                    <div class="crop-section-heading">
                        <p class="crop-section-number">"02 / why people should not be persuaded by the argument"</p>
                        <h2 id="notes-title">"Objections to this argument"</h2>
                    </div>

                    <ol class="crop-notes" data-navigable="">
                        <li>
                            <h3>"Farmed animals eat crops too"</h3>
                            <p>"
                                Roughly "
                                inline_popover(
                                    id: "crop-us-corn-use",
                                    label: "40% of U.S. domestic corn use goes to livestock feed",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">
                                            "USDA’s Economic Research Service says livestock feed typically accounts for about 40% of domestic corn use."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "Fuel ethanol has separately accounted for more than 40% in recent years."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "Those shares move over time. They do not mean every harvest is divided into a fixed 40% feed and 40% ethanol split."
                                        </span>
                                    </span>
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://ers.usda.gov/topics/crops/corn-and-other-feed-grains/feed-grains-sector-at-a-glance",
                                        label: "USDA feed use ↗"
                                    )
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://ers.usda.gov/data-products/chart-gallery/58346",
                                        label: "USDA ethanol use ↗"
                                    )
                                ) ". Direct food is a much smaller slice, alongside exports and
                                other industrial uses.
                            "</p>
                            <p>
                                inline_popover(
                                    id: "crop-global-soy-feed",
                                    label: "About 77% of the global soybean crop by weight goes to livestock feed",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">"
                                            The Food Climate Research Network estimate allocates 77%
                                            of global soy by mass to animal feed.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            Most soybeans are crushed into meal and oil. That makes soy
                                            a co-product system, not a crop in which each bean has only
                                            one purpose. By economic value, the animal-feed side still
                                            dominates.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            Saying the feed is all \"for cattle\" would be too narrow.
                                            Poultry and pigs consume large shares of soy meal too.
                                        "</span>
                                    </span>
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://tabledebates.org/sites/default/files/2021-12/FCRN%20Building%20Block%20-%20Soy_food%2C%20feed%2C%20and%20land%20use%20change%20%281%29.pdf",
                                        label: "Oxford FCRN report ↗"
                                    )
                                ) ". When you see stories about soy expansion and South American
                                deforestation, the demand story is overwhelmingly livestock feed,
                                not a tofu boom."
                            </p>
                            <p>"
                                Incidentally, I often eat soy meal sold as textured vegetable protein,
                                or TVP. It is mostly protein and fiber, and it works in all sorts of food.
                                I am boiling some as I type this. A vegan can also eat whole beans or
                                products such as Beyond Meat and Impossible Meat instead of feeding the
                                soy through an animal first.
                            "</p>
                            <p>"
                                One caveat is important: "
                                inline_popover(
                                    id: "crop-feed-edibility",
                                    label: "most livestock feed dry matter is not human-edible",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">
                                            "\"Human-edible\" answers a food-competition question. It does not measure field-animal deaths or make the other feed impact-free."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "The largest non-human-edible categories in Mottet et al.’s global ration are grass and leaves (46%) and crop residues such as straw and stover (19%)."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "Dedicated fodder crops can involve planting, pest control, and harvesting. Grazing and cut grass can also harm animals through land management."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "Crop residues are trickier. They come from a crop already grown for another output, so assigning its upstream deaths to the residue is a co-product choice—not a second field planted for feed."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "Livestock also consume about one-third of global cereal production. It's very typical for an animal to graze or forage on pastures for much of their life and then to be \"finished\" in a feedlot by essentially force-feeding them grains."
                                        </span>
                                    </span>
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://doi.org/10.1016/j.gfs.2017.01.001",
                                        label: "Mottet et al. ↗"
                                    )
                                )",
                                but the basic point still holds. Dedicated feed crops can have
                                crop deaths. Crop residues share the original crop’s upstream
                                harms.
                            "</p>
                            <p>
                                inline_popover(
                                    id: "crop-uncle-farm",
                                    label: "What about an animal that ate no crops?",
                                    heading: "A pet peeve",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">"
                                            I have seen dozens of people claim that
                                            they get all of their meat from some down-to-earth homesteading
                                            setup. I call this the \"my uncle's farm\" argument.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            First of all: I don't believe these people. Maybe they visited
                                            a good farm once upon a time. Maybe they really do get some of
                                            their meat from their uncle's farm upstate. Maybe they even
                                            homestead. I don't believe that they never go to restaurants,
                                            order food online, or buy meat from the grocery store. It's not
                                            terribly hard to have a homestead and outsource a lot of the
                                            things that vegans complain about.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            If all animal agriculture were replaced with so-called sustainable,
                                            local, non-industrialized farms with 100% grass-fed and -finished
                                            meat, a steak would cost hundreds of dollars and the majority of the
                                            world would have to become vegan out of necessity. The only way the
                                            economy can feed all Americans meat every day is through
                                            industrialization.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            That said, my general stance is that, if you are actually the person
                                            raising and slaughtering, hunting, or fishing 95% of the animal
                                            products you eat, I'm not talking to you and you can feel free to exit
                                            this page believing that you've bested me. I also believe that these
                                            patterns of behavior result in substantially reduced consumption of
                                            animal products.
                                        "</span>
                                    </span>
                                ) " Pastures and other direct land use from animal agriculture still
                                uses pesticides, still removes wild habitats, and can cause pollution
                                or other disruptions to local ecosystems. You are "
                                inline_popover(
                                    id: "crop-comparative-land",
                                    label: "still probably responsible for more overall deaths",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">
                                            "Joseph Poore and Thomas Nemecek assembled data from roughly 38,000 farms. Their study finds large variation among producers, but the conclusion is that animal products generally use much more land than plant substitutes."
                                        </span>
                                        <span class="inline-popover-paragraph">
                                            "More agricultural land creates more opportunities for habitat displacement and pest control. That is the inference used here."
                                        </span>
                                    </span>
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://ora.ox.ac.uk/objects/uuid:b0b53649-5e93-4415-bf07-6b0b1227172f",
                                        label: "paper + data ↗"
                                    )
                                ) " than a vegan who eats crops directly. But, even if this
                                weren't true, if you " <em>"really"</em> " care about crop deaths,
                                you wouldn't focus on vegans.
                            "</p>
                        </li>
                        <li>
                            <h3>"Not that much land can make a lot of food"</h3>
                            <p>"
                                Deaths attributed to one hectare over a crop year are not deaths per lunch.
                                The food from that hectare is divided across many meals.
                            "</p>
                            <p>"
                                The calculator below makes that scale visible. It is built from "
                                inline_popover(
                                    id: "crop-field-evidence",
                                    label: "sparse field evidence and explicit modeling choices",
                                    <span class="inline-popover-copy">
                                        <span class="inline-popover-paragraph">"
                                            A 2018 paper by Bob Fischer and Andy Lamey audits the thin
                                            empirical record. It explains why famous global totals depend
                                            on definitions and moral choices as well as field measurements.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            A 2023 Australian field study observed two machinery deaths
                                            among collared mice thought alive at harvest. The measured
                                            population decline was only marginal. That result is useful,
                                            but it is not a universal death rate.
                                        "</span>
                                        <span class="inline-popover-paragraph">"
                                            For pesticides, EPA's framework is the other warning:
                                            toxicity alone is not field mortality. Exposure, environmental
                                            fate, species, and uncertainty all change ecological risk.
                                        "</span>
                                    </span>
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://doi.org/10.1007/s10806-018-9733-8",
                                        label: "estimate audit ↗"
                                    )
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://doi.org/10.1002/ps.7670",
                                        label: "2023 field study ↗"
                                    )
                                    ext_link(
                                        class: "quiet-link crop-inline-source-link",
                                        href: "https://www.epa.gov/pesticide-science-and-assessing-pesticide-risks/factsheet-ecological-risk-assessment-pesticides",
                                        label: "EPA method ↗"
                                    )
                                ) ".
                            "</p>
                            <p>"
                                The beef example deliberately assigns a large feed-crop equivalent to
                                each cheeseburger. Even there, the calculation spreads one represented
                                field-animal death across the food produced from its share of a hectare.
                            "</p>
                        </li>
                        <li>
                            <h3>"I care about some animals more than others"</h3>
                            <p>"
                                I know there are some vegans who deny this, and there are even
                                more meat-eaters who will argue that I'm not vegan for thinking
                                this, but I do have a hierarchy of the kinds of animals I care
                                about. For example, I am a human supremacist: I would choose a
                                human's life over a hundred cows. (I suppose depending on the
                                human, but you know what I mean)
                            "</p>
                            <p>"
                                I'm also, generally, a mammal supremacist. This is related as
                                well to my size-supremacy; I believe that an elephant's death
                                is substantially more tragic than a chicken's. The hierarchy
                                here is not clear but one very obvious thing is: I don't care
                                that much about bugs/insects.
                            "</p>
                            <p>"
                                There is evidence that bugs have something approaching feelings,
                                but it's quite limited and, frankly, I'm just not that convinced.
                                My mental model of insects focuses on the hives or colonies:
                                ecologists often refer to these as \"super-organisms\".
                                Succinctly, if I had to estimate how much I care, killing one
                                queen bee/hive is similar to me to killing one mammal.
                            "</p>
                        </li>
                        <li>
                            <h3>"Quality of life is important"</h3>
                            <p>"
                                Generally, the animals who are killed through pesticides or other
                                de-pesting actions have lived their lives otherwise free of the
                                machinations of humanity. They were not caged and they have
                                largely intact and natural mating structures. Often they are
                                killed by accident or entirely incidentally. It's not like
                                \"that's ok\", but it is way better than the way we treat
                                farmed animals, on the whole.
                            "</p>
                        </li>
                        <li>
                            <h3>"Veganism isn't absolutist."</h3>
                            <p>"
                                I am a vegan and my choices are sometimes responsible for animal
                                deaths. It is not a tenet of veganism to completely eliminate any
                                animal deaths one is responsible for. It is a tenet of veganism to
                                reduce it as far as possible and practicable. I personally think
                                of veganism a little bit like frugality: a frugal person wastes
                                no money. If they can get a good stapler for $5 vs. a slightly
                                better stapler for $300, someone who purchases the $300 stapler
                                will have a hard time claiming they are \"frugal\". The fact
                                that the other stapler is not $0 is not a good reason for the
                                non-frugal person to throw their hands up in the air and say
                                \"you have to spend money either way\". If it were true that
                                vegans cause more crop deaths than non-vegans, I would agree
                                that there is a good point to be made. However, \"you're still
                                killing some animals, so you're not really vegan\" isn't a good
                                point. It's not even hypocritical for a frugal person to pay
                                a small price for something they " <em> "have" </em> " to buy.
                            "</p>
                        </li>
                        <li>
                            <h3>"Sometimes, it's just true!"</h3>
                            <p>"
                                Not every action that is typically categorized as vegan is a
                                good thing to do, per se.
                            "</p>
                            <p>"
                                Avocados and palm oil are often very bad for native ecosystems
                                and water usage. Rice produces significant amounts of methane
                                and fertilizer run-off. Many nuts are similarly horrible for
                                ecosystems or just otherwise inefficient.
                            "</p>
                            <p>"
                                Coffee is simply unnecessary, so it might be fair to say something
                                like \"anyone who buys coffee for any reason is not frugal\", and
                                similarly to say \"coffee isn't vegan because it's practicable to
                                simply not drink it\".
                            "</p>
                            <p>"
                                So I do try to avoid these, and I know many vegans
                                who do as well. \"You're not going far enough\" is often a fair
                                critique as long as it's not paired with \"so why are you even
                                bothering?\"
                            "</p>
                        </li>
                    </ol>
                </section>

                full_bleed(class: "crop-calculator-band",
                    <div class="crop-page-wrap">
                        calculator::calculator(state: calculator_state)
                    </div>
                )
                <script type="module" src=(CROP_DEATHS_JS)></script>
            </article>
        )
    }
}
