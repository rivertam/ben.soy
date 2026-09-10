//! Crop receipt arithmetic and the receipt's adaptive decimal precision.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Calculation {
    pub hectares: f64,
    pub food_kg: f64,
}

impl Calculation {
    pub fn new(yield_kg: f64, deaths_per_hectare: f64) -> Self {
        let hectares = 1.0 / deaths_per_hectare;
        Self {
            hectares,
            food_kg: hectares * yield_kg,
        }
    }

    pub fn meals(self, crop_kg: f64) -> f64 {
        self.food_kg / crop_kg
    }
}

#[derive(Serialize)]
pub struct Receipt {
    pub food_kg: f64,
    pub meal_count: f64,
    pub meals_per_hectare: f64,
}

impl Receipt {
    pub fn new(yield_kg: f64, rate: f64, crop_kg: f64) -> Self {
        let calculation = Calculation::new(yield_kg, rate);
        Self {
            food_kg: calculation.food_kg,
            meal_count: calculation.meals(crop_kg),
            meals_per_hectare: yield_kg / crop_kg,
        }
    }
}

pub fn format_number(value: f64) -> String {
    let decimals = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else if value >= 1.0 {
        2
    } else {
        3
    };
    format_decimal(value, decimals)
}

pub fn format_decimal(value: f64, decimals: usize) -> String {
    let mut text = crate::decimal::format_grouped(value, decimals);
    if decimals > 0 {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_ties_match_intl_in_every_precision_band() {
        for (value, expected) in [
            (1.005, "1.01"),
            (1.015, "1.02"),
            (9.995, "10"),
            (10.25, "10.3"),
            (0.1005, "0.101"),
            (999.5, "1,000"),
        ] {
            assert_eq!(format_number(value), expected, "{value}");
        }
        assert_eq!(format_decimal(-1.005, 2), "-1.01");
    }
    #[test]
    fn recipe_receipt_uses_one_shared_conversion() {
        let receipt = Receipt::new(4000.0, 2.0, 0.1);
        assert_eq!(receipt.meals_per_hectare, 40000.0);
        assert_eq!(receipt.meal_count, 20000.0);
        assert_eq!(receipt.food_kg, 2000.0);
    }
}
