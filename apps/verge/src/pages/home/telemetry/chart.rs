use gpui_kit::{Bounds, PathBuilder, Pixels, point, px};

pub(super) struct Axis {
    pub maximum: f64,
    pub unit: &'static str,
    divisor: f64,
}

impl Axis {
    pub fn new(peak: u64) -> Self {
        let peak = (peak as f64).max(1.);
        let (unit, divisor) = if peak >= 1024_f64.powi(3) {
            ("GB/s", 1024_f64.powi(3))
        } else if peak >= 1024_f64.powi(2) {
            ("MB/s", 1024_f64.powi(2))
        } else if peak >= 1024. {
            ("KB/s", 1024.)
        } else {
            ("B/s", 1.)
        };
        let target = peak / divisor * 1.1;
        let magnitude = 10_f64.powf(target.log10().floor());
        let step = [1., 2., 5., 10.]
            .into_iter()
            .find(|step| step * magnitude >= target)
            .unwrap_or(10.);
        Self {
            maximum: step * magnitude * divisor,
            unit,
            divisor,
        }
    }

    pub fn label(&self, fraction: f64) -> String {
        let value = self.maximum / self.divisor * fraction;
        if value.fract().abs() < 0.01 {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        }
    }
}

// Shape-preserving Hermite tangents: retain every sample and flatten at extrema.
// Unlike averaging or unconstrained splines, peaks are neither erased nor invented.
fn segments(values: &[f64]) -> Vec<[f64; 4]> {
    if values.len() < 2 {
        return Vec::new();
    }
    let slopes: Vec<_> = values.windows(2).map(|v| v[1] - v[0]).collect();
    let mut tangents = vec![0.; values.len()];
    tangents[0] = slopes[0];
    tangents[values.len() - 1] = slopes[slopes.len() - 1];
    for i in 1..values.len() - 1 {
        let (left, right) = (slopes[i - 1], slopes[i]);
        if left * right > 0. {
            let low = left.abs().min(right.abs());
            let high = left.abs().max(right.abs());
            tangents[i] = left.signum() * 2. * low / (1. + low / high);
        }
    }
    values
        .windows(2)
        .enumerate()
        .map(|(i, pair)| {
            [
                pair[0],
                pair[0] + tangents[i] / 3.,
                pair[1] - tangents[i + 1] / 3.,
                pair[1],
            ]
        })
        .collect()
}

pub(super) fn curve(
    path: &mut PathBuilder,
    values: &[f64],
    bounds: Bounds<Pixels>,
    maximum: f64,
    capacity: usize,
    fill: bool,
) {
    if values.len() < 2 {
        return;
    }
    let step = bounds.size.width / (capacity - 1) as f32;
    let first_x = bounds.left() + step * (capacity - values.len()) as f32;
    let y = |value: f64| bounds.bottom() - bounds.size.height * (value / maximum) as f32;
    path.move_to(point(first_x, y(values[0])));
    for (i, controls) in segments(values).into_iter().enumerate() {
        let x = first_x + step * i as f32;
        path.cubic_bezier_to(
            point(x + step, y(controls[3])),
            point(x + step / 3., y(controls[1])),
            point(x + step * (2. / 3.), y(controls[2])),
        );
    }
    if fill {
        path.line_to(point(bounds.right(), bounds.bottom()));
        path.line_to(point(first_x, bounds.bottom()));
        path.close();
    }
}

pub(super) fn plot_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        bounds.origin + point(px(1.), px(2.)),
        bounds.size - gpui_kit::size(px(2.), px(4.)),
    )
}

#[cfg(test)]
mod tests {
    use super::{Axis, segments};

    #[test]
    fn smooth_curve_preserves_samples_and_never_overshoots() {
        for values in [
            vec![],
            vec![0.],
            vec![0., 0.],
            vec![0., 1000., 0., 3., 3., 20.],
            vec![1., 2., 900., 901., 3., 0.],
            vec![u64::MAX as f64, 0., u64::MAX as f64],
        ] {
            let curves = segments(&values);
            assert_eq!(curves.len(), values.len().saturating_sub(1));
            for (i, [a, b, c, d]) in curves.into_iter().enumerate() {
                assert_eq!(a, values[i]);
                assert_eq!(d, values[i + 1]);
                for j in 0..=100 {
                    let t = f64::from(j) / 100.;
                    let u = 1. - t;
                    let y =
                        u.powi(3) * a + 3. * u.powi(2) * t * b + 3. * u * t * t * c + t.powi(3) * d;
                    assert!(y >= a.min(d) - 1e-6 && y <= a.max(d) * (1. + 1e-14) + 1e-6);
                }
            }
        }
    }

    #[test]
    fn axis_has_headroom_and_consistent_units() {
        for peak in [0, 1, 999, 1024, 42_500, 1024 * 1024, u64::MAX] {
            let axis = Axis::new(peak);
            assert!(axis.maximum > peak as f64);
            assert_eq!(axis.label(0.), "0");
        }
        let axis = Axis::new(42_500);
        assert_eq!(axis.unit, "KB/s");
        assert_eq!(axis.label(1.), "50");
        assert_eq!(axis.label(0.5), "25");
    }
}
