// Progressive enhancement for the lifestyle calculator. The server-rendered
// GET form remains the no-JavaScript path; once this module loads, every input
// updates the receipt in place and the current assumptions stay in the URL.

const calculator = document.querySelector("[data-crop-calculator]");
const form = calculator?.querySelector("[data-crop-controls]");

if (calculator && form) {
  const rateInput = form.querySelector("[data-crop-rate]");
  const rateSlider = form.querySelector("[data-crop-rate-slider]");
  const warning = form.querySelector("[data-crop-warning]");
  const mealTabs = form.querySelector("[data-crop-meal-tabs]");
  const recipePanels = [
    ...calculator.querySelectorAll("[data-crop-recipe-panel]"),
  ];

  const output = (name) => calculator.querySelector(`[data-crop-${name}]`);
  const write = (name, value) => {
    const node = output(name);
    if (node) node.textContent = value;
  };

  const numberFormats = [0, 1, 2, 3].map(
    (digits) =>
      new Intl.NumberFormat("en-US", {
        minimumFractionDigits: 0,
        maximumFractionDigits: digits,
      }),
  );

  const formatNumber = (value) => {
    const digits = value >= 100 ? 0 : value >= 10 ? 1 : value >= 1 ? 2 : 3;
    return numberFormats[digits].format(value);
  };

  const plural = (amount, singular, pluralForm) =>
    amount === 1 ? singular : pluralForm;

  const showWarning = (message) => {
    if (!warning) return;
    warning.textContent = message;
    warning.hidden = !message;
  };

  const selectedMealKey = () =>
    form.querySelector('input[name="meal"]:checked')?.value ?? null;

  const parseMeals = (scenario) => {
    try {
      return JSON.parse(scenario.dataset.meals || "[]");
    } catch {
      return [];
    }
  };

  const rangeFor = (scenario) => ({
    minimum: Number(scenario.dataset.rangeMin),
    maximum: Number(scenario.dataset.rangeMax),
  });

  const ratePosition = (rate, minimum, maximum) => {
    if (maximum <= minimum) return 50;
    const clamped = Math.min(maximum, Math.max(minimum, rate));
    return (
      ((Math.log(clamped) - Math.log(minimum)) /
        (Math.log(maximum) - Math.log(minimum))) *
      100
    );
  };

  const rateFromSlider = (scenario, sliderValue) => {
    const { minimum, maximum } = rangeFor(scenario);
    const fraction = Math.min(1000, Math.max(0, Number(sliderValue))) / 1000;
    return Math.exp(
      Math.log(minimum) + fraction * (Math.log(maximum) - Math.log(minimum)),
    );
  };

  const inputRate = (rate) => Number(rate.toPrecision(6));

  const sampleButtonsFor = (scenario) => {
    const group = [
      ...calculator.querySelectorAll("[data-crop-range-samples]"),
    ].find((candidate) => candidate.dataset.cropScenario === scenario.value);
    return [...(group?.querySelectorAll("[data-crop-sample-pick]") || [])];
  };

  const matchingSample = (scenario, rate) =>
    sampleButtonsFor(scenario).find(
      (sample) => Math.abs(Number(sample.dataset.cropSampleRate) - rate) < 1e-9,
    );

  const closestSample = (scenario, rate) =>
    sampleButtonsFor(scenario).reduce((closest, sample) => {
      if (!closest) return sample;
      const sampleDistance = Math.abs(
        Math.log(Number(sample.dataset.cropSampleRate) / rate),
      );
      const closestDistance = Math.abs(
        Math.log(Number(closest.dataset.cropSampleRate) / rate),
      );
      return sampleDistance < closestDistance ? sample : closest;
    }, null);

  const sampleLabel = (sample) =>
    sample?.querySelector("span")?.textContent || "landmark";

  const sampleDistanceStatus = (scenario, rate) => {
    const closest = closestSample(scenario, rate);
    return `closest: ${sampleLabel(closest)}`;
  };

  const rangeStatus = (scenario, rate) => {
    const sample = matchingSample(scenario, rate);
    if (sample) {
      return `sample · ${sampleLabel(sample)}`;
    }
    const { minimum, maximum } = rangeFor(scenario);
    if (rate < minimum) return `below range · ${sampleDistanceStatus(scenario, rate)}`;
    if (rate > maximum) return `above range · ${sampleDistanceStatus(scenario, rate)}`;
    return `closest · ${sampleLabel(closestSample(scenario, rate))}`;
  };

  const renderMealTabs = (meals, activeKey) => {
    if (!mealTabs) return;
    const chosen = meals.some((meal) => meal.key === activeKey)
      ? activeKey
      : meals[0]?.key;
    mealTabs.replaceChildren(
      ...meals.map((meal) => {
        const label = document.createElement("label");
        const input = document.createElement("input");
        input.type = "radio";
        input.name = "meal";
        input.value = meal.key;
        input.checked = meal.key === chosen;
        input.dataset.cropMeal = meal.key;
        const span = document.createElement("span");
        span.textContent = meal.singular;
        label.append(input, span);
        return label;
      }),
    );
    return chosen;
  };

  const showRecipePanel = (scenarioKey, mealKey) => {
    let selected = null;
    for (const panel of recipePanels) {
      const current =
        panel.dataset.cropScenario === scenarioKey &&
        panel.dataset.cropMeal === mealKey;
      panel.hidden = !current;
      if (current) {
        panel.removeAttribute("aria-hidden");
        selected = panel;
      } else {
        panel.setAttribute("aria-hidden", "true");
      }
    }
    return selected;
  };

  const syncUrl = (scenario, rate, meal) => {
    const url = new URL(window.location.href);
    url.searchParams.set("food", scenario.value);
    url.searchParams.set("rate", String(rate));
    url.searchParams.set("meal", meal);
    url.searchParams.delete("deaths");
    url.searchParams.delete("approach");
    window.history.replaceState(null, "", url);
  };

  const updateRateSlider = (scenario, rate) => {
    if (!rateSlider) return;
    const { minimum, maximum } = rangeFor(scenario);
    rateSlider.value = String(
      Math.round(ratePosition(rate, minimum, maximum) * 10),
    );
    rateSlider.setAttribute(
      "aria-valuetext",
      `${formatNumber(rate)} deaths per hectare per crop year`,
    );
  };

  const updateRange = (scenario, rate) => {
    const { minimum, maximum } = rangeFor(scenario);

    write("rate-min", formatNumber(minimum));
    write("rate-max", formatNumber(maximum));
    write("rate-status", rangeStatus(scenario, rate));

    for (const group of calculator.querySelectorAll("[data-crop-range-samples]")) {
      const activeGroup = group.dataset.cropScenario === scenario.value;
      group.hidden = !activeGroup;
      if (!activeGroup) continue;
      const closest = closestSample(scenario, rate);
      for (const article of group.querySelectorAll("[data-crop-range-sample]")) {
        const button = article.querySelector("[data-crop-sample-pick]");
        if (!button) continue;
        const current = button === closest;
        article.hidden = !current;
        if (current) button.dataset.current = "true";
        else button.removeAttribute("data-current");
      }
    }
  };

  const updateAltMeals = (meals, activeMeal, foodKg) => {
    const list = calculator.querySelector("[data-crop-alt-meals] ul");
    if (!list) return;
    list.replaceChildren(
      ...meals
        .filter((meal) => meal.key !== activeMeal.key)
        .map((meal) => {
          const count = foodKg / meal.cropKg;
          const item = document.createElement("li");
          item.dataset.altMeal = meal.key;
          const strong = document.createElement("strong");
          strong.dataset.altCount = "";
          strong.textContent = formatNumber(count);
          const span = document.createElement("span");
          span.dataset.altLabel = "";
          span.textContent = plural(count, meal.singular, meal.plural);
          item.append(strong, span);
          return item;
        }),
    );
  };

  const update = ({ fromClaim = false } = {}) => {
    const scenario = form.querySelector('input[name="food"]:checked');
    if (!scenario || !rateInput) return;

    const meals = parseMeals(scenario);
    if (!meals.length) return;

    let mealKey;
    if (fromClaim) {
      rateInput.value = scenario.dataset.defaultRate;
      mealKey = renderMealTabs(meals, meals[0].key);
    } else {
      mealKey = selectedMealKey();
      if (!meals.some((meal) => meal.key === mealKey)) {
        mealKey = renderMealTabs(meals, meals[0].key);
      }
    }

    const meal = meals.find((entry) => entry.key === mealKey) || meals[0];
    const recipePanel = showRecipePanel(scenario.value, meal.key);
    const recipeCropKg = Number(recipePanel?.dataset.cropKg);
    const cropKg = Number.isFinite(recipeCropKg) && recipeCropKg > 0
      ? recipeCropKg
      : meal.cropKg;
    const recipeUnit =
      recipePanel?.dataset.totalUnit || `${scenario.dataset.cropUnit} / ${meal.singular}`;
    const rate = rateInput.valueAsNumber;
    if (
      !rateInput.validity.valid ||
      !Number.isFinite(rate)
    ) {
      showWarning(
        "Enter positive values within the ranges shown to update the receipt.",
      );
      return;
    }

    showWarning("");

    const data = scenario.dataset;
    const yieldKg = Number(data.yieldKg);
    const cropUnit = data.cropUnit;
    const mealsPerHectare = yieldKg / cropKg;
    const hectares = 1 / rate;
    const foodKg = hectares * yieldKg;
    const mealCount = foodKg / cropKg;
    const mealLabel = plural(mealCount, meal.singular, meal.plural);
    const rateAnimalLabel = plural(rate, data.animalSingular, data.animalPlural);
    const formattedMealsPerHectare = formatNumber(mealsPerHectare);
    const formattedYieldKg = formatNumber(yieldKg);
    const formattedMealGrams =
      recipePanel?.dataset.displayGrams || formatNumber(cropKg * 1000);
    const formattedRate = formatNumber(rate);
    const conversionLabel = `${meal.plural} / hectare / crop year`;
    const rateLabel = `${rateAnimalLabel} / hectare / crop year`;
    const resultPerAnimalLabel = `${mealLabel} / ${data.animalSingular}`;

    updateRateSlider(scenario, rate);
    updateRange(scenario, rate);

    write("name", data.crop);
    write("region", data.region);
    write(
      "result-lede",
      `under your assumption, 1 ${data.animalSingular} corresponds to`,
    );
    write("result-units", formatNumber(mealCount));
    write("result-unit-label", mealLabel);
    write(
      "result-detail",
      `at ${formattedRate} ${rateAnimalLabel} / hectare / crop year · 1 ${data.animalSingular} represented`,
    );
    write("yield-kg", formattedYieldKg);
    write("yield-unit", `kg ${cropUnit} / hectare / crop year`);
    write("meal-grams", `${formattedMealGrams} g`);
    write("meal-grams-label", recipeUnit);
    write("conversion-meals", formattedMealsPerHectare);
    write("conversion-label", conversionLabel);
    write("division-meals", formattedMealsPerHectare);
    write("division-meals-label", conversionLabel);
    write("death-rate", formattedRate);
    write("death-rate-label", rateLabel);
    write("division-result", formatNumber(mealCount));
    write("division-result-label", resultPerAnimalLabel);
    write("yield-source", data.yieldSource);

    const equation = output("equation");
    if (equation) {
      equation.setAttribute(
        "aria-label",
        `The selected meal recipe evaluates to ${formattedMealGrams} grams of ${recipeUnit}. ${formattedYieldKg} kg ${cropUnit} per hectare per crop year times 1,000 grams per kilogram divided by ${formattedMealGrams} grams per ${meal.singular}, equals ${formattedMealsPerHectare} ${meal.plural} per hectare per crop year; ${formattedMealsPerHectare} ${meal.plural} per hectare per crop year divided by ${formattedRate} ${rateLabel}, equals ${formatNumber(mealCount)} ${resultPerAnimalLabel}`,
      );
    }

    updateAltMeals(meals, meal, foodKg);

    const yieldLink = output("yield-link");
    if (yieldLink) yieldLink.href = data.yieldUrl;

    syncUrl(scenario, rate, meal.key);
  };

  form.addEventListener("input", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement)) {
      update();
      return;
    }
    if (target.matches('input[name="food"]')) {
      update({ fromClaim: true });
    } else if (target.matches("[data-crop-rate-slider]")) {
      const scenario = form.querySelector('input[name="food"]:checked');
      if (scenario && rateInput) {
        rateInput.value = String(
          inputRate(rateFromSlider(scenario, target.value)),
        );
      }
      update();
    } else {
      update();
    }
  });

  calculator.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const sample = target.closest("[data-crop-sample-pick]");
    if (!sample || !calculator.contains(sample) || !rateInput) return;
    rateInput.value = sample.dataset.cropSampleRate || rateInput.value;
    update();
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    update();
  });
  form.dataset.enhanced = "true";
  update();
}
