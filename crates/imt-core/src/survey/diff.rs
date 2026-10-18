//! A small line diff for reviewing profile changes (common prefix and suffix, LCS in between).

use super::dto::{DiffLine, DiffOp};

/// Largest changed middle part diffed line by line; bigger ones show as removed then added.
const MAX_MIDDLE: usize = 2_000;

fn line(op: DiffOp, text: &str) -> DiffLine {
    DiffLine {
        op,
        text: text.to_string(),
    }
}

/// The lines of `after` against `before`, in order.
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out: Vec<DiffLine> = a[..prefix].iter().map(|l| line(DiffOp::Same, l)).collect();
    if am.len() > MAX_MIDDLE || bm.len() > MAX_MIDDLE {
        out.extend(am.iter().map(|l| line(DiffOp::Remove, l)));
        out.extend(bm.iter().map(|l| line(DiffOp::Add, l)));
    } else {
        // lcs[i][j] = length of the LCS of am[i..] and bm[j..].
        let (n, m) = (am.len(), bm.len());
        let mut lcs = vec![vec![0u16; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i][j] = if am[i] == bm[j] {
                    lcs[i + 1][j + 1] + 1
                } else {
                    lcs[i + 1][j].max(lcs[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(line(DiffOp::Same, am[i]));
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                out.push(line(DiffOp::Add, bm[j]));
                j += 1;
            } else {
                out.push(line(DiffOp::Remove, am[i]));
                i += 1;
            }
        }
    }
    out.extend(a[a.len() - suffix..].iter().map(|l| line(DiffOp::Same, l)));
    out
}
