use image::{Rgba, RgbaImage};

pub(super) const FRAME_INTERVAL_MS: u64 = 40;
pub(super) const DURATION_MS: u64 = 720;
pub(super) const FRAME_COUNT: u8 = (DURATION_MS / FRAME_INTERVAL_MS) as u8;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct Frame(u8);

impl Frame {
    pub(super) const REST: Self = Self(0);

    pub(super) fn all() -> impl Iterator<Item = Self> {
        (0..FRAME_COUNT).map(Self)
    }

    pub(super) fn at_elapsed(elapsed_ms: u64) -> Self {
        if elapsed_ms >= DURATION_MS {
            Self::REST
        } else {
            Self((elapsed_ms / FRAME_INTERVAL_MS) as u8)
        }
    }
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn pulse(t: f64, start: f64, peak: f64, end: f64) -> f64 {
    if t < peak {
        smooth((t - start) / (peak - start))
    } else {
        1.0 - smooth((t - peak) / (end - peak))
    }
}

struct Pose {
    head: f64,
    body: f64,
    left_ear: f64,
    right_ear: f64,
    jaw: f64,
}

impl Pose {
    fn new(frame: Frame) -> Self {
        let t = f64::from(frame.0) / f64::from(FRAME_COUNT);
        Self {
            head: pulse(t, 0.05, 0.35, 0.97) - 0.08 * pulse(t, 0.76, 0.87, 1.0),
            body: pulse(t, 0.0, 0.31, 0.94) - 0.12 * pulse(t, 0.74, 0.86, 1.0),
            left_ear: pulse(t, 0.07, 0.23, 0.49) - 0.3 * pulse(t, 0.35, 0.48, 0.68),
            right_ear: pulse(t, 0.21, 0.39, 0.63) - 0.23 * pulse(t, 0.48, 0.61, 0.81),
            jaw: pulse(t, 0.27, 0.43, 0.69),
        }
    }

    fn displacement(&self, x: f64, y: f64) -> (f64, f64) {
        let upper = 1.0 - smooth((y - 13.0) / 15.0);
        let body_weight = 1.0 - smooth((y - 20.0) / 11.5);
        let angle = -0.09 * self.head;
        let mut dx =
            (-(y - 20.0) * angle - 0.10 * self.head) * upper - 0.50 * self.body * body_weight;
        let mut dy =
            ((x - 15.0) * angle + 0.72 * self.head) * upper - 0.24 * self.body * body_weight;
        let ear = 1.0 - smooth((y - 1.0) / 7.0);
        let left = 1.0 - smooth((x - 15.6) / 2.3);
        let ear_angle = -0.12 * self.left_ear * left + 0.12 * self.right_ear * (1.0 - left);
        dx += -(y - 7.5) * ear_angle * ear;
        dy += (x - (14.7 * left + 20.0 * (1.0 - left))) * ear_angle * ear
            + 0.12 * self.right_ear.abs() * ear;
        let jaw_weight = smooth((x - 20.0) / 8.0) * smooth((y - 17.6) / 2.8);
        dx += 0.20 * self.jaw * jaw_weight;
        dy += 0.78 * self.jaw * jaw_weight;

        // Pin the original S/T cutout, feathering into the moving outline in every mode.
        let glyph_distance = (10.5 - x).max(x - 22.5).max(15.5 - y).max(y - 28.5);
        let movable = smooth(glyph_distance / 2.0);
        (dx * movable, dy * movable)
    }
}

pub(super) fn render(source: &RgbaImage, frame: Frame) -> RgbaImage {
    if frame == Frame::REST {
        return source.clone();
    }
    let pose = Pose::new(frame);
    // Sample in the original 32px coordinate space: no padding, crop or global scaling.
    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        let (target_x, target_y) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let (mut sx, mut sy) = (target_x, target_y);
        for _ in 0..3 {
            let (dx, dy) = pose.displacement(sx, sy);
            sx = target_x - dx;
            sy = target_y - dy;
        }
        if sx == target_x && sy == target_y {
            return *source.get_pixel(x, y);
        }
        let (sx, sy) = (sx - 0.5, sy - 0.5);
        let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
        let (fx, fy) = (sx - sx.floor(), sy - sy.floor());
        let alpha = |x: i32, y: i32| {
            if x < 0 || y < 0 || x >= source.width() as i32 || y >= source.height() as i32 {
                0.0
            } else {
                f64::from(source.get_pixel(x as u32, y as u32)[3])
            }
        };
        let a = (alpha(ix, iy) * (1.0 - fx) + alpha(ix + 1, iy) * fx) * (1.0 - fy)
            + (alpha(ix, iy + 1) * (1.0 - fx) + alpha(ix + 1, iy + 1) * fx) * fy;
        // NSImage is templated; only alpha determines its light/dark appearance.
        Rgba([0, 0, 0, a.round() as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_skips_late_frames_and_finishes_on_the_original() {
        assert_eq!(Frame::at_elapsed(0), Frame::REST);
        assert_eq!(Frame::at_elapsed(39), Frame::REST);
        assert_eq!(Frame::at_elapsed(40), Frame(1));
        assert_eq!(Frame::at_elapsed(299), Frame(7));
        assert_eq!(Frame::at_elapsed(719), Frame(17));
        assert_eq!(Frame::at_elapsed(720), Frame::REST);
        assert_eq!(Frame::at_elapsed(u64::MAX), Frame::REST);
        assert_eq!(Frame::all().count(), usize::from(FRAME_COUNT));
    }

    #[test]
    fn opaque_outline_stays_inside_the_original_canvas() {
        let source =
            image::load_from_memory(include_bytes!("../../../../assets/icons/tray-logo.png"))
                .unwrap()
                .into_rgba8();
        for frame in Frame::all() {
            let pose = Pose::new(frame);
            for (x, y, pixel) in source.enumerate_pixels().filter(|(_, _, p)| p[3] >= 128) {
                let (x, y) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (dx, dy) = pose.displacement(x, y);
                assert!(
                    (0.0..32.0).contains(&(x + dx)),
                    "{frame:?}: {x}, {y}, {pixel:?}"
                );
                assert!(
                    (0.0..32.0).contains(&(y + dy)),
                    "{frame:?}: {x}, {y}, {pixel:?}"
                );
            }
        }
    }
}
