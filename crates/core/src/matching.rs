//! Name comparison that tolerates the usual differences between documents.

/// How two names relate to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMatch {
    /// The same words in the same order.
    Same,
    /// Probably the same person: reordered, shortened, initialled or mistyped.
    Variation,
    Different,
}

/// Number of single-character edits needed to turn `a` into `b`.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            let deletion = previous[j + 1] + 1;
            let insertion = current[j] + 1;
            current[j + 1] = substitution.min(deletion).min(insertion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// 1.0 for identical text, falling towards 0.0 as the texts differ.
pub fn similarity(a: &str, b: &str) -> f32 {
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - levenshtein(a, b) as f32 / longest as f32
}

pub fn compare_names(a: &str, b: &str) -> NameMatch {
    let left = tokens(a);
    let right = tokens(b);

    if left.is_empty() || right.is_empty() {
        return NameMatch::Different;
    }
    if left == right {
        return NameMatch::Same;
    }

    let (short, long) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };

    // One shared word is not enough to call two names the same person.
    if short.len() < 2 {
        return NameMatch::Different;
    }

    // Every word of the shorter name must pair with its own word in the longer one.
    let mut used = vec![false; long.len()];
    for token in short {
        let found = long
            .iter()
            .enumerate()
            .find(|(i, other)| !used[*i] && tokens_match(token, other));

        match found {
            Some((i, _)) => used[i] = true,
            None => return NameMatch::Different,
        }
    }
    NameMatch::Variation
}

fn tokens(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_uppercase)
        .collect()
}

fn tokens_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if (is_initial(a) && b.starts_with(a)) || (is_initial(b) && a.starts_with(b)) {
        return true;
    }
    // Typos are only forgiven in longer words; short names must match exactly.
    a.chars().count() >= 5 && b.chars().count() >= 5 && similarity(a, b) >= 0.85
}

fn is_initial(token: &str) -> bool {
    token.chars().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_matches_the_textbook_example() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[test]
    fn punctuation_and_case_do_not_matter() {
        assert_eq!(compare_names("R. Kumar", "R KUMAR"), NameMatch::Same);
    }

    #[test]
    fn reordered_names_are_a_variation() {
        assert_eq!(
            compare_names("RAHUL KUMAR SHARMA", "SHARMA RAHUL KUMAR"),
            NameMatch::Variation
        );
    }

    #[test]
    fn dropped_middle_name_is_a_variation() {
        assert_eq!(
            compare_names("RAHUL SHARMA", "RAHUL KUMAR SHARMA"),
            NameMatch::Variation
        );
    }

    #[test]
    fn initial_matches_the_full_word() {
        assert_eq!(
            compare_names("RAHUL K SHARMA", "RAHUL KUMAR SHARMA"),
            NameMatch::Variation
        );
    }

    #[test]
    fn single_letter_typo_in_a_long_word_is_a_variation() {
        assert_eq!(
            compare_names("RAHUL KUMAR SHARMAA", "RAHUL KUMAR SHARMA"),
            NameMatch::Variation
        );
    }

    #[test]
    fn different_people_are_different() {
        assert_eq!(
            compare_names("RAHUL SHARMA", "ROHAN VERMA"),
            NameMatch::Different
        );
        assert_eq!(
            compare_names("AMIT SHARMA", "SUMIT SHARMA"),
            NameMatch::Different
        );
    }

    #[test]
    fn a_single_shared_word_is_not_enough() {
        assert_eq!(compare_names("RAHUL", "RAHUL SHARMA"), NameMatch::Different);
    }
}
