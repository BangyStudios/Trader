pub fn interpolate_linear(timeseries_source: &[f64], len_target: usize) -> Vec<f64> {
    let len_source = timeseries_source.len();
    assert!(
        len_source >= 2,
        "Input vector must have at least 2 elements"
    );
    assert!(len_target >= 2, "Output length must be at least 2");

    // Trivial case
    if len_target == len_source {
        return timeseries_source.to_vec();
    }

    let scale = (len_source - 1) as f64 / (len_target - 1) as f64;

    let result = (0..len_target)
        .map(|i| {
            let x = i as f64 * scale;
            let index = x.floor() as usize;
            let t = x - index as f64;

            if index + 1 < len_source {
                timeseries_source[index] * (1.0 - t) + timeseries_source[index + 1] * t
            } else {
                timeseries_source[len_source - 1]
            }
        })
        .collect();

    return result;
}
