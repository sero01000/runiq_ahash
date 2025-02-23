//! Module containing filter implementations for Runiq.
//!
//! Each structure in this module has different filtering properties
//! and should be chosen based on the specific use case of the user.
//!
//! Please see the struct documentation for further information on
//! each filter, including their runtime characteristics.
use clap::*;
use fnv::FnvHashSet;
use scalable_bloom_filter::ScalableBloomFilter;
// use twox_hash::XxHash64;
use ahash::AHasher;

use std::collections::HashSet;
use std::hash::Hasher;
use std::str::FromStr;

/// Enum to store all possible variants of filters.
///
/// This will implement the `Into` trait in order to create a new
/// boxed filter from a filter kind to keep conversion contained.
#[doc(hidden)]
#[derive(ArgEnum, Copy, Clone, Debug)]
pub enum FilterKind {
    Sorted,
    Digest,
    Naive,
    Bloom,
}

// FromStr impl for `FilterKind` for arg parsing.
impl FromStr for FilterKind {
    type Err = String;

    /// Converts an input string to a `FilterKind` value.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        for variant in Self::value_variants() {
            if variant.to_possible_value().unwrap().matches(s, false) {
                return Ok(*variant);
            }
        }
        Err(format!("Invalid variant: {}", s))
    }
}

/// Trait for any type which can be used to filter unique values.
///
/// The filter only supports a single operation to detect a unique
/// which will provide the ability to check/insert in a single operation.
pub trait Filter {
    /// Create a new instance using defaults.
    fn new() -> Self
    where
        Self: Sized;

    /// Detects a unique value.
    ///
    /// Return values are booleans to represent whether the value
    /// was added to the internal filter or not (i.e. `true` if
    /// this is the first time the value has been seen).
    fn detect(&mut self, input: &[u8]) -> bool;
}

/// Implement `From` to convert to `Filter`.
impl From<FilterKind> for Box<dyn Filter> {
    /// Creates a new `Filter` type based on the enum value.
    fn from(kind: FilterKind) -> Self {
        match kind {
            FilterKind::Sorted => Box::new(SortedFilter::new()),
            FilterKind::Digest => Box::new(DigestFilter::new()),
            FilterKind::Naive => Box::new(NaiveFilter::new()),
            FilterKind::Bloom => Box::new(BloomFilter::new()),
        }
    }
}

/// Basic filter implementation backed by a `HashSet`.
///
/// This implementation offers nothing more than abstraction over
/// using a `HashSet` directly, and will store raw values in the
/// set. Naturally this means that memory will not be particularly
/// efficient, but it is guaranteed to be completely accurate when
/// calculating unique collisions in inputs.
#[derive(Clone, Debug, Default)]
pub struct NaiveFilter {
    inner: HashSet<Vec<u8>>,
}

/// Implement all trait methods.
impl Filter for NaiveFilter {
    /// Creates a new `NaiveFilter`.
    fn new() -> NaiveFilter {
        NaiveFilter::default()
    }

    /// Detects a unique value.
    #[inline]
    fn detect(&mut self, input: &[u8]) -> bool {
        self.inner.insert(input.to_vec())
    }
}

/// Digest filter implementation backed by a `HashSet`.
///
/// This implementation offers much better memory efficiency when
/// compared to the `NaiveFilter` due to the fact that raw values
/// are hashed to `usize` values before being stored in the set.
///
/// It's also a little faster due to some improved efficiency
/// when comparing values in the set itself, but it's not of any
/// real consequence and is barely noticeable.
#[derive(Clone, Debug, Default)]
pub struct DigestFilter {
    inner: FnvHashSet<u64>,
}

/// Implement all trait methods.
impl Filter for DigestFilter {
    /// Creates a new `DigestFilter`.
    fn new() -> DigestFilter {
        DigestFilter::default()
    }

    /// Detects a unique value.
    #[inline]
    fn detect(&mut self, input: &[u8]) -> bool {
        // insert as a hashed digest
        self.inner.insert(hash(input))
    }
}

/// Uniq filter implementation to only remove consecutive duplicates.
///
/// This is the fastest filter (although not by much), and the best in
/// terms of memory efficiency as it only requires a single value stored
/// in the filter memory at once. It operates in the same was as the Unix
/// `uniq` utility, and thus requires your data be sorted prior to any
/// execution.
///
/// Remember that repeatedly running Runiq on the same input would be
/// a good candidate for sorting your data initially and then making
/// use of this filter to optimize memory usage going forward.
#[derive(Clone, Debug)]
pub struct SortedFilter {
    inner: Vec<u8>,
}

/// Implement all trait methods.
impl Filter for SortedFilter {
    /// Creates a new `SortedFilter`.
    fn new() -> SortedFilter {
        SortedFilter { inner: Vec::new() }
    }

    /// Detects a unique value.
    #[inline]
    fn detect(&mut self, input: &[u8]) -> bool {
        // check for consec collision
        if input == &self.inner[..] {
            return false;
        }

        // overwrite the previous value
        self.inner = input.to_vec();
        true
    }
}

/// Bitset filter backed by a scalable Bloom Filter.
///
/// This filter operates with the least amount of memory, with a cost
/// of speed (roughly 60-70% of the speed of the `DigestFilter`, using
/// only 25% of the memory).
///
/// The backing bloom filter initializes with `1e6` bits by default, with
/// `1e-7` probability of collisions. This is roughly comparable to the
/// collision rate of the digest filter, so this should be chosen when
/// memory is critical.
#[derive(Debug)]
pub struct BloomFilter {
    inner: ScalableBloomFilter<u64>,
}

/// Implement all trait methods.
impl Filter for BloomFilter {
    /// Creates a new `BloomFilter`.
    fn new() -> BloomFilter {
        BloomFilter {
            inner: ScalableBloomFilter::new(1_000_000, 1e-8),
        }
    }

    /// Detects a unique value.
    #[inline]
    fn detect(&mut self, input: &[u8]) -> bool {
        // create a digest from the input
        let digest = hash(input);

        // short circuit if duplicated
        if self.inner.contains(&digest) {
            return false;
        }

        // insert on duplicates
        self.inner.insert(&digest);
        true
    }
}

/// Small hash binding around `Hasher`.
fn hash(input: &[u8]) -> u64 {
    // create a new default hasher
    // let mut hasher = XxHash64::default();
    let mut hasher = AHasher::default();

    // write the bytes to the hasher
    hasher.write(input);

    // finish the hash
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_filter_detection() {
        let mut filter = NaiveFilter::new();

        let ins1 = filter.detect(b"input1");
        let ins2 = filter.detect(b"input1");

        assert!(ins1);
        assert!(!ins2);
    }

    #[test]
    fn digest_filter_detection() {
        let mut filter = DigestFilter::new();

        let ins1 = filter.detect(b"input1");
        let ins2 = filter.detect(b"input1");

        assert!(ins1);
        assert!(!ins2);
    }

    #[test]
    fn sorted_filter_detection() {
        let mut filter = SortedFilter::new();

        let ins1 = filter.detect(b"input1");
        let ins2 = filter.detect(b"input1");
        let ins3 = filter.detect(b"input2");
        let ins4 = filter.detect(b"input1");

        assert!(ins1);
        assert!(!ins2);
        assert!(ins3);
        assert!(ins4);
    }

    #[test]
    fn bloom_filter_detection() {
        let mut filter = BloomFilter::new();

        let ins1 = filter.detect(b"input1");
        let ins2 = filter.detect(b"input1");

        assert!(ins1);
        assert!(!ins2);
    }
}
