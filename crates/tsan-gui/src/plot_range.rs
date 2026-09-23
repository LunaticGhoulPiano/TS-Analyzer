/// Fit GOP charts to their variation, not to a percentage of the absolute byte count.
/// A large baseline with a small spread must still produce a readable curve.
pub(crate) fn gop_axis_range(low: f64, high: f64) -> (f64, f64) {
    if !low.is_finite() || !high.is_finite() || high < low {
        return (0.0, 1.0);
    }
    let padding = if high > low {
        (high - low) * 0.10
    } else {
        // Constant GOP lengths/byte counts still need a nonzero axis range.
        high.abs().mul_add(0.02, 0.0).max(1.0)
    };
    let bottom = if low >= 0.0 {
        (low - padding).max(0.0)
    } else {
        low - padding
    };
    (bottom, high + padding)
}

#[cfg(test)]
mod tests {
    use super::gop_axis_range;

    #[test]
    fn small_variation_on_large_byte_count_occupies_the_chart() {
        let data = (589_380.0, 590_320.0);
        let (low, high) = gop_axis_range(data.0, data.1);
        assert!(low < data.0 && high > data.1);
        let occupied = (data.1 - data.0) / (high - low);
        assert!(occupied > 0.8 && occupied < 0.9);
        let kib = gop_axis_range(data.0 / 1024.0, data.1 / 1024.0);
        assert!((kib.0 * 1024.0 - low).abs() < 0.001);
        assert!((kib.1 * 1024.0 - high).abs() < 0.001);
    }

    #[test]
    fn constant_and_empty_gop_ranges_remain_finite() {
        for value in [0.0, 16.0, 60.0, 589_568.0] {
            let (low, high) = gop_axis_range(value, value);
            assert!(low.is_finite() && high.is_finite() && high > low);
            assert!(low <= value && high > value && low >= 0.0);
        }
        assert_eq!(gop_axis_range(f64::INFINITY, f64::NEG_INFINITY), (0.0, 1.0));
    }
}
