//! Integer-math replacements for `{:.1}`-style float formatting.
//!
//! Every float-Display call site removed keeps `core::fmt`'s float machinery
//! (Grisu/Dragon digit generation) out of the binary and out of the frame
//! path. The outputs here are byte-identical to `format!("{:.1}", …)` /
//! `format!("{:.0}", …)`: the rounding operand is derived from the f64's exact
//! mantissa/exponent and rounded half-to-even, exactly like Rust's decimal
//! digit generation (proven empirically by the differential tests below).

/// `num / 2^d`, ties to even.
fn round_half_even_div(num: u128, d: u32) -> u128 {
    if d == 0 {
        return num;
    }
    if d >= 128 {
        return 0;
    }
    let q = num >> d;
    let rem = num & ((1u128 << d) - 1);
    let doubled = rem << 1;
    let denom = 1u128 << d;
    if doubled > denom || (doubled == denom && q & 1 == 1) {
        q + 1
    } else {
        q
    }
}

/// `value * mul` (mul < 128), rounded to an integer, ties to even — computed
/// from the f64 bit decomposition so no double rounding can occur.
fn exact_scaled(value: f64, mul: u64) -> u128 {
    let bits = value.to_bits();
    debug_assert!(bits >> 63 == 0, "exact_scaled: negative {value}");
    let mantissa = bits & ((1u64 << 52) - 1);
    let biased = ((bits >> 52) & 0x7FF) as i32;
    let (m, e): (u128, i32) = if biased == 0 {
        (mantissa as u128, -1074) // subnormal
    } else {
        ((mantissa | (1u64 << 52)) as u128, biased - 1075)
    };
    let num = m * u128::from(mul);
    if e >= 0 {
        debug_assert!(e < 100, "exact_scaled: {value} out of domain");
        num << e.min(100)
    } else {
        round_half_even_div(num, (-e) as u32)
    }
}

/// `format!("{:width$.1}", value)` for finite `value >= 0`, via integer math.
pub(crate) fn tenths_str(value: impl Into<f64>, width: usize) -> String {
    let value = value.into();
    debug_assert!(value >= 0.0 && value.is_finite(), "tenths_str: {value}");
    let tenths = exact_scaled(value, 10);
    let body = format!("{}.{}", tenths / 10, tenths % 10);
    if body.len() >= width {
        body
    } else {
        format!("{}{body}", " ".repeat(width - body.len()))
    }
}

/// [`tenths_str`] writing into a caller-owned (pooled) string.
pub(crate) fn tenths_into(buf: &mut String, value: impl Into<f64>, width: usize) {
    let value = value.into();
    debug_assert!(value >= 0.0 && value.is_finite(), "tenths_into: {value}");
    let tenths = exact_scaled(value, 10);
    let whole = tenths / 10;
    buf.clear();
    let digits_len = usize::from(whole == 0) + if whole == 0 {
        0
    } else {
        whole.ilog10() as usize + 1
    };
    let total = digits_len + 2; // '.' + one decimal digit
    if width > total {
        buf.extend(std::iter::repeat_n(' ', width - total));
    }
    use std::fmt::Write as _;
    let _ = write!(buf, "{whole}.{}", tenths % 10);
}

/// `format!("{:.0}", value)` for finite `value >= 0`, via integer math.
pub(crate) fn round0_str(value: impl Into<f64>) -> String {
    let value = value.into();
    debug_assert!(value >= 0.0 && value.is_finite(), "round0_str: {value}");
    format!("{}", exact_scaled(value, 1))
}

#[cfg(test)]
mod tests {
    use super::{exact_scaled, round0_str, tenths_str};

    #[test]
    fn tenths_matches_format_at_boundaries() {
        // Values where binary/decimal rounding could disagree.
        let cases = [0.0f64, 0.05, 0.15, 0.25, 0.35, 0.5, 0.95, 1.5, 2.5, 99.95, 100.0];
        for &value in &cases {
            for width in [0usize, 4, 5, 6] {
                assert_eq!(
                    tenths_str(value, width),
                    format!("{:width$.1}", value),
                    "tenths_str({value}, {width})"
                );
            }
        }
    }

