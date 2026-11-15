//! The Levenberg–Marquardt solver of A27 and the small dense matrix routines it needs
//! (specification A27). Owner: WP1.
//!
//! Everything here is pure arithmetic on `f64`: no allocation beyond the caller's, no randomness
//! and no threading, so the results do not depend on the number of threads (§7.6).

use crate::GeomError;

/// The Cauchy loss ρ(s) = 4·ln(1 + s/4), with its scale at 2σ (A27's cost).
pub fn cauchy_loss(s: f64) -> f64 {
    if !s.is_finite() || s < 0.0 {
        return f64::INFINITY;
    }
    4.0 * (1.0 + s / 4.0).ln()
}

/// The weight of the Cauchy loss for a residual of squared length `e2`: w = 1/(1 + |e|²/4).
pub fn cauchy_weight(e2: f64) -> f64 {
    if !e2.is_finite() || e2 < 0.0 {
        return 1.0;
    }
    1.0 / (1.0 + e2 / 4.0)
}

/// The rotation matrix of the rotation vector `w`, by Rodrigues' formula:
/// exp([w]×) = I + sinθ/θ·[w]× + (1 − cosθ)/θ²·[w]×², with θ = |w|.
pub fn rodrigues(w: [f64; 3]) -> [[f64; 3]; 3] {
    let theta = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    if !theta.is_finite() || theta < 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (s, c) = (theta.sin(), theta.cos());
    let k = [[0.0, -w[2], w[1]], [w[2], 0.0, -w[0]], [-w[1], w[0], 0.0]];
    let kk = mat_mul(&k, &k);
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let identity = if i == j { 1.0 } else { 0.0 };
            out[i][j] = identity + s / theta * k[i][j] + (1.0 - c) / (theta * theta) * kk[i][j];
        }
    }
    out
}

/// The product of two 3 × 3 matrices.
pub fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut sum = 0.0;
            for k in 0..3 {
                sum += a[i][k] * b[k][j];
            }
            out[i][j] = sum;
        }
    }
    out
}

/// The product of a 3 × 3 matrix and a column vector.
pub fn mat_vec(a: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for i in 0..3 {
        let mut sum = 0.0;
        for k in 0..3 {
            sum += a[i][k] * v[k];
        }
        out[i] = sum;
    }
    out
}

/// The Cholesky factorisation of a symmetric positive-definite matrix, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Cholesky {
    n: usize,
    /// The lower triangle.
    lower: Vec<f64>,
}

impl Cholesky {
    /// Factors the symmetric matrix `a`, given as `n × n` row-major values. `None` when the
    /// matrix is not positive definite.
    pub fn factor(a: &[f64], n: usize) -> Option<Cholesky> {
        let mut lower = vec![0.0; n * n];
        for j in 0..n {
            let mut sum = a.get(j * n + j).copied().unwrap_or(0.0);
            for k in 0..j {
                let v = lower[j * n + k];
                sum -= v * v;
            }
            if !sum.is_finite() || sum <= 0.0 {
                return None;
            }
            let d = sum.sqrt();
            lower[j * n + j] = d;
            for i in (j + 1)..n {
                let mut s = a.get(i * n + j).copied().unwrap_or(0.0);
                for k in 0..j {
                    s -= lower[i * n + k] * lower[j * n + k];
                }
                lower[i * n + j] = s / d;
            }
        }
        Some(Cholesky { n, lower })
    }

    /// Solves A x = b. `None` when `b` does not have `n` values.
    pub fn solve(&self, b: &[f64]) -> Option<Vec<f64>> {
        if b.len() != self.n {
            return None;
        }
        let n = self.n;
        let mut y = vec![0.0; n];
        for i in 0..n {
            let mut s = b.get(i).copied().unwrap_or(0.0);
            for (k, yk) in y.iter().take(i).enumerate() {
                s -= self.lower[i * n + k] * yk;
            }
            y[i] = s / self.lower[i * n + i];
        }
        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let mut s = y[i];
            for (k, xk) in x.iter().enumerate().skip(i + 1) {
                s -= self.lower[k * n + i] * xk;
            }
            x[i] = s / self.lower[i * n + i];
        }
        Some(x)
    }
}

