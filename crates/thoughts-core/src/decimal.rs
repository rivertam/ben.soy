//! Intl-compatible decimal-string rounding shared by calculator renderers.

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
pub fn round_decimal_str(s: &str, decimals: usize) -> String {
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
pub fn format_grouped(n: f64, decimals: usize) -> String {
    if !n.is_finite() {
        return if n.is_nan() {
            "NaN".into()
        } else if n.is_sign_negative() {
            "-∞".into()
        } else {
            "∞".into()
        };
    }
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
