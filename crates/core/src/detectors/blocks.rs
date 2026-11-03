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