/// One point's contribution to the fit: its residual, its Cauchy weight and its Jacobian.
#[derive(Debug, Clone, PartialEq)]
pub struct LmRow {
    /// The residual, (projected − observed) / σ, in photo pixels.
    pub e: [f64; 2],
    /// The Cauchy weight of the residual.
    pub w: f64,
    /// The shared unknowns this row touches: the column and the residual's derivatives by it.
    pub columns: Vec<(usize, f64, f64)>,
    /// The residual's derivative by the row's own unknown.
    pub a: [f64; 2],
}

/// A quadratic term added to the normal equations: the bending penalty and the focal prior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtraTerm {
    /// The shared unknown the term's gradient belongs to.
    pub row: usize,
    /// The shared unknown the term's second derivative belongs to.
    pub column: usize,
    /// The term's second derivative.
    pub second: f64,
    /// The term's first derivative.
    pub first: f64,
}

/// The damped normal equations of one Levenberg–Marquardt step, with the per-point unknowns
/// eliminated by a Schur complement (A27's solver, step 2).
#[derive(Debug, Clone, PartialEq)]
pub struct LmSystem {
    /// The number of shared unknowns.
    pub shared: usize,
    /// The rows, in a fixed order.
    pub rows: Vec<LmRow>,
    /// The extra quadratic terms.
    pub extra: Vec<ExtraTerm>,
    /// Nielsen's damping μ.
    pub damping: f64,
}

impl LmSystem {
    /// The Schur-complemented normal matrix and the right-hand side, without the damping.
    ///
    /// Each point's own unknown enters only its own residual, so it is eliminated first:
    /// (A − B C⁻¹ Bᵀ) δ = b − B C⁻¹ b_p.
    pub fn assemble(&self) -> (Vec<f64>, Vec<f64>) {
        let n = self.shared;
        let mut matrix = vec![0.0; n * n];
        let mut rhs = vec![0.0; n];
        for row in &self.rows {
            let w = row.w;
            let a = row.a;
            let c = w * (a[0] * a[0] + a[1] * a[1]);
            if !c.is_finite() {
                continue;
            }
            // A row with no coordinate of its own (a sheet corner, whose place on the sheet is
            // fixed) has nothing to eliminate: it adds ĴᵀWĴ and −ĴᵀWe to the shared unknowns
            // as they are. Only the Schur correction needs c, so only it is left out. Skipping
            // the whole row dropped all the corner evidence from the step.
            let inverse = if c > 0.0 { 1.0 / c } else { 0.0 };
            let b = w * (a[0] * row.e[0] + a[1] * row.e[1]);
            // v = Ĵᵀ W a, for the shared columns this row touches.
            let mut v = Vec::with_capacity(row.columns.len());
            for (_, dx, dy) in &row.columns {
                v.push(w * (dx * a[0] + dy * a[1]));
            }
            for (p, &(column, dx, dy)) in row.columns.iter().enumerate() {
                let cross = w * (dx * row.e[0] + dy * row.e[1]);
                if let Some(slot) = rhs.get_mut(column) {
                    *slot += -cross + v[p] * b * inverse;
                }
                for (q, &(_, ex, ey)) in row.columns.iter().enumerate() {
                    let inner = w * (dx * ex + dy * ey);
                    let outer = v[p] * v[q] * inverse;
                    let index = column * n + row.columns[q].0;
                    if let Some(slot) = matrix.get_mut(index) {
                        *slot += inner - outer;
                    }
                }
            }
        }
        for term in &self.extra {
            if let Some(slot) = matrix.get_mut(term.row * n + term.column) {
                *slot += term.second;
            }
            if let Some(slot) = rhs.get_mut(term.row) {
                *slot -= term.first;
            }
        }
        (matrix, rhs)
    }

