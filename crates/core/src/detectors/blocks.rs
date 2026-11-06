use image::GrayImage;

use super::stats::median;
use crate::Rect;

/// A group of flagged blocks that touch each other.
pub(crate) struct Cluster {
    pub min_col: usize,
    pub max_col: usize,
    pub min_row: usize,
    pub max_row: usize,
    pub members: Vec<usize>,
}

impl Cluster {
    /// Pixel rectangle covered by the cluster, clipped to the page.
    pub fn region(&self, block: u32, width: u32, height: u32) -> Rect {
        let x = self.min_col as u32 * block;
        let y = self.min_row as u32 * block;
        let right = ((self.max_col as u32 + 1) * block).min(width);
        let bottom = ((self.max_row as u32 + 1) * block).min(height);
        Rect::new(x as f32, y as f32, (right - x) as f32, (bottom - y) as f32)
    }
}

/// Groups flagged blocks (stored row by row) into connected clusters.
pub(crate) fn find_clusters(flagged: &[bool], cols: usize, rows: usize) -> Vec<Cluster> {
    let mut seen = vec![false; flagged.len()];
    let mut clusters = Vec::new();

    for start in 0..flagged.len() {
        if !flagged[start] || seen[start] {
            continue;
        }

        seen[start] = true;
        let mut stack = vec![start];
        let mut cluster = Cluster {
            min_col: cols,
            max_col: 0,
            min_row: rows,
            max_row: 0,
            members: Vec::new(),
        };

        while let Some(i) = stack.pop() {
            let (col, row) = (i % cols, i / cols);
            cluster.min_col = cluster.min_col.min(col);
            cluster.max_col = cluster.max_col.max(col);
            cluster.min_row = cluster.min_row.min(row);
            cluster.max_row = cluster.max_row.max(row);
            cluster.members.push(i);

            for n in neighbours(i, cols, rows).into_iter().flatten() {
                if flagged[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        clusters.push(cluster);
    }
    clusters
}

fn neighbours(i: usize, cols: usize, rows: usize) -> [Option<usize>; 4] {
    let (col, row) = (i % cols, i / cols);
    [
        (col > 0).then(|| i - 1),
        (col + 1 < cols).then(|| i + 1),
        (row > 0).then(|| i - cols),
        (row + 1 < rows).then(|| i + cols),
    ]
}

/// Average brightness change between neighbouring pixels in each block,
/// stored row by row. Blank paper scores near zero and printed text scores
/// high, which lets detectors compare like with like.
pub(crate) fn block_activity(gray: &GrayImage, block: u32) -> Vec<f32> {
    let (width, height) = gray.dimensions();
    let cols = width.div_ceil(block) as usize;
    let rows = height.div_ceil(block) as usize;
    let mut sums = vec![0f32; cols * rows];
    let mut counts = vec![0u32; cols * rows];

    for y in 0..height {
        for x in 0..width {
            let here = i32::from(gray.get_pixel(x, y)[0]);
            let right = if x + 1 < width {
                i32::from(gray.get_pixel(x + 1, y)[0])
            } else {
                here
            };
            let below = if y + 1 < height {
                i32::from(gray.get_pixel(x, y + 1)[0])
            } else {
                here
            };

            let index = (y / block) as usize * cols + (x / block) as usize;
            sums[index] += ((here - right).abs() + (here - below).abs()) as f32;
            counts[index] += 1;
        }
    }

    sums.iter()
        .zip(&counts)
        .map(|(sum, count)| {
            if *count == 0 {
                0.0
            } else {
                sum / *count as f32
            }
        })
        .collect()
}

/// How far a block sits from comparable blocks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeerScore {
    /// Robust z-score: positive means higher than its peers.
    pub z: f32,
    /// The median value among its peers.
    pub typical: f32,
}

/// Scores each block against the blocks nearest to it in activity.
///
/// Blocks are ranked by `activity`, and a block's peers are the
/// `half_window` blocks on either side of it in that ranking. The median
/// and median absolute deviation are used so a few tampered blocks cannot
/// distort the baseline they are judged against.
pub(crate) fn peer_scores(
    values: &[f32],
    activity: &[f32],
    half_window: usize,
    min_spread: f32,
) -> Vec<PeerScore> {
    let count = values.len().min(activity.len());
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| activity[a].total_cmp(&activity[b]));

    let mut scores = vec![
        PeerScore {
            z: 0.0,
            typical: 0.0
        };
        values.len()
    ];

    for (rank, &block) in order.iter().enumerate() {
        let start = rank.saturating_sub(half_window);
        let end = (rank + half_window + 1).min(count);

        let mut peers: Vec<f32> = order[start..end].iter().map(|&i| values[i]).collect();
        let centre = median(&mut peers);
        let mut deviations: Vec<f32> = peers.iter().map(|v| (v - centre).abs()).collect();
        let spread = (1.4826 * median(&mut deviations)).max(min_spread);

        scores[block] = PeerScore {
            z: (values[block] - centre) / spread,
            typical: centre,
        };
    }
    scores
}

#[cfg(test)]
mod tests {
    use image::Luma;

    use super::*;

    /// Forty quiet blocks with small values, forty busy blocks with large ones.
    fn two_groups() -> (Vec<f32>, Vec<f32>) {
        let mut values = Vec::new();
        let mut activity = Vec::new();
        for i in 0..40 {
            activity.push(i as f32 * 0.01);
            values.push(1.0 + (i % 3) as f32 * 0.1);
        }
        for i in 0..40 {
            activity.push(100.0 + i as f32 * 0.01);
            values.push(50.0 + (i % 3) as f32 * 0.5);
        }
        (values, activity)
    }

    #[test]
    fn blocks_are_judged_against_similar_blocks_only() {
        let (values, activity) = two_groups();
        let scores = peer_scores(&values, &activity, 10, 0.5);

        // A global baseline would call every busy block an outlier.
        assert!(scores.iter().all(|s| s.z.abs() < 3.0));
    }

    #[test]
    fn an_outlier_among_its_peers_stands_out() {
        let (mut values, mut activity) = two_groups();
        values.push(90.0);
        activity.push(100.2);

        let scores = peer_scores(&values, &activity, 10, 0.5);
        assert!(scores[80].z > 4.0);
        assert!((scores[80].typical - 50.5).abs() < 1.0);
        assert!(scores[..80].iter().all(|s| s.z.abs() < 3.0));
    }

    #[test]
    fn identical_values_score_zero() {
        let scores = peer_scores(&[2.0; 30], &[0.0; 30], 5, 0.5);
        assert!(scores.iter().all(|s| s.z == 0.0));
    }

    #[test]
    fn nothing_in_gives_nothing_out() {
        assert!(peer_scores(&[], &[], 5, 0.5).is_empty());
    }

    #[test]
    fn activity_is_high_only_where_there_is_an_edge() {
        let page = GrayImage::from_fn(32, 16, |x, _| Luma([if x < 16 { 255 } else { 0 }]));
        let activity = block_activity(&page, 16);

        assert_eq!(activity.len(), 2);
        assert!(activity[0] > 10.0);
        assert_eq!(activity[1], 0.0);
    }
}
