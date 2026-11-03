/// Median of `values`. Sorts the slice in place; an empty slice gives 0.
pub(crate) fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let len = values.len();
    (values[(len - 1) / 2] + values[len / 2]) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_length_takes_the_middle_value() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
    }

    #[test]
    fn even_length_averages_the_middle_pair() {
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn empty_slice_is_zero() {
        assert_eq!(median(&mut []), 0.0);
    }
}