    /// The step for the shared unknowns: the solution of the damped normal equations. `None` when
    /// the matrix is not positive definite, which counts as a rejected step.
    pub fn step(&self) -> Option<Vec<f64>> {
        let (mut matrix, rhs) = self.assemble();
        for i in 0..self.shared {
            let diagonal = matrix.get(i * self.shared + i).copied().unwrap_or(0.0);
            let index = i * self.shared + i;
            if let Some(slot) = matrix.get_mut(index) {
                if diagonal > 0.0 {
                    *slot += self.damping * diagonal;
                } else {
                    // No row and no penalty touches this unknown (in the plane fit, the profile's
                    // ten), so its whole row and column are zero and damping by μ·diagonal leaves
                    // the matrix singular. A one on the diagonal sets its step to zero, which is
                    // what no information means, and lets the others be solved.
                    *slot += 1.0;
                }
            }
        }
        let factor = Cholesky::factor(&matrix, self.shared)?;
        factor.solve(&rhs)
    }

    /// The step for each row's own unknown, given the shared step: the minimiser of the row's
    /// own quadratic model.
    pub fn own_steps(&self, delta: &[f64]) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let w = row.w;
            let a = row.a;
            let c = w * (a[0] * a[0] + a[1] * a[1]);
            if !c.is_finite() || c <= 0.0 {
                out.push(0.0);
                continue;
            }
            let mut shifted = [row.e[0], row.e[1]];
            for &(column, dx, dy) in &row.columns {
                let d = delta.get(column).copied().unwrap_or(0.0);
                shifted[0] += dx * d;
                shifted[1] += dy * d;
            }
            out.push(-(a[0] * shifted[0] + a[1] * shifted[1]) / (a[0] * a[0] + a[1] * a[1]));
        }
        out
    }

    /// The decrease the linearised model predicts: the quadratic model's cost at the current
    /// point minus its cost at the step.
    pub fn predicted_decrease(&self, delta: &[f64], own: &[f64]) -> f64 {
        let mut before = 0.0;
        let mut after = 0.0;
        for (index, row) in self.rows.iter().enumerate() {
            let w = row.w;
            before += w * (row.e[0] * row.e[0] + row.e[1] * row.e[1]);
            let mut shifted = [row.e[0], row.e[1]];
            for &(column, dx, dy) in &row.columns {
                let d = delta.get(column).copied().unwrap_or(0.0);
                shifted[0] += dx * d;
                shifted[1] += dy * d;
            }
            let step = own.get(index).copied().unwrap_or(0.0);
            shifted[0] += row.a[0] * step;
            shifted[1] += row.a[1] * step;
            after += w * (shifted[0] * shifted[0] + shifted[1] * shifted[1]);
        }
        0.5 * (before - after)
    }
}

/// The polar decomposition's rotation: the nearest rotation to the matrix `a`, found by
/// 20 iterations of X ← (X + X⁻ᵀ)/2 (Higham 1986), as A27's start asks.
pub fn nearest_rotation(a: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut x = *a;
    for _ in 0..20 {
        let inverse = invert3(&x).unwrap_or(x);
        let mut next = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                next[i][j] = 0.5 * (x[i][j] + inverse[j][i]);
            }
        }
        x = next;
    }
    x
}

