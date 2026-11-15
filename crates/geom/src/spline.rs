//! The natural cubic spline of A27's profile (specification A27). Owner: WP1.
//!
//! The profile η is the natural cubic spline through 12 knots at t_k = (1 − cos(πk/11))/2, held
//! at 0 at both ends, so 10 of the knot values are free. The knots are fixed, which makes the
//! spline a *linear* function of the free values: the second derivatives at the knots are
//! M = A·y, and η(t) and η'(t) are linear in y as well. The bending penalty is then the exact
//! quadratic form yᵀ Q y, with no numerical integration and no finite differences.

use crate::solver::Cholesky;

/// The number of knots.
pub const KNOTS: usize = 12;
/// The number of free knot values: the two end knots are held at 0.
pub const FREE: usize = 10;

/// The natural cubic spline through the 12 knots, as a linear function of the 10 free values.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The knot positions.
    knots: [f64; KNOTS],
    /// The knot spacings.
    h: [f64; KNOTS - 1],
    /// The second derivative at each knot, linear in the free values: M = A·y.
    a: [[f64; FREE]; KNOTS],
    /// The bending quadratic form: ∫₀¹ η''² dt = yᵀ Q y.
    q: [[f64; FREE]; FREE],
}

impl Default for Profile {
    fn default() -> Self {
        Profile::new()
    }
}

impl Profile {
    /// The spline over the 12 fixed knots.
    pub fn new() -> Profile {
        let mut knots = [0.0; KNOTS];
        for (k, t) in knots.iter_mut().enumerate() {
            let angle = std::f64::consts::PI * k as f64 / (KNOTS - 1) as f64;
            *t = (1.0 - angle.cos()) / 2.0;
        }
        let mut h = [0.0; KNOTS - 1];
        for k in 0..KNOTS - 1 {
            h[k] = knots[k + 1] - knots[k];
        }
        let (a, q) = Profile::basis_and_bending(&knots, &h);
        Profile { knots, h, a, q }
    }

    /// The knot positions.
    pub fn knots(&self) -> &[f64; KNOTS] {
        &self.knots
    }

    /// The second derivatives at the knots: M = A·y.
    pub fn second_derivatives(&self, y: &[f64; FREE]) -> [f64; KNOTS] {
        let mut m = [0.0; KNOTS];
        for (k, row) in self.a.iter().enumerate() {
            let mut sum = 0.0;
            for (j, value) in y.iter().enumerate() {
                sum += row[j] * value;
            }
            m[k] = sum;
        }
        m
    }

    /// The basis functions at `t`: η(t) = Σ_j basis[j]·y[j].
    pub fn basis(&self, t: f64) -> [f64; FREE] {
        let (k, d, e, h) = self.interval(t);
        let mut basis = [0.0; FREE];
        let (row_k, row_next) = (&self.a[k], &self.a[k + 1]);
        for j in 0..FREE {
            basis[j] = row_k[j] * (e * e * e / (6.0 * h)) + row_next[j] * (d * d * d / (6.0 * h));
            basis[j] += (self.knot_share(k, j) - row_k[j] * h * h / 6.0) * e / h;
            basis[j] += (self.knot_share(k + 1, j) - row_next[j] * h * h / 6.0) * d / h;
        }
        basis
    }

    /// The derivatives of the basis functions at `t`: η'(t) = Σ_j basis'[j]·y[j].
    pub fn derivative_basis(&self, t: f64) -> [f64; FREE] {
        let (k, d, e, h) = self.interval(t);
        let mut basis = [0.0; FREE];
        let (row_k, row_next) = (&self.a[k], &self.a[k + 1]);
        for j in 0..FREE {
            basis[j] = row_k[j] * (-e * e / (2.0 * h)) + row_next[j] * (d * d / (2.0 * h));
            basis[j] -= (self.knot_share(k, j) - row_k[j] * h * h / 6.0) / h;
            basis[j] += (self.knot_share(k + 1, j) - row_next[j] * h * h / 6.0) / h;
        }
        basis
    }

    /// The profile's height at `t`.
    pub fn value(&self, y: &[f64; FREE], t: f64) -> f64 {
        let basis = self.basis(t);
        let mut sum = 0.0;
        for j in 0..FREE {
            sum += basis[j] * y[j];
        }
        sum
    }

    /// The profile's slope at `t`.
    pub fn derivative(&self, y: &[f64; FREE], t: f64) -> f64 {
        let basis = self.derivative_basis(t);
        let mut sum = 0.0;
        for j in 0..FREE {
            sum += basis[j] * y[j];
        }
        sum
    }

