/// Formats a finite number as a DICOM Decimal String of at most 16 bytes.
///
/// Callers must reject non-finite inputs at their data boundary. Nonzero values
/// retain their sign and magnitude, using scientific notation when needed.
#[must_use]
pub fn format_ds(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let shortest = value.to_string();
    if shortest.len() <= 16 {
        return shortest;
    }
    for precision in (0..=9).rev() {
        let candidate = compact_exponent(format!("{value:.precision$e}"));
        if candidate.len() <= 16 && candidate.parse::<f64>().is_ok_and(f64::is_finite) {
            return candidate;
        }
    }
    compact_exponent(format!("{value:.0e}"))
}

fn compact_exponent(mut value: String) -> String {
    if let Some(index) = value.find('e') {
        let exponent = value.split_off(index);
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
        value.push_str(&exponent.replace("e+", "e"));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::format_ds;

    #[test]
    fn ds_values_fit_the_vr_without_erasing_small_nonzero_values() {
        for value in [
            0.0,
            -0.0,
            1e-13,
            f64::EPSILON,
            -1.234_567_890_123_456e-120,
            f64::MAX,
            f64::from_bits(1),
        ] {
            let encoded = format_ds(value);
            assert!(encoded.len() <= 16, "{encoded:?}");
            let decoded = encoded.parse::<f64>().unwrap();
            assert!(decoded.is_finite(), "{encoded:?}");
            assert_eq!(decoded == 0.0, value == 0.0, "{encoded:?}");
            if value != 0.0 {
                assert_eq!(decoded.is_sign_negative(), value.is_sign_negative());
                assert!(
                    (decoded / value - 1.0).abs() < 1e-8,
                    "{value} became {encoded}"
                );
            }
        }
    }
}
