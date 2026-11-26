//! Bounded subset-sum search used by the group passes.

/// Most amounts the search looks at; each half then has at most 4,096 subsets.
pub(crate) const GROUP_CANDIDATE_LIMIT: usize = 24;

/// Most subsets one search returns.
pub(crate) const GROUP_SUBSET_LIMIT: usize = 32;

/// Finds subsets of 2 to `max_size` indices of `amounts` whose sum is within `tolerance` of
/// `target`.
///
/// Only the first [`GROUP_CANDIDATE_LIMIT`] amounts are considered. Sums use `i128`, so no
/// input overflows. A subset of one amount is never returned. Each subset lists ascending
/// indices, and at most `limit` subsets come back in a deterministic order: when more
/// exist, the first `limit` in enumeration order (left half by mask, then right half by
/// sum) are kept.
pub(crate) fn find_subsets(
    amounts: &[i64],
    target: i64,
    tolerance: u64,
    max_size: usize,
    limit: usize,
) -> Vec<Vec<usize>> {
    let pool = &amounts[..amounts.len().min(GROUP_CANDIDATE_LIMIT)];
    let (left, right) = pool.split_at(pool.len() / 2);
    let left_subsets = subsets_of(left, max_size);
    let mut right_subsets = subsets_of(right, max_size);
    right_subsets.sort_unstable();
    let low = i128::from(target) - i128::from(tolerance);
    let high = i128::from(target) + i128::from(tolerance);
    let mut found = Vec::new();
    // Sums outside this range of the right half cannot match, which skips most searches.
    let (lowest, highest) = match (right_subsets.first(), right_subsets.last()) {
        (Some(&(lowest, ..)), Some(&(highest, ..))) => (lowest, highest),
        _ => return found,
    };
    for &(left_sum, left_size, left_mask) in &left_subsets {
        if high - left_sum < lowest || low - left_sum > highest {
            continue;
        }
        let start = right_subsets.partition_point(|&(sum, ..)| sum < low - left_sum);
        let end = right_subsets.partition_point(|&(sum, ..)| sum <= high - left_sum);
        for &(_, right_size, right_mask) in &right_subsets[start..end] {
            if found.len() >= limit {
                return found;
            }
            if (2..=max_size).contains(&(left_size + right_size)) {
                let mut subset = indices(left_mask, 0);
                subset.extend(indices(right_mask, left.len()));
                found.push(subset);
            }
        }
    }
    found
}

/// Lists every subset of `half` with at most `max_size` members as `(sum, size, mask)`.
///
/// Bit `b` of the mask selects `half[b]`; `half` has at most 12 entries.
fn subsets_of(half: &[i64], max_size: usize) -> Vec<(i128, usize, u32)> {
    // The sum of a mask is the sum of the mask without its lowest bit plus that amount.
    let mut sums = vec![0_i128; 1 << half.len()];
    let mut subsets = Vec::with_capacity(sums.len());
    for mask in 0..1_u32 << half.len() {
        if mask != 0 {
            let rest = mask & (mask - 1);
            sums[mask as usize] =
                sums[rest as usize] + i128::from(half[mask.trailing_zeros() as usize]);
        }
        let size = mask.count_ones() as usize;
        if size <= max_size {
            subsets.push((sums[mask as usize], size, mask));
        }
    }
    subsets
}