    /// The bending quadratic form Q, with ∫₀¹ η''² dt = yᵀ Q y.
    pub fn bending_quadratic(&self) -> &[[f64; FREE]; FREE] {
        &self.q
    }

    /// The bending integral ∫₀¹ η''² dt of the profile `y`.
    pub fn bending_integral(&self, y: &[f64; FREE]) -> f64 {
        let mut sum = 0.0;
        for i in 0..FREE {
            for j in 0..FREE {
                sum += y[i] * self.q[i][j] * y[j];
            }
        }
        sum
    }

    /// The interval `k` holding `t`, the distances to its two knots and its spacing.
    fn interval(&self, t: f64) -> (usize, f64, f64, f64) {
        let t = t.clamp(0.0, 1.0);
        let mut k = 0;
        while k + 2 < KNOTS && self.knots[k + 1] < t {
            k += 1;
        }
        let h = self.h[k];
        let d = (t - self.knots[k]).clamp(0.0, h);
        (k, d, h - d, h)
    }

    /// 1 when the free value `j` is the value of knot `k`, 0 otherwise.
    fn knot_share(&self, k: usize, j: usize) -> f64 {
        if (1..=FREE).contains(&k) && j + 1 == k { 1.0 } else { 0.0 }
    }

    /// The matrix A with M = A·y, and the bending quadratic form Q.
    fn basis_and_bending(knots: &[f64; KNOTS], h: &[f64; KNOTS - 1]) -> ([[f64; FREE]; KNOTS], [[f64; FREE]; FREE]) {
        // The tridiagonal system for the interior second derivatives, and its right-hand side as
        // a linear function of the free values.
        let mut t = vec![0.0; FREE * FREE];
        let mut rhs = vec![0.0; FREE * FREE];
        for k in 1..=FREE {
            let row = k - 1;
            let diagonal = 2.0 * (h[k - 1] + h[k]);
            t[row * FREE + row] = diagonal;
            if row > 0 {
                t[row * FREE + row - 1] = h[k - 1];
            }
            if row + 1 < FREE {
                t[row * FREE + row + 1] = h[k];
            }
            // r_k = 6·[(y_{k+1} − y_k)/h_k − (y_k − y_{k-1})/h_{k-1}].
            let mut add = |j: usize, v: f64| {
                if let Some(slot) = rhs.get_mut(row * FREE + j) {
                    *slot += v;
                }
            };
            if k < FREE {
                add(k, 6.0 / h[k]);
            }
            add(k - 1, -6.0 / h[k] - 6.0 / h[k - 1]);
            if k >= 2 {
                add(k - 2, 6.0 / h[k - 1]);
            }
        }
        let factor = Cholesky::factor(&t, FREE);
        let mut a = [[0.0; FREE]; KNOTS];
        if let Some(factor) = factor {
            for column in 0..FREE {
                let b: Vec<f64> = (0..FREE).map(|row| rhs[row * FREE + column]).collect();
                if let Some(solution) = factor.solve(&b) {
                    for (row, value) in solution.iter().enumerate() {
                        a[row + 1][column] = *value;
                    }
                }
            }
        }
        // The bending quadratic form: Q = Aᵀ Q_knot A.
        let mut q_knot = vec![0.0; KNOTS * KNOTS];
        for (k, spacing) in h.iter().enumerate() {
            let w = spacing / 3.0;
            for (p, q) in [(k, k), (k, k + 1), (k + 1, k + 1)] {
                let index = p * KNOTS + q;
                if let Some(slot) = q_knot.get_mut(index) {
                    *slot += w;
                }
            }
        }
        let mut q = [[0.0; FREE]; FREE];
        for i in 0..FREE {
            for j in 0..FREE {
                let mut sum = 0.0;
                for p in 0..KNOTS {
                    let ap = a[p][i];
                    if ap == 0.0 {
                        continue;
                    }
                    for r in 0..KNOTS {
                        let qr = q_knot[p * KNOTS + r];
                        if qr == 0.0 {
                            continue;
                        }
                        sum += ap * qr * a[r][j];
                    }
                }
                q[i][j] = sum;
            }
        }
        let _ = knots;
        (a, q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spline_is_natural_and_interpolates() {
        let profile = Profile::new();
        let y = [0.1, -0.2, 0.3, 0.05, -0.15, 0.22, -0.08, 0.12, -0.04, 0.06];
        for (k, t) in profile.knots().iter().enumerate() {
            let value = profile.value(&y, *t);
            let expected = if k == 0 || k == KNOTS - 1 { 0.0 } else { y[k - 1] };
            assert!((value - expected).abs() < 1e-12, "knot {k}: {value} against {expected}");
        }
        let m = profile.second_derivatives(&y);
        assert!(m[0].abs() < 1e-12);
        assert!(m[KNOTS - 1].abs() < 1e-12);
        assert!(profile.value(&y, 0.0).abs() < 1e-12);
        assert!(profile.value(&y, 1.0).abs() < 1e-12);
        // The knots crowd toward the ends.
        let knots = profile.knots();
        assert!((knots[1] - knots[0]) < (knots[6] - knots[5]));
    }

    #[test]
    fn the_spline_is_linear_in_the_free_values() {
        let profile = Profile::new();
        let a = [0.1, -0.2, 0.3, 0.05, -0.15, 0.22, -0.08, 0.12, -0.04, 0.06];
        let b = [-0.3, 0.1, 0.05, -0.22, 0.4, 0.01, -0.09, 0.14, 0.07, -0.02];
        let sum = [0.0; FREE];
        for t in [0.0, 0.1, 0.37, 0.5, 0.82, 1.0] {
            let sum_value = profile.value(&a, t) + profile.value(&b, t);
            let sum_slope = profile.derivative(&a, t) + profile.derivative(&b, t);
            let mut both = sum;
            for j in 0..FREE {
                both[j] = a[j] + b[j];
            }
            assert!((profile.value(&both, t) - sum_value).abs() < 1e-12);
            assert!((profile.derivative(&both, t) - sum_slope).abs() < 1e-12);
        }
    }

    #[test]
    fn the_bending_integral_is_exact_for_a_cubic() {
        let profile = Profile::new();
        // A cubic η(t) = a t³ + b t² + c t + d, whose second derivative is 6a·t + 2b.
        let (a, b, c, d) = (0.7f64, -0.4f64, 0.2f64, 0.1f64);
        let cubic = |t: f64| a * t * t * t + b * t * t + c * t + d;
        let second = |t: f64| 6.0 * a * t + 2.0 * b;
        // The closed form of the integral of (6a·t + 2b)² over an interval, from its antiderivative.
        let antiderivative = |t: f64| 12.0 * a * a * t * t * t + 12.0 * a * b * t * t + 4.0 * b * b * t;
        for k in 0..KNOTS - 1 {
            let (t0, t1) = (profile.knots()[k], profile.knots()[k + 1]);
            let h = t1 - t0;
            let alpha = second(t0);
            let beta = second(t1) - second(t0);
            let closed = h * (alpha * alpha + alpha * beta + beta * beta / 3.0);
            let exact = antiderivative(t1) - antiderivative(t0);
            assert!((closed - exact).abs() < 1e-9, "interval {k}: {closed} against {exact}");
        }
        // The spline through a cubic's knot values is not the cubic itself — the natural end
        // conditions hold η'' at 0 — so the check is that the quadratic form gives the spline's
        // own integral exactly, against the same per-interval formula.
        let mut y = [0.0; FREE];
        for (j, slot) in y.iter_mut().enumerate() {
            *slot = cubic(profile.knots()[j + 1]);
        }
        let m = profile.second_derivatives(&y);
        let mut exact = 0.0;
        for k in 0..KNOTS - 1 {
            let h = profile.knots()[k + 1] - profile.knots()[k];
            exact += h * (m[k] * m[k] + m[k] * m[k + 1] + m[k + 1] * m[k + 1]) / 3.0;
        }
        assert!((profile.bending_integral(&y) - exact).abs() < 1e-9);
        // The quadratic form gives the same answer as the integral.
        let mut form = 0.0;
        for i in 0..FREE {
            for j in 0..FREE {
                form += y[i] * profile.bending_quadratic()[i][j] * y[j];
            }
        }
        assert!((form - exact).abs() < 1e-9);
    }

    #[test]
    fn the_spline_slope_matches_a_finite_difference() {
        let profile = Profile::new();
        let y = [0.1, -0.2, 0.3, 0.05, -0.15, 0.22, -0.08, 0.12, -0.04, 0.06];
        let h = 1e-6;
        for t in [0.05, 0.2, 0.5, 0.77, 0.95] {
            let numerical = (profile.value(&y, t + h) - profile.value(&y, t - h)) / (2.0 * h);
            assert!((profile.derivative(&y, t) - numerical).abs() < 1e-6, "at {t}");
        }
    }
}
