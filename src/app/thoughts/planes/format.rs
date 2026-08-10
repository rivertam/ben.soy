//! Number formatting for the flight receipt. The original used
//! `Intl.NumberFormat('en-US')`; here the en-US comma thousands grouping is
//! hand-rolled so the SSR output matches ("8,000", "1,234,567").

/// Insert en-US comma grouping into a plain run of integer digits.
fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Round a plain decimal digit string ("999.95") to `decimals` fraction
/// digits, half away from zero, with carry ("1000.0").
fn round_decimal_str(s: &str, decimals: usize) -> String {
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if frac_part.len() <= decimals {
        let mut out = String::from(int_part);
        if decimals > 0 {
            out.push('.');
            out.push_str(frac_part);
            out.extend(std::iter::repeat_n('0', decimals - frac_part.len()));
        }
        return out;
    }
    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(decimals))
        .map(|b| b - b'0')
        .collect();
    if frac_part.as_bytes()[decimals] >= b'5' {
        let mut carried = true;
        for d in digits.iter_mut().rev() {
            if *d < 9 {
                *d += 1;
                carried = false;
                break;
            }
            *d = 0;
        }
        if carried {
            // A full carry out of the leading digit ("999.95" → "1000.0").
            digits.insert(0, 1);
        }
    }
    let int_len = digits.len() - decimals;
    let mut out: String = digits[..int_len]
        .iter()
        .map(|d| (d + b'0') as char)
        .collect();
    if decimals > 0 {
        out.push('.');
        out.extend(digits[int_len..].iter().map(|d| (d + b'0') as char));
    }
    out
}

/// `Intl.NumberFormat('en-US')` with a fixed number of fraction digits.
///
/// Intl rounds the number's shortest decimal representation half away from
/// zero — `12.35` → "12.4" even though the underlying double is 12.3499…
/// (which is why `toFixed(1)` says "12.3"). Match Intl, since that's what
/// the original page rendered.
fn format_grouped(n: f64, decimals: usize) -> String {
    let shortest = format!("{n}");
    let (sign, rest) = match shortest.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", shortest.as_str()),
    };
    let s = round_decimal_str(rest, decimals);
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut out = String::from(sign);
    out.push_str(&group_thousands(int_part));
    if let Some(f) = frac_part {
        out.push('.');
        out.push_str(f);
    }
    out
}

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
