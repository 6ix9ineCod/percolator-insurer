pub mod bounds;
pub mod rate_limit;
pub mod objective;

use bounds::ParamBounds;

pub struct OptimizeResult {
    pub best_params: Vec<f64>,
    pub best_score: f64,
    pub iterations: u32,
}

pub fn nelder_mead<F>(
    bounds: &[ParamBounds],
    evaluate: F,
    max_iter: u32,
    stale_limit: u32,
    seed: Option<u64>,
) -> OptimizeResult
where
    F: Fn(&[f64]) -> f64,
{
    let n = bounds.len();
    let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(n + 1);

    let mut rng_state = seed.unwrap_or(42);
    let mut next_rng = || -> f64 {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng_state >> 33) as f64) / (u32::MAX as f64)
    };

    for i in 0..=n {
        let mut point = Vec::with_capacity(n);
        for j in 0..n {
            let v = if i == 0 {
                bounds[j].min + 0.5 * bounds[j].range()
            } else if i - 1 == j {
                bounds[j].min + 0.75 * bounds[j].range()
            } else {
                bounds[j].min + next_rng() * bounds[j].range()
            };
            point.push(bounds[j].clamp(v));
        }
        simplex.push(point);
    }

    let mut scores: Vec<f64> = simplex.iter().map(|p| evaluate(p)).collect();
    let mut best_score = f64::NEG_INFINITY;
    let mut best_params = simplex[0].clone();
    let mut stale_count = 0u32;

    let clamp_point = |p: &mut Vec<f64>| {
        for (i, v) in p.iter_mut().enumerate() {
            *v = bounds[i].clamp(*v);
        }
    };

    for iter in 0..max_iter {
        let mut order: Vec<usize> = (0..=n).collect();
        order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());

        let best_idx = order[0];
        let worst_idx = order[n];
        let second_worst_idx = order[n - 1];

        if scores[best_idx] > best_score {
            best_score = scores[best_idx];
            best_params = simplex[best_idx].clone();
            stale_count = 0;
        } else {
            stale_count += 1;
        }

        if stale_count >= stale_limit {
            return OptimizeResult {
                best_params,
                best_score,
                iterations: iter,
            };
        }

        let diameter: f64 = (0..n).map(|d| {
            let vals: Vec<f64> = simplex.iter().map(|p| p[d]).collect();
            let mn = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            (mx - mn) / bounds[d].range()
        }).fold(0.0f64, f64::max);

        if diameter < 0.01 {
            return OptimizeResult {
                best_params,
                best_score,
                iterations: iter,
            };
        }

        let mut centroid = vec![0.0; n];
        for &i in &order[..n] {
            for d in 0..n {
                centroid[d] += simplex[i][d];
            }
        }
        for d in 0..n {
            centroid[d] /= n as f64;
        }

        // Reflection
        let mut reflected: Vec<f64> = (0..n)
            .map(|d| 2.0 * centroid[d] - simplex[worst_idx][d])
            .collect();
        clamp_point(&mut reflected);
        let reflected_score = evaluate(&reflected);

        if reflected_score > scores[second_worst_idx] && reflected_score <= scores[best_idx] {
            simplex[worst_idx] = reflected;
            scores[worst_idx] = reflected_score;
            continue;
        }

        // Expansion
        if reflected_score > scores[best_idx] {
            let mut expanded: Vec<f64> = (0..n)
                .map(|d| 3.0 * centroid[d] - 2.0 * simplex[worst_idx][d])
                .collect();
            clamp_point(&mut expanded);
            let expanded_score = evaluate(&expanded);
            if expanded_score > reflected_score {
                simplex[worst_idx] = expanded;
                scores[worst_idx] = expanded_score;
            } else {
                simplex[worst_idx] = reflected;
                scores[worst_idx] = reflected_score;
            }
            continue;
        }

        // Contraction
        let mut contracted: Vec<f64> = (0..n)
            .map(|d| 0.5 * (centroid[d] + simplex[worst_idx][d]))
            .collect();
        clamp_point(&mut contracted);
        let contracted_score = evaluate(&contracted);

        if contracted_score > scores[worst_idx] {
            simplex[worst_idx] = contracted;
            scores[worst_idx] = contracted_score;
            continue;
        }

        // Shrink
        for i in 1..=n {
            let idx = order[i];
            for d in 0..n {
                simplex[idx][d] = 0.5 * (simplex[best_idx][d] + simplex[idx][d]);
                simplex[idx][d] = bounds[d].clamp(simplex[idx][d]);
            }
            scores[idx] = evaluate(&simplex[idx]);
        }
    }

    OptimizeResult {
        best_params,
        best_score,
        iterations: max_iter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bounds::ParamBounds;

    #[test]
    fn optimize_simple_quadratic() {
        let bounds = vec![
            ParamBounds::new(0.0, 10.0),
            ParamBounds::new(0.0, 10.0),
        ];
        let result = nelder_mead(
            &bounds,
            |params| -((params[0] - 3.0).powi(2) + (params[1] - 7.0).powi(2)),
            100,
            50,
            None,
        );
        assert!((result.best_params[0] - 3.0).abs() < 0.5);
        assert!((result.best_params[1] - 7.0).abs() < 0.5);
    }

    #[test]
    fn respects_bounds() {
        let bounds = vec![
            ParamBounds::new(5.0, 10.0),
        ];
        let result = nelder_mead(
            &bounds,
            |params| -((params[0] - 3.0).powi(2)),
            50,
            20,
            None,
        );
        assert!(result.best_params[0] >= 5.0);
    }

    #[test]
    fn returns_after_max_iterations() {
        let bounds = vec![ParamBounds::new(0.0, 100.0)];
        let result = nelder_mead(
            &bounds,
            |params| -(params[0] - 50.0).powi(2),
            10,
            5,
            None,
        );
        assert!(result.iterations <= 10);
    }
}