    #[test]
    fn tenths_matches_format_on_random_samples() {
        // Deterministic xorshift over f64 bit patterns (positive, finite,
        // magnitudes spanning tiny to ~1e9 after scaling).
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut checked = 0usize;
        for _ in 0..40_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let value = f64::from_bits(state & 0x7FFF_FFFF_FFFF_FFFF) * 1e-9;
            if !value.is_finite() || value >= 1e12 {
                continue;
            }
            checked += 1;
            assert_eq!(
                tenths_str(value, 5),
                format!("{:5.1}", value),
                "tenths_str({value})"
            );
            assert_eq!(
                tenths_str(value, 8),
                format!("{:8.1}", value),
                "tenths_str width({value})"
            );
        }
        assert!(checked > 10_000, "too few usable samples: {checked}");
    }

    #[test]
    fn round0_matches_format_at_boundaries() {
        let cases = [0.0f64, 0.5, 1.5, 2.5, 3.5, 0.49, 0.51, 99.5, 1023.5];
        for &value in &cases {
            assert_eq!(round0_str(value), format!("{:.0}", value), "round0({value})");
        }
    }

    #[test]
    fn round0_matches_format_on_random_samples() {
        let mut state = 0xDEADBEEF_CAFE_F00Du64;
        for _ in 0..40_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let value = f64::from_bits(state & 0x7FFF_FFFF_FFFF_FFFF) * 1e-9;
            if !value.is_finite() || value >= 1e12 {
                continue;
            }
            assert_eq!(round0_str(value), format!("{:.0}", value), "round0({value})");
        }
    }

    #[test]
    fn width_padding_matches() {
        assert_eq!(tenths_str(0.0, 5), "  0.0");
        assert_eq!(tenths_str(100.0, 5), "100.0");
        assert_eq!(tenths_str(12.5, 5), " 12.5");
        assert_eq!(tenths_str(1234.5, 5), "1234.5"); // wider than the column
    }

    #[test]
    fn exact_scaled_handles_dyadic_ties() {
        // 0.25 → tenths 2.5 → ties to even → 2 ("0.2")
        assert_eq!(exact_scaled(0.25, 10), 2);
        // 0.75 → tenths 7.5 → ties to even → 8 ("0.8")
        assert_eq!(exact_scaled(0.75, 10), 8);
    }
}

/// `(bytes / 2^divisor_pow2)` with one decimal ("13.4"), ties to even —
/// byte-identical to `format!("{:.1}", bytes as f64 / 2^d)`.
pub(crate) fn scaled_bytes(bytes: u64, divisor_pow2: u32) -> String {
    let tenths = round_half_even_div(u128::from(bytes) * 10, divisor_pow2);
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// [`scaled_bytes`] writing into a caller-owned (pooled) string.
pub(crate) fn scaled_bytes_into(buf: &mut String, bytes: u64, divisor_pow2: u32) {
    let tenths = round_half_even_div(u128::from(bytes) * 10, divisor_pow2);
    buf.clear();
    use std::fmt::Write as _;
    let _ = write!(buf, "{}.{}", tenths / 10, tenths % 10);
}

/// `(bytes / 2^divisor_pow2)` rounded to an integer — matches
/// `format!("{:.0}", bytes as f64 / 2^d)`.
pub(crate) fn scaled_bytes_round0(bytes: u64, divisor_pow2: u32) -> String {
    format!("{}", round_half_even_div(u128::from(bytes), divisor_pow2))
}

#[cfg(test)]
mod bytes_tests {
    use super::{scaled_bytes, scaled_bytes_round0};

    fn reference(bytes: u64, divisor_pow2: u32, decimals: usize) -> String {
        let divisor = f64::from(1u32 << divisor_pow2.min(31) as u32);
        // powers above 31 need f64 math; express divisor as f64 pow2
        let divisor = f64::powi(2.0, divisor_pow2 as i32);
        let _ = divisor; // silence unused when inlined below
        if decimals == 1 {
            format!("{:.1}", bytes as f64 / f64::powi(2.0, divisor_pow2 as i32))
        } else {
            format!("{:.0}", bytes as f64 / f64::powi(2.0, divisor_pow2 as i32))
        }
    }

    #[test]
    fn scaled_bytes_match_float_format() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut checked = 0usize;
        for &pow in &[10u32, 20, 30, 40] {
            // boundary values around each power of two
            for delta in [0u64, 1, 5, 51, 512, 5120, 52_428] {
                let base = 1u64 << pow;
                for bytes in [base.saturating_sub(delta), base + delta, base / 2 + delta] {
                    assert_eq!(scaled_bytes(bytes, pow), reference(bytes, pow, 1), "1dp {bytes}/{pow}");
                    assert_eq!(
                        scaled_bytes_round0(bytes, pow),
                        reference(bytes, pow, 0),
                        "0dp {bytes}/{pow}"
                    );
                    checked += 1;
                }
            }
        }
        // random samples — capped below 2^53 where `bytes as f64` stays exact;
        // beyond that the FLOAT path silently loses precision (unreachable for
        // real byte counts anyway: 2^53 K ≈ 9 PB).
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let bytes = state >> 11;
            let pow = (state >> 60) as u32 % 3 * 10 + 10;
            assert_eq!(scaled_bytes(bytes, pow), reference(bytes, pow, 1), "1dp {bytes}/{pow}");
            assert_eq!(
                scaled_bytes_round0(bytes, pow),
                reference(bytes, pow, 0),
                "0dp {bytes}/{pow}"
            );
            checked += 1;
        }
        assert!(checked > 20_000);
    }
}