/// The inverse of a 3 × 3 matrix, or `None` when it is singular.
pub fn invert3(a: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let inv_det = 1.0 / det;
    let mut out = [[0.0; 3]; 3];
    out[0][0] = (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inv_det;
    out[0][1] = (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inv_det;
    out[0][2] = (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inv_det;
    out[1][0] = (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inv_det;
    out[1][1] = (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inv_det;
    out[1][2] = (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inv_det;
    out[2][0] = (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inv_det;
    out[2][1] = (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inv_det;
    out[2][2] = (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inv_det;
    Some(out)
}

/// The matrix of a rotation vector, for the tests.
pub fn rotation_matrix(w: [f64; 3]) -> [[f64; 3]; 3] {
    rodrigues(w)
}

/// Fails when a value is not finite, for the callers that need an error.
pub fn require_finite(values: &[f64], what: &'static str) -> Result<(), GeomError> {
    if values.iter().all(|v| v.is_finite()) { Ok(()) } else { Err(GeomError::Degenerate(what)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimiser of ½ Σ w |e + J δ + a s|² over the shared step δ and every row's own step s,
    /// by a dense solve of the whole normal matrix with no elimination: the reference the Schur
    /// complement of `LmSystem` must agree with. Rows without a coordinate of their own (a = 0)
    /// have no s.
    fn dense_shared_step(system: &LmSystem) -> Vec<f64> {
        let n = system.shared;
        let own_rows: Vec<usize> = system.rows.iter().enumerate().filter(|(_, r)| r.a != [0.0, 0.0]).map(|(i, _)| i).collect();
        let size = n + own_rows.len();
        let mut matrix = vec![0.0; size * size];
        let mut rhs = vec![0.0; size];
        for (index, row) in system.rows.iter().enumerate() {
            let own_column = own_rows.iter().position(|&r| r == index).map(|k| n + k);
            for component in 0..2 {
                let mut gradient: Vec<(usize, f64)> = row.columns.iter().map(|&(c, dx, dy)| (c, if component == 0 { dx } else { dy })).collect();
                if let Some(column) = own_column {
                    gradient.push((column, row.a[component]));
                }
                for &(i, gi) in &gradient {
                    rhs[i] -= row.w * gi * row.e[component];
                    for &(j, gj) in &gradient {
                        matrix[i * size + j] += row.w * gi * gj;
                    }
                }
            }
        }
        let x = Cholesky::factor(&matrix, size).unwrap().solve(&rhs).unwrap();
        x[..n].to_vec()
    }

    /// Eight rows on three shared unknowns: five with a coordinate of their own, and three
    /// without (like a sheet corner, whose place on the sheet is fixed).
    fn mixed_system(seed: u64, damping: f64) -> LmSystem {
        let mut rng = scancraft_core::determinism::Xorshift64Star::new(seed);
        let mut unit = move || 2.0 * rng.next_f64() - 1.0;
        let mut rows = Vec::new();
        for index in 0..8usize {
            let columns = vec![(index % 3, unit(), unit()), ((index + 1) % 3, unit(), unit())];
            let a = if index < 5 { [1.0 + unit() * 0.4, unit() * 0.4] } else { [0.0, 0.0] };
            rows.push(LmRow { e: [unit(), unit()], w: 0.6 + 0.3 * unit().abs(), columns, a });
        }
        LmSystem { shared: 3, rows, extra: Vec::new(), damping }
    }

    #[test]
    fn the_step_agrees_with_a_dense_solve_and_rows_without_an_own_coordinate_count() {
        // The corner evidence of A27 has no coordinate of its own. It still pulls on the shared
        // unknowns, and a step that leaves it out is a different step.
        for seed in 1..=6 {
            let system = mixed_system(seed, 0.0);
            let step = system.step().expect("the system has information in every direction");
            let dense = dense_shared_step(&system);
            for (i, (got, want)) in step.iter().zip(&dense).enumerate() {
                assert!((got - want).abs() < 1e-9, "seed {seed}, unknown {i}: {got} against {want}");
            }
        }
    }

    #[test]
    fn an_unknown_that_no_row_touches_gets_no_step_instead_of_failing_the_solve() {
        // In A27's plane fit the ten profile unknowns have no rows and no penalty. Their
        // diagonal is zero, which damping by μ·diagonal leaves at zero, and the solve used to fail
        // on every iteration, so the plane was never fitted.
        let rows = vec![
            LmRow { e: [1.0, 0.5], w: 1.0, columns: vec![(0, 1.0, 0.0), (1, 0.0, 1.0)], a: [0.0, 0.0] },
            LmRow { e: [-0.5, 2.0], w: 1.0, columns: vec![(0, 0.5, 0.2), (1, 0.1, 1.0)], a: [0.0, 0.0] },
        ];
        for damping in [0.0, 1e-3, 1.0] {
            let system = LmSystem { shared: 4, rows: rows.clone(), extra: Vec::new(), damping };
            let step = system.step().expect("a zero diagonal must not fail the solve");
            assert_eq!(step[2], 0.0);
            assert_eq!(step[3], 0.0);
            assert!(step[0].abs() + step[1].abs() > 0.0);
        }
    }

    #[test]
    fn cholesky_solves_a_small_system() {
        // A 5 × 5 symmetric positive-definite system.
        let mut a = vec![0.0; 25];
        for i in 0..5 {
            for j in 0..5 {
                let v = if i == j { 6.0 + i as f64 } else { 1.0 / (1.0 + (i as f64 - j as f64).abs()) };
                a[i * 5 + j] = v;
            }
        }
        let b = [1.0, -2.0, 0.5, 3.0, -1.5];
        let factor = Cholesky::factor(&a, 5).unwrap();
        let x = factor.solve(&b).unwrap();
        for i in 0..5 {
            let mut sum = 0.0;
            for j in 0..5 {
                sum += a[i * 5 + j] * x[j];
            }
            assert!((sum - b[i]).abs() < 1e-12, "row {i}");
        }
    }

    #[test]
    fn a_matrix_that_is_not_positive_definite_is_an_error() {
        // A symmetric matrix with a negative eigenvalue.
        let a = [1.0, 2.0, 2.0, 1.0];
        assert!(Cholesky::factor(&a, 2).is_none());
        // A singular one.
        let b = [1.0, 1.0, 1.0, 1.0];
        assert!(Cholesky::factor(&b, 2).is_none());
    }

    #[test]
    fn rodrigues_matches_the_exponential() {
        let quarter = std::f64::consts::FRAC_PI_2;
        let r = rodrigues([0.0, 0.0, quarter]);
        let expected = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        for i in 0..3 {
            for j in 0..3 {
                assert!((r[i][j] - expected[i][j]).abs() < 1e-12, "{i},{j}");
            }
        }
        let identity = rodrigues([0.0, 0.0, 0.0]);
        for (i, row) in identity.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                assert!((*value - if i == j { 1.0 } else { 0.0 }).abs() < 1e-15);
            }
        }
    }

    #[test]
    fn the_cauchy_loss_and_its_weight_agree() {
        assert!((cauchy_loss(0.0)).abs() < 1e-15);
        assert!((cauchy_weight(0.0) - 1.0).abs() < 1e-15);
        // The weight is the derivative of the loss by the squared residual, halved.
        let e2 = 3.0;
        let analytic = 1.0 / (1.0 + e2 / 4.0);
        assert!((cauchy_weight(e2) - analytic).abs() < 1e-15);
        assert!(cauchy_loss(e2) > 0.0);
    }

    #[test]
    fn the_nearest_rotation_is_a_rotation() {
        let a = [[1.1, 0.2, -0.1], [0.05, 0.95, 0.3], [-0.2, 0.1, 1.2]];
        let r = nearest_rotation(&a);
        // RᵀR is the identity.
        for (i, _) in r.iter().enumerate() {
            for (j, _) in r.iter().enumerate() {
                let mut sum = 0.0;
                for row in r.iter() {
                    sum += row[i] * row[j];
                }
                assert!((sum - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9, "{i},{j}");
            }
        }
        let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1]) - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
        assert!((det - 1.0).abs() < 1e-9);
    }
}
