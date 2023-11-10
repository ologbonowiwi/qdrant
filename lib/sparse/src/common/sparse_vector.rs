use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError, ValidationErrors};

use crate::common::types::{DimId, DimWeight};

/// Sparse vector structure
///
/// expects:
/// - indices to be unique
/// - indices and values to be the same length
/// - indices and values to be non-empty
/// - indices to be sorted TODO(sparse) this is not validated yet
#[derive(Debug, PartialEq, Clone, Default, Serialize, Deserialize)]
pub struct SparseVector {
    pub indices: Vec<DimId>,
    pub values: Vec<DimWeight>,
}

impl SparseVector {
    pub fn new(indices: Vec<DimId>, values: Vec<DimWeight>) -> Result<Self, ValidationErrors> {
        let vector = SparseVector { indices, values };
        vector.validate()?;
        Ok(vector)
    }

    /// Score this vector against another vector using dot product.
    /// Warning: Expects both vectors to be sorted by indices.
    pub fn score(&self, other: &SparseVector) -> f32 {
        let mut score = 0.0;
        let mut i = 0;
        let mut j = 0;
        while i < self.indices.len() && j < other.indices.len() {
            match self.indices[i].cmp(&other.indices[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    score += self.values[i] * other.values[j];
                    i += 1;
                    j += 1;
                }
            }
        }
        score
    }

    /// Check if this vector overlaps with another vector.
    /// Warning: Expects both vectors to be sorted by indices.
    pub fn overlaps(&self, other: &SparseVector) -> bool {
        let mut i = 0;
        let mut j = 0;
        while i < self.indices.len() && j < other.indices.len() {
            match self.indices[i].cmp(&other.indices[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }
}
impl TryFrom<Vec<(i32, f64)>> for SparseVector {
    type Error = ValidationErrors;

    fn try_from(tuples: Vec<(i32, f64)>) -> Result<Self, Self::Error> {
        let mut indices = Vec::with_capacity(tuples.len());
        let mut values = Vec::with_capacity(tuples.len());
        for (i, w) in tuples {
            indices.push(i as u32);
            values.push(w as f32);
        }
        SparseVector::new(indices, values)
    }
}

impl Validate for SparseVector {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::default();

        if self.indices.is_empty() {
            errors.add("indices", ValidationError::new("must be non-empty"));
        }
        if self.values.is_empty() {
            errors.add("values", ValidationError::new("must be non-empty"));
        }
        if self.indices.len() != self.values.len() {
            errors.add(
                "values",
                ValidationError::new("must be the same length as indices"),
            );
        }
        if self.indices.windows(2).any(|w| w[0] == w[1]) {
            errors.add("indices", ValidationError::new("must be unique"));
        }
        if self.indices.windows(2).any(|w| w[0] > w[1]) {
            errors.add("indices", ValidationError::new("must be sorted"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_aligned_same_size() {
        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(v1.score(&v2), 14.0);
    }

    #[test]
    fn test_score_not_aligned_same_size() {
        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![2, 3, 4], vec![2.0, 3.0, 4.0]).unwrap();
        assert_eq!(v1.score(&v2), 13.0);
    }

    #[test]
    fn test_score_aligned_different_size() {
        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![1, 2, 3, 4], vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(v1.score(&v2), 14.0);
    }

    #[test]
    fn test_score_not_aligned_different_size() {
        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![2, 3, 4, 5], vec![2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(v1.score(&v2), 13.0);
    }

    #[test]
    fn validation_test() {
        let empty_indices = SparseVector::new(vec![], vec![1.0, 2.0, 3.0]);
        assert!(empty_indices.is_err());

        let empty_values = SparseVector::new(vec![1, 2, 3], vec![]);
        assert!(empty_values.is_err());

        let different_length = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0]);
        assert!(different_length.is_err());

        let not_sorted = SparseVector::new(vec![1, 3, 2], vec![1.0, 2.0, 3.0]);
        assert!(not_sorted.is_err());

        let not_unique = SparseVector::new(vec![1, 2, 2], vec![1.0, 2.0, 3.0]);
        assert!(not_unique.is_err());
    }

    #[test]
    fn overlaps_test() {
        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![2, 3, 4], vec![2.0, 3.0, 4.0]).unwrap();
        assert!(v1.overlaps(&v2));

        let v1 = SparseVector::new(vec![1, 2, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![4, 5, 6], vec![2.0, 3.0, 4.0]).unwrap();
        assert!(!v1.overlaps(&v2));

        let v1 = SparseVector::new(vec![2, 3], vec![2.0, 3.0]).unwrap();
        let v2 = SparseVector::new(vec![3, 4, 5], vec![2.0, 3.0, 4.0]).unwrap();
        assert!(v1.overlaps(&v2));

        let v1 = SparseVector::new(vec![3, 4, 5], vec![2.0, 3.0, 4.0]).unwrap();
        let v2 = SparseVector::new(vec![2, 3], vec![2.0, 3.0]).unwrap();
        assert!(v1.overlaps(&v2));
    }
}
