use crate::error::FileFormat;

#[derive(Debug, Clone)]
pub struct HeaderCandidate {
    pub offset: u64,
    pub format: FileFormat,
    pub quality: f64,
    pub dimensions: Option<(u32, u32)>,
    pub expected_size_range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct FooterCandidate {
    pub offset: u64,
    pub format: FileFormat,
    pub quality: f64,
}

#[derive(Debug, Clone)]
pub struct MatchWeight {
    pub score: f64,
    pub evidence: MatchEvidence,
}

#[derive(Debug, Clone, Default)]
pub struct MatchEvidence {
    pub format_match: bool,
    pub distance_reasonable: bool,
    pub size_in_expected_range: bool,
    pub structure_valid: Option<bool>,
    pub stats_consistent: bool,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub header_idx: usize,
    pub footer_idx: usize,
    pub weight: MatchWeight,
}

pub struct GlobalMatcher {
    headers: Vec<HeaderCandidate>,
    footers: Vec<FooterCandidate>,
    config: MatcherConfig,
}

#[derive(Debug, Clone)]
pub struct MatcherConfig {
    pub min_weight: f64,
    pub max_file_size: u64,
    pub min_file_size: u64,
    pub allow_multi_footer: bool,
}

impl Default for MatcherConfig {
    fn default() -> Self {
        Self {
            min_weight: 0.1,
            max_file_size: 100 * 1024 * 1024,
            min_file_size: 100,
            allow_multi_footer: false,
        }
    }
}

impl GlobalMatcher {
    pub fn new() -> Self {
        Self::with_config(MatcherConfig::default())
    }

    pub fn with_config(config: MatcherConfig) -> Self {
        Self {
            headers: Vec::new(),
            footers: Vec::new(),
            config,
        }
    }

    pub fn add_header(&mut self, header: HeaderCandidate) {
        self.headers.push(header);
    }

    pub fn add_footer(&mut self, footer: FooterCandidate) {
        self.footers.push(footer);
    }

    pub fn header_count(&self) -> usize {
        self.headers.len()
    }

    pub fn footer_count(&self) -> usize {
        self.footers.len()
    }

    pub fn clear(&mut self) {
        self.headers.clear();
        self.footers.clear();
    }

    pub fn evaluate_pair(&self, header: &HeaderCandidate, footer: &FooterCandidate) -> MatchWeight {
        let mut evidence = MatchEvidence::default();
        let mut score = 0.0;

        if header.format != footer.format {
            return MatchWeight {
                score: 0.0,
                evidence,
            };
        }
        evidence.format_match = true;
        score += 0.2;

        if footer.offset <= header.offset {
            return MatchWeight {
                score: 0.0,
                evidence,
            };
        }

        let distance = footer.offset - header.offset;

        if distance < self.config.min_file_size || distance > self.config.max_file_size {
            return MatchWeight {
                score: 0.0,
                evidence,
            };
        }
        evidence.distance_reasonable = true;
        score += 0.1;

        if let Some((min_size, max_size)) = header.expected_size_range {
            if distance >= min_size && distance <= max_size {
                evidence.size_in_expected_range = true;
                score += 0.3;
            } else if distance >= min_size / 2 && distance <= max_size * 2 {
                score += 0.1;
            }
        } else {
            let typical_size = match header.format {
                FileFormat::Jpeg => 500_000,
                FileFormat::Png => 200_000,
                FileFormat::Unknown => 100_000,
            };

            let size_ratio = distance as f64 / typical_size as f64;
            if (0.1..=10.0).contains(&size_ratio) {
                evidence.size_in_expected_range = true;
                score += 0.2 * (1.0 - (size_ratio.log10().abs() / 2.0)).max(0.0);
            }
        }

        score += header.quality * 0.15;
        score += footer.quality * 0.15;

        let distance_score = 1.0 / (1.0 + (distance as f64 / 1_000_000.0).ln().max(0.0));
        score += distance_score * 0.1;
        evidence.stats_consistent = true;

        MatchWeight {
            score: score.min(1.0),
            evidence,
        }
    }

    fn build_weight_matrix(&self) -> Vec<Vec<f64>> {
        let n_headers = self.headers.len();
        let n_footers = self.footers.len();

        let mut matrix = vec![vec![0.0; n_footers]; n_headers];

        for (i, header) in self.headers.iter().enumerate() {
            for (j, footer) in self.footers.iter().enumerate() {
                let weight = self.evaluate_pair(header, footer);
                matrix[i][j] = weight.score;
            }
        }

        matrix
    }

