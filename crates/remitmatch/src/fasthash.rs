//! A small multiplicative hasher for the engine's internal maps and sets.
//!
//! It is only safe for keys that the engine generates itself: row indices, anchors and
//! pairs of internal ids. Its constant is fixed and it is not seeded, so anyone who can
//! choose the keys can make them collide and turn a map quadratic. Keys that are caller or
//! bank text, or are derived directly from it (ids, invoice numbers and their substrings,
//! names, scope and currency), must use the standard `HashMap` and `HashSet` with the
//! default randomly seeded hasher. The engine never decides anything from a map's iteration
//! order, which the random seed would otherwise change between runs.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplier of the hash, an odd constant with well-spread bits.
const MULTIPLIER: u64 = 0x517c_c1b7_2722_0a95;

/// A hasher that folds the input a word at a time: rotate, xor, multiply.
#[derive(Default, Clone, Copy)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    /// Folds one word into the state.
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(MULTIPLIER);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            // The length sits in the top byte, so "ab" and "ab\0" differ.
            let mut word = (rest.len() as u64) << 56;
            for (place, &byte) in rest.iter().enumerate() {
                word |= u64::from(byte) << (8 * place);
            }
            self.add(word);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        // The table uses the low bits for the slot and the top bits as a tag; spread both.
        let mixed = (self.0 ^ (self.0 >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        mixed ^ (mixed >> 29)
    }
}

/// A map with the fast hasher.
pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// A set with the fast hasher.
pub(crate) type FastSet<T> = HashSet<T, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use std::hash::{BuildHasher, Hash};

    use super::*;

    fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
        BuildHasherDefault::<FastHasher>::default().hash_one(value)
    }

    #[test]
    fn texts_that_differ_in_any_byte_hash_differently() {
        let base = "INV-2024-000123";
        for end in 1..=base.len() {
            let text = &base[..end];
            for position in 0..end {
                let mut changed = text.as_bytes().to_vec();
                changed[position] ^= 1;
                let changed = String::from_utf8(changed).unwrap_or_default();
                assert_ne!(
                    hash_of(text),
                    hash_of(changed.as_str()),
                    "{text} at {position}"
                );
            }
        }
        assert_ne!(hash_of("ab"), hash_of("ab\0"), "the length takes part");
    }

    #[test]
    fn equal_values_hash_equally_and_numbers_spread() {
        assert_eq!(hash_of("same"), hash_of(&String::from("same")));
        assert_eq!(hash_of(&(3_usize, 4_usize)), hash_of(&(3_usize, 4_usize)));
        let slots: std::collections::BTreeSet<u64> =
            (0_usize..4096).map(|n| hash_of(&n) & 1023).collect();
        assert!(
            slots.len() > 900,
            "{} of 1024 low-bit slots used",
            slots.len()
        );
    }

    #[test]
    fn a_fast_map_and_set_behave_like_the_std_ones() {
        let ids: Vec<String> = (0..20_000).map(|n| format!("id-{n:06}")).collect();
        let mut map = FastMap::default();
        let mut set = FastSet::default();
        for (n, id) in ids.iter().enumerate() {
            assert!(set.insert(id.as_str()));
            map.insert(id.as_str(), n);
        }
        assert!(ids.iter().all(|id| !set.insert(id.as_str())));
        assert!(ids.iter().enumerate().all(|(n, id)| map[id.as_str()] == n));
    }
}
