//! Number formatting for the flight receipt. The original used
//! `Intl.NumberFormat('en-US')`; here the en-US comma thousands grouping is
//! hand-rolled so the SSR output matches ("8,000", "1,234,567").

use thoughts_core::decimal::format_grouped;
#[cfg(test)]
use thoughts_core::decimal::round_decimal_str;

pub fn format_km(km: f64) -> String {
    format!("{} km", format_grouped(km.round(), 0))
}

pub fn format_tonnes(t: f64) -> String {
    format!("{} t", format_grouped(t, 1))
}

pub fn format_tonnes_smart(t: f64) -> String {
    if t < 0.01 {
        return "<0.01 t".to_string();
    }
    if t < 0.1 {
        return format!("{t:.2} t");
    }
    format_tonnes(t)
}

pub fn format_litres(l: f64) -> String {
    format!("{} L", format_grouped(l.round(), 0))
}

pub fn format_ice(m2: f64) -> String {
    format!("{} m²", format_grouped(m2, 1))
}

fn round_to_sig(n: f64, sig: i32) -> f64 {
    let magnitude = 10f64.powi(n.log10().floor() as i32 - (sig - 1));
    (n / magnitude).round() * magnitude
}

/// Round to the friendliest number that stays honest: one significant figure
/// when that's within ~12% of the true value ("8,000", not "7,600"), otherwise
/// two significant figures.
pub fn round_count(n: f64) -> f64 {
    if n < 10.0 {
        return n.round().max(1.0);
    }
    let coarse = round_to_sig(n, 1);
    if (coarse - n).abs() / n <= 0.12 {
        return coarse;
    }
    round_to_sig(n, 2)
}

pub fn format_count(n: f64) -> String {
    format_grouped(round_count(n), 0)
}

pub fn format_whole(n: f64) -> String {
    format_grouped(n.round(), 0)
}

pub fn format_bar_value(kg: f64) -> String {
    if kg < 100.0 {
        if kg < 10.0 {
            return if kg < 1.0 {
                format!("{kg:.2} kg")
            } else {
                format!("{kg:.1} kg")
            };
        }
        return format!("{} kg", kg.round() as i64);
    }
    format_tonnes_smart(kg / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_matches_en_us() {
        assert_eq!(format_grouped(1234567.0, 0), "1,234,567");
        assert_eq!(format_grouped(8000.0, 0), "8,000");
        assert_eq!(format_grouped(999.0, 0), "999");
    }

    #[test]
    fn rounds_half_away_from_zero_like_intl() {
        // Intl rounds the shortest decimal repr half away from zero: 12.35
        // is "12.4" even though the double is 12.3499… (toFixed says 12.3).
        assert_eq!(format_tonnes(12.35), "12.4 t");
        assert_eq!(format_tonnes(0.05), "0.1 t");
    }

    #[test]
    fn carry_propagates_through_the_integer_part() {
        assert_eq!(format_tonnes(999.95), "1,000.0 t");
        assert_eq!(round_decimal_str("999.95", 1), "1000.0");
        assert_eq!(round_decimal_str("9.99", 1), "10.0");
    }

    #[test]
    fn tonnes_smart_bands() {
        assert_eq!(format_tonnes_smart(0.004), "<0.01 t");
        assert_eq!(format_tonnes_smart(0.05), "0.05 t");
        assert_eq!(format_tonnes_smart(0.5), "0.5 t");
        assert_eq!(format_tonnes_smart(3.456), "3.5 t");
    }

    #[test]
    fn round_count_prefers_friendly_but_honest() {
        assert_eq!(round_count(7600.0), 8000.0); // within 12% of 1 sig fig
        assert_eq!(round_count(14.0), 14.0); // 10 would be off by 28%
        assert_eq!(round_count(0.3), 1.0); // never rounds to zero
    }
}