    pub fn solve_greedy(&self) -> Vec<MatchResult> {
        if self.headers.is_empty() || self.footers.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut used_headers = vec![false; self.headers.len()];
        let mut used_footers = vec![false; self.footers.len()];
        let mut edges: Vec<(usize, usize, MatchWeight)> = Vec::new();

        for (i, header) in self.headers.iter().enumerate() {
            for (j, footer) in self.footers.iter().enumerate() {
                let weight = self.evaluate_pair(header, footer);
                if weight.score >= self.config.min_weight {
                    edges.push((i, j, weight));
                }
            }
        }

        edges.sort_by(|a, b| {
            b.2.score
                .partial_cmp(&a.2.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (h_idx, f_idx, weight) in edges {
            if !used_headers[h_idx] && !used_footers[f_idx] {
                used_headers[h_idx] = true;
                used_footers[f_idx] = true;

                results.push(MatchResult {
                    header_idx: h_idx,
                    footer_idx: f_idx,
                    weight,
                });
            }
        }

        results
    }

    pub fn solve_optimal(&self) -> Vec<MatchResult> {
        if self.headers.is_empty() || self.footers.is_empty() {
            return Vec::new();
        }

        let matrix = self.build_weight_matrix();
        let assignments = hungarian_algorithm(&matrix);

        let mut results = Vec::new();
        for (h_idx, f_idx) in assignments {
            if h_idx < self.headers.len() && f_idx < self.footers.len() {
                let weight = self.evaluate_pair(&self.headers[h_idx], &self.footers[f_idx]);
                if weight.score >= self.config.min_weight {
                    results.push(MatchResult {
                        header_idx: h_idx,
                        footer_idx: f_idx,
                        weight,
                    });
                }
            }
        }

        results.sort_by(|a, b| {
            b.weight
                .score
                .partial_cmp(&a.weight.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    pub fn get_header(&self, idx: usize) -> Option<&HeaderCandidate> {
        self.headers.get(idx)
    }

    pub fn get_footer(&self, idx: usize) -> Option<&FooterCandidate> {
        self.footers.get(idx)
    }

    pub fn headers(&self) -> impl Iterator<Item = &HeaderCandidate> {
        self.headers.iter()
    }

    pub fn footers(&self) -> impl Iterator<Item = &FooterCandidate> {
        self.footers.iter()
    }
}

impl Default for GlobalMatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn hungarian_algorithm(weights: &[Vec<f64>]) -> Vec<(usize, usize)> {
    if weights.is_empty() || weights[0].is_empty() {
        return Vec::new();
    }

    let n_rows = weights.len();
    let n_cols = weights[0].len();
    let n = n_rows.max(n_cols);

    let max_weight = weights
        .iter()
        .flat_map(|row| row.iter())
        .cloned()
        .fold(0.0f64, f64::max);

    let mut cost = vec![vec![0.0; n]; n];

    for i in 0..n_rows {
        for j in 0..n_cols {
            cost[i][j] = max_weight - weights[i][j];
        }
    }

    let assignment = hungarian_solve(&cost);

    assignment
        .into_iter()
        .enumerate()
        .filter(|&(i, j)| i < n_rows && j < n_cols && weights[i][j] > 0.0)
        .collect()
}

fn hungarian_solve(cost: &[Vec<f64>]) -> Vec<usize> {
    let n = cost.len();
    if n == 0 {
        return Vec::new();
    }

    let mut u = vec![0.0; n + 1];
    let mut v = vec![0.0; n + 1];
    let mut p = vec![0; n + 1];
    let mut way = vec![0; n + 1];

    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut minv = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];

        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1 = 0;

            for j in 1..=n {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }

            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }

            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }

        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut assignment = vec![0; n];
    for j in 1..=n {
        if p[j] > 0 && p[j] <= n {
            assignment[p[j] - 1] = j - 1;
        }
    }

    assignment
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_simple_matching() {
        let mut matcher = GlobalMatcher::new();

        matcher.add_header(HeaderCandidate {
            offset: 0,
            format: FileFormat::Jpeg,
            quality: 0.9,
            dimensions: None,
            expected_size_range: None,
        });

        matcher.add_footer(FooterCandidate {
            offset: 10000,
            format: FileFormat::Jpeg,
            quality: 0.9,
        });

        let results = matcher.solve_greedy();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].header_idx, 0);
        assert_eq!(results[0].footer_idx, 0);
    }

    #[test]
    fn test_format_mismatch() {
        let mut matcher = GlobalMatcher::new();

        matcher.add_header(HeaderCandidate {
            offset: 0,
            format: FileFormat::Jpeg,
            quality: 0.9,
            dimensions: None,
            expected_size_range: None,
        });

        matcher.add_footer(FooterCandidate {
            offset: 10000,
            format: FileFormat::Png,
            quality: 0.9,
        });

        let results = matcher.solve_greedy();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_interleaved_matching() {
        let mut matcher = GlobalMatcher::new();

        matcher.add_header(HeaderCandidate {
            offset: 0,
            format: FileFormat::Jpeg,
            quality: 0.9,
            dimensions: None,
            expected_size_range: Some((2500, 4000)),
        });

        matcher.add_header(HeaderCandidate {
            offset: 1000,
            format: FileFormat::Jpeg,
            quality: 0.9,
            dimensions: None,
            expected_size_range: Some((500, 1500)),
        });

        matcher.add_footer(FooterCandidate {
            offset: 2000,
            format: FileFormat::Jpeg,
            quality: 0.9,
        });

        matcher.add_footer(FooterCandidate {
            offset: 3000,
            format: FileFormat::Jpeg,
            quality: 0.9,
        });

        let results = matcher.solve_optimal();
        assert_eq!(results.len(), 2);

        let mut matches: HashMap<usize, usize> = HashMap::new();
        for r in &results {
            matches.insert(r.header_idx, r.footer_idx);
        }

        assert_eq!(matches.get(&0), Some(&1));
        assert_eq!(matches.get(&1), Some(&0));
    }

    #[test]
    fn test_hungarian_simple() {
        let weights = vec![vec![3.0, 1.0], vec![2.0, 4.0]];
        let result = hungarian_algorithm(&weights);
        let mut sum = 0.0;

        for (i, j) in &result {
            sum += weights[*i][*j];
        }
        assert!((sum - 7.0).abs() < 0.001);
    }
}
