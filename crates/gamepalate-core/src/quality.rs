use crate::game::Reviews;

pub fn quality(reviews: &Reviews) -> f64 {
    const Z: f64 = 1.96;

    let n: f64 = reviews.total.into();

    if n == 0.0 {
        return 0.0;
    }

    let p: f64 = f64::from(reviews.positive) / n;

    let z2 = Z.powi(2);
    let center = p + z2 / (2.0 * n);
    let margin = Z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();

    (center - margin) / (1.0 + z2 / n)
}