/// Returns the positions of the set bits of `mask`, shifted by `offset`.
fn indices(mask: u32, offset: usize) -> Vec<usize> {
    (0..u32::BITS as usize)
        .filter(|&bit| mask >> bit & 1 == 1)
        .map(|bit| bit + offset)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;

    /// Reference implementation: tries every subset of at most 24 amounts.
    fn brute_force(
        amounts: &[i64],
        target: i64,
        tolerance: u64,
        max_size: usize,
    ) -> BTreeSet<Vec<usize>> {
        (0u32..1 << amounts.len())
            .map(|mask| {
                (0..amounts.len())
                    .filter(|&b| mask >> b & 1 == 1)
                    .collect::<Vec<usize>>()
            })
            .filter(|subset| (2..=max_size).contains(&subset.len()))
            .filter(|subset| {
                let sum: i128 = subset.iter().map(|&i| i128::from(amounts[i])).sum();
                sum.abs_diff(i128::from(target)) <= u128::from(tolerance)
            })
            .collect()
    }

    fn as_set(subsets: Vec<Vec<usize>>) -> BTreeSet<Vec<usize>> {
        subsets.into_iter().collect()
    }

    #[test]
    fn no_subset_gives_an_empty_result() {
        assert!(find_subsets(&[10, 20, 30], 7, 0, 3, 32).is_empty());
        assert!(find_subsets(&[], 0, 0, 3, 32).is_empty());
    }

    #[test]
    fn one_subset_is_found_with_ascending_indices() {
        let found = find_subsets(&[10, 20, 30], 50, 0, 3, 32);
        assert_eq!(found, vec![vec![1, 2]]);
    }

    #[test]
    fn several_subsets_are_all_found() {
        let found = find_subsets(&[10, 20, 30, 40], 50, 0, 4, 32);
        assert_eq!(as_set(found), BTreeSet::from([vec![0, 3], vec![1, 2]]));
    }

    #[test]
    fn tolerance_applies_on_both_sides() {
        assert_eq!(find_subsets(&[100, 50], 148, 2, 2, 32), vec![vec![0, 1]]);
        assert_eq!(find_subsets(&[100, 50], 152, 2, 2, 32), vec![vec![0, 1]]);
        assert!(find_subsets(&[100, 50], 153, 2, 2, 32).is_empty());
        assert!(find_subsets(&[100, 50], 147, 2, 2, 32).is_empty());
    }

    #[test]
    fn max_size_is_respected() {
        let amounts = [10, 10, 10, 10];
        assert!(find_subsets(&amounts, 30, 0, 2, 32).is_empty());
        let found = find_subsets(&amounts, 30, 0, 3, 32);
        assert_eq!(found.len(), 4);
        assert!(found.iter().all(|s| s.len() == 3));
    }

    #[test]
    fn size_one_is_never_returned() {
        assert!(find_subsets(&[10, 20], 10, 0, 5, 32).is_empty());
        assert!(find_subsets(&[10, 20], 20, 0, 1, 32).is_empty());
        assert!(find_subsets(&[10, 20], 30, 0, 0, 32).is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let amounts = [1; 10];
        assert_eq!(find_subsets(&amounts, 3, 0, 3, 7).len(), 7);
        assert!(find_subsets(&amounts, 3, 0, 3, 0).is_empty());
        assert_eq!(find_subsets(&amounts, 3, 0, 3, 1000).len(), 120);
    }

    #[test]
    fn twenty_four_amounts_are_searched() {
        let ones = [1; GROUP_CANDIDATE_LIMIT];
        let everything: Vec<usize> = (0..GROUP_CANDIDATE_LIMIT).collect();
        assert_eq!(find_subsets(&ones, 24, 0, 24, 32), vec![everything]);
        assert_eq!(find_subsets(&ones, 3, 0, 5, GROUP_SUBSET_LIMIT).len(), 32);
        let distinct: Vec<i64> = (1..=24).collect();
        let found = find_subsets(&distinct, 300, 0, 24, 32);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].len(), 24);
    }

    #[test]
    fn amounts_beyond_the_candidate_limit_are_ignored() {
        let mut amounts = vec![1; GROUP_CANDIDATE_LIMIT];
        amounts.extend([500, 500]);
        assert!(find_subsets(&amounts, 1000, 0, 5, 32).is_empty());
    }

    #[test]
    fn amounts_near_i64_max_do_not_overflow() {
        let found = find_subsets(&[i64::MAX - 1, 1, i64::MAX], i64::MAX, 0, 3, 32);
        assert_eq!(found, vec![vec![0, 1]]);
        // The sum of the two largest amounts exceeds i64::MAX and still compares exactly.
        let both = [i64::MAX, i64::MAX];
        assert!(find_subsets(&both, i64::MAX, 0, 2, 32).is_empty());
        assert_eq!(
            find_subsets(&both, i64::MAX, i64::MAX.unsigned_abs(), 2, 32),
            vec![vec![0, 1]]
        );
    }

    #[test]
    fn the_result_is_deterministic() {
        let amounts = [5, 5, 10, 10, 15, 15, 20, 20, 25, 25];
        let first = find_subsets(&amounts, 40, 0, 4, 9);
        assert_eq!(first.len(), 9);
        for _ in 0..3 {
            assert_eq!(find_subsets(&amounts, 40, 0, 4, 9), first);
        }
    }

    /// The search as it was before the sums were built incrementally and out-of-range sums
    /// skipped: same enumeration order, one searched range per left subset.
    fn find_subsets_reference(
        amounts: &[i64],
        target: i64,
        tolerance: u64,
        max_size: usize,
        limit: usize,
    ) -> Vec<Vec<usize>> {
        let subsets_of = |half: &[i64]| -> Vec<(i128, usize, u32)> {
            (0..1u32 << half.len())
                .filter_map(|mask| {
                    let size = mask.count_ones() as usize;
                    let sum = (0..half.len())
                        .filter(|&bit| mask >> bit & 1 == 1)
                        .map(|bit| i128::from(half[bit]))
                        .sum();
                    (size <= max_size).then_some((sum, size, mask))
                })
                .collect()
        };
        let pool = &amounts[..amounts.len().min(GROUP_CANDIDATE_LIMIT)];
        let (left, right) = pool.split_at(pool.len() / 2);
        let left_subsets = subsets_of(left);
        let mut right_subsets = subsets_of(right);
        right_subsets.sort_unstable();
        let low = i128::from(target) - i128::from(tolerance);
        let high = i128::from(target) + i128::from(tolerance);
        let mut found = Vec::new();
        for &(left_sum, left_size, left_mask) in &left_subsets {
            let start = right_subsets.partition_point(|&(sum, ..)| sum < low - left_sum);
            let end = right_subsets.partition_point(|&(sum, ..)| sum <= high - left_sum);
            for &(_, right_size, right_mask) in &right_subsets[start..end] {
                if found.len() >= limit {
                    return found;
                }
                if (2..=max_size).contains(&(left_size + right_size)) {
                    let mut subset = indices(left_mask, 0);
                    subset.extend(indices(right_mask, left.len()));
                    found.push(subset);
                }
            }
        }
        found
    }

    proptest! {
        #[test]
        fn the_result_and_its_order_match_the_previous_search(
            amounts in prop::collection::vec(-40i64..80, 0..27),
            target in -50i64..400,
            tolerance in 0u64..8,
            max_size in 0usize..8,
            limit in 0usize..40,
        ) {
            prop_assert_eq!(
                find_subsets(&amounts, target, tolerance, max_size, limit),
                find_subsets_reference(&amounts, target, tolerance, max_size, limit)
            );
        }
    }

    proptest! {
        #[test]
        fn matches_brute_force_on_small_pools(
            amounts in prop::collection::vec(1i64..60, 0..11),
            target in 0i64..250,
            tolerance in 0u64..6,
            max_size in 0usize..7,
        ) {
            let expected = brute_force(&amounts, target, tolerance, max_size);
            let found = find_subsets(&amounts, target, tolerance, max_size, 100_000);
            prop_assert_eq!(found.len(), expected.len());
            prop_assert_eq!(as_set(found), expected);
        }

        #[test]
        fn limited_results_are_a_valid_prefix_of_the_truth(
            amounts in prop::collection::vec(1i64..20, 0..11),
            target in 0i64..100,
            max_size in 2usize..6,
            limit in 0usize..12,
        ) {
            let expected = brute_force(&amounts, target, 0, max_size);
            let found = find_subsets(&amounts, target, 0, max_size, limit);
            prop_assert_eq!(found.len(), expected.len().min(limit));
            prop_assert!(found.iter().all(|s| expected.contains(s)));
            prop_assert_eq!(as_set(found.clone()).len(), found.len());
        }
    }
}
