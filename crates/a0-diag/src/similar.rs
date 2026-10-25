//! "Did you mean" suggestions: the declared name closest to a misspelled one.

/// The candidate closest to `name`, if one is close enough to be a likely misspelling: the same
/// name in another case, or one within an edit distance of a third of the name's length (at least
/// one), the way rustc chooses. Ties go to the earlier candidate, so list the likeliest first
/// (innermost scope first).
pub fn similar_name(name: &str, candidates: &[&str]) -> Option<String> {
    let lower = name.to_lowercase();
    if let Some(c) = candidates
        .iter()
        .find(|c| **c != name && c.to_lowercase() == lower)
    {
        return Some(c.to_string());
    }
    let limit = (name.chars().count().max(3) / 3).max(1);
    let mut best: Option<(usize, &str)> = None;
    for &c in candidates {
        if c == name {
            continue;
        }
        let d = distance(name, c);
        if d <= limit && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c.to_string())
}

/// The Damerau-Levenshtein distance (with adjacent transpositions), in characters.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = d;
        }
    }
    rows[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_names_are_suggested_and_far_ones_are_not() {
        let names = ["count", "total", "Point", "x"];
        assert_eq!(similar_name("cuont", &names).as_deref(), Some("count"));
        assert_eq!(similar_name("totl", &names).as_deref(), Some("total"));
        assert_eq!(similar_name("point", &names).as_deref(), Some("Point"));
        assert_eq!(similar_name("y", &names).as_deref(), Some("x"));
        assert_eq!(similar_name("elephant", &names), None);
        assert_eq!(similar_name("count", &names), None);
        assert_eq!(similar_name("abc", &[]), None);
    }

    #[test]
    fn distance_counts_transpositions_once() {
        assert_eq!(distance("ab", "ba"), 1);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "abc"), 3);
    }
}
