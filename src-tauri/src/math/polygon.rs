pub type Point = [f64; 2];

const SMALL_POLYGON_CAPACITY: usize = 12;
const COLLINEAR_TOLERANCE: f64 = 1e-12;

fn cross(origin: Point, a: Point, b: Point) -> f64 {
    (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
}

fn length(a: Point, b: Point) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn turns_left(origin: Point, a: Point, b: Point) -> bool {
    cross(origin, a, b) > COLLINEAR_TOLERANCE * length(origin, a) * length(origin, b)
}

pub fn signed_area(polygon: &[Point]) -> f64 {
    let n = polygon.len();
    if n < 3 {
        return 0.0;
    }
    0.5 * (0..n).map(|i| {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>()
}

pub fn area(polygon: &[Point]) -> f64 {
    signed_area(polygon).abs()
}

pub fn convex_hull(points: &[Point]) -> Vec<Point> {
    let mut sorted: Vec<Point> = points.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let mut hull: Vec<Point> = Vec::with_capacity(2 * sorted.len());
    for pass in [sorted.clone(), sorted.iter().rev().copied().collect()] {
        let floor = hull.len();
        for point in pass {
            while hull.len() >= floor + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
                hull.pop();
            }
            hull.push(point);
        }
        hull.pop();
    }
    without_collinear(hull)
}

fn without_collinear(mut polygon: Vec<Point>) -> Vec<Point> {
    loop {
        let n = polygon.len();
        if n < 3 {
            return polygon;
        }
        let kept: Vec<Point> =
            (0..n).filter(|&i| turns_left(polygon[(i + n - 1) % n], polygon[i], polygon[(i + 1) % n])).map(|i| polygon[i]).collect();
        if kept.len() == n {
            return polygon;
        }
        polygon = kept;
    }
}

fn edge_crossing(p: Point, q: Point, a: Point, b: Point) -> Point {
    let (dp, dq) = (cross(a, b, p), cross(a, b, q));
    let t = dp / (dp - dq);
    [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]
}

pub fn clip_convex(subject: &[Point], clip: &[Point]) -> Vec<Point> {
    let mut output = subject.to_vec();
    for i in 0..clip.len() {
        if output.is_empty() {
            break;
        }
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let input = std::mem::take(&mut output);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (p_in, q_in) = (cross(a, b, p) >= 0.0, cross(a, b, q) >= 0.0);
            if p_in {
                output.push(p);
            }
            if p_in != q_in {
                output.push(edge_crossing(p, q, a, b));
            }
        }
    }
    output
}

pub fn contains(convex: &[Point], point: Point) -> bool {
    let n = convex.len();
    n >= 3
        && (0..n).all(|i| {
            let (a, b) = (convex[i], convex[(i + 1) % n]);
            cross(a, b, point) >= -COLLINEAR_TOLERANCE * length(a, b) * length(a, point).max(1.0)
        })
}

#[derive(Debug, Clone, Copy)]
pub struct SmallPolygon {
    points: [Point; SMALL_POLYGON_CAPACITY],
    len: usize,
}

impl SmallPolygon {
    pub fn new(points: &[Point]) -> Self {
        let mut polygon = Self { points: [[0.0; 2]; SMALL_POLYGON_CAPACITY], len: points.len().min(SMALL_POLYGON_CAPACITY) };
        polygon.points[..polygon.len].copy_from_slice(&points[..polygon.len]);
        polygon
    }

    fn clip_half(&self, axis: usize, bound: f64, keep_above: bool) -> Self {
        let mut out = Self { points: [[0.0; 2]; SMALL_POLYGON_CAPACITY], len: 0 };
        let inside = |p: Point| if keep_above { p[axis] >= bound } else { p[axis] <= bound };
        for j in 0..self.len {
            let (p, q) = (self.points[j], self.points[(j + 1) % self.len]);
            if inside(p) && out.len < SMALL_POLYGON_CAPACITY {
                out.points[out.len] = p;
                out.len += 1;
            }
            if inside(p) != inside(q) && out.len < SMALL_POLYGON_CAPACITY {
                let t = (bound - p[axis]) / (q[axis] - p[axis]);
                let mut crossing = [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])];
                crossing[axis] = bound;
                out.points[out.len] = crossing;
                out.len += 1;
            }
        }
        out
    }

    pub fn clip_band(&self, axis: usize, low: f64, high: f64) -> Self {
        self.clip_half(axis, low, true).clip_half(axis, high, false)
    }

    pub fn area(&self) -> f64 {
        area(&self.points[..self.len])
    }

    pub fn is_empty(&self) -> bool {
        self.len < 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, side: f64) -> Vec<Point> {
        vec![[x0, y0], [x0 + side, y0], [x0 + side, y0 + side], [x0, y0 + side]]
    }

    fn rotated_square(side: f64, angle_deg: f64) -> Vec<Point> {
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let h = side / 2.0;
        [[-h, -h], [h, -h], [h, h], [-h, h]].iter().map(|[x, y]| [x * cos - y * sin, x * sin + y * cos]).collect()
    }

    #[test]
    fn clipping_two_convex_polygons_gives_their_overlap() {
        let a = square(0.0, 0.0, 4.0);
        let b = square(1.0, 2.0, 4.0);
        let overlap = clip_convex(&a, &b);
        assert!((area(&overlap) - 6.0).abs() < 1e-12, "{overlap:?}");
        assert!(signed_area(&a) > 0.0 && signed_area(&a.iter().rev().copied().collect::<Vec<_>>()) < 0.0);
        assert!(clip_convex(&a, &square(10.0, 10.0, 1.0)).len() < 3);

        let octagon = clip_convex(&rotated_square(2.0, 0.0), &rotated_square(2.0, 45.0));
        assert!((area(&octagon) - 4.0 * (2.0 * 2f64.sqrt() - 2.0)).abs() < 1e-12, "{}", area(&octagon));
        assert_eq!(convex_hull(&octagon).len(), 8);
        assert!(contains(&convex_hull(&octagon), [0.99, 0.0]));
        assert!(!contains(&convex_hull(&octagon), [0.99, 0.99]));
    }

    #[test]
    fn the_hull_is_counter_clockwise_without_interior_or_collinear_points() {
        let points = [[0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [2.0, 2.0], [0.0, 2.0], [1.0, 1.0], [0.0, 1.0], [2.0, 1.0 + 1e-15]];
        let hull = convex_hull(&points);
        assert_eq!(hull.len(), 4, "{hull:?}");
        assert!(signed_area(&hull) > 0.0);
        assert!((area(&hull) - 4.0).abs() < 1e-12);
        for corner in [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]] {
            assert!(hull.contains(&corner), "{hull:?}");
        }
        assert!(contains(&hull, [2.0, 1.0]) && contains(&hull, [0.0, 0.0]));
        assert!(!contains(&hull, [2.0 + 1e-9, 1.0]));
        assert!(convex_hull(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]).len() < 3);
    }

    #[test]
    fn the_hull_keeps_corners_when_edge_points_tie_in_x() {
        let mut points = Vec::new();
        for i in 0..=16 {
            let y = -3.2 + 0.4 * i as f64;
            points.push([5.0 + ((i * 7919) % 13) as f64 * 1e-15, y]);
            points.push([-1.4 - ((i * 104_729) % 11) as f64 * 1e-15, y]);
        }
        for i in 1..16 {
            let x = -1.4 + 0.4 * i as f64;
            points.push([x, 3.2 + ((i * 31) % 7) as f64 * 1e-15]);
            points.push([x, -3.2 - ((i * 17) % 5) as f64 * 1e-15]);
        }
        let hull = convex_hull(&points);
        assert!((area(&hull) - 6.4 * 6.4).abs() < 1e-9, "{} from {hull:?}", area(&hull));
        assert_eq!(hull.len(), 4, "{hull:?}");
        for corner in [[5.0, 3.2], [5.0, -3.2], [-1.4, 3.2], [-1.4, -3.2]] {
            assert!(hull.iter().any(|p| (p[0] - corner[0]).abs() < 1e-12 && (p[1] - corner[1]).abs() < 1e-12), "{corner:?} missing from {hull:?}");
        }
    }

    #[test]
    fn a_small_polygon_clipped_to_bands_keeps_the_overlap_area() {
        let quad = SmallPolygon::new(&rotated_square(2.0, 30.0));
        assert!((quad.area() - 4.0).abs() < 1e-12);
        let mut total = 0.0;
        for row in -2..2 {
            let band = quad.clip_band(1, row as f64, row as f64 + 1.0);
            for col in -2..2 {
                total += band.clip_band(0, col as f64, col as f64 + 1.0).area();
            }
        }
        assert!((total - 4.0).abs() < 1e-12, "{total}");
        let half = quad.clip_band(0, 0.0, 10.0);
        assert!((half.area() - 2.0).abs() < 1e-12, "{}", half.area());
        assert!(quad.clip_band(0, 5.0, 6.0).is_empty());
        let reversed: Vec<Point> = rotated_square(2.0, 30.0).into_iter().rev().collect();
        assert!((SmallPolygon::new(&reversed).clip_band(1, 0.0, 1.0).area() - quad.clip_band(1, 0.0, 1.0).area()).abs() < 1e-12);
    }
}
