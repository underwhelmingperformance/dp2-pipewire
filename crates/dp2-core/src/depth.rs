// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::{DepthError, Dimensions};

/// Validated inclusive metric depth range in metres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DepthRange {
    min_meters: f32,
    max_meters: f32,
}

impl DepthRange {
    /// Creates a validated metric depth range where `0 < min_meters <
    /// max_meters`.
    ///
    /// # Errors
    ///
    /// Returns [`DepthError::InvalidRange`] if either bound is non-finite,
    /// non-positive, or if `min_meters >= max_meters`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::DepthRange;
    ///
    /// let range = DepthRange::new(0.5, 10.0)?;
    /// assert_eq!(range.min_meters(), 0.5);
    /// assert_eq!(range.max_meters(), 10.0);
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    pub fn new(min_meters: f32, max_meters: f32) -> Result<Self, DepthError> {
        if !min_meters.is_finite()
            || !max_meters.is_finite()
            || min_meters <= 0.0
            || max_meters <= min_meters
        {
            return Err(DepthError::InvalidRange {
                min_meters,
                max_meters,
            });
        }

        Ok(Self {
            min_meters,
            max_meters,
        })
    }

    /// Returns the near bound in metres.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::DepthRange;
    ///
    /// let range = DepthRange::new(0.25, 5.0)?;
    /// assert_eq!(range.min_meters(), 0.25);
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    #[must_use]
    pub const fn min_meters(self) -> f32 {
        self.min_meters
    }

    /// Returns the far bound in metres.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::DepthRange;
    ///
    /// let range = DepthRange::new(0.25, 5.0)?;
    /// assert_eq!(range.max_meters(), 5.0);
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    #[must_use]
    pub const fn max_meters(self) -> f32 {
        self.max_meters
    }
}

/// Per-pixel metric depth estimate in metres along the camera viewing
/// direction.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthMap {
    dimensions: Dimensions,
    values_meters: Vec<f32>,
    focal_length_px: f32,
}

impl DepthMap {
    /// Creates a validated metric depth map.
    ///
    /// # Errors
    ///
    /// Returns [`DepthError::LengthMismatch`] if `values_meters.len()` does not
    /// equal `dimensions.pixel_count()`, or
    /// [`DepthError::InvalidFocalLength`] if `focal_length_px` is not
    /// finite and positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, Dimensions};
    ///
    /// let dims = Dimensions::new(2, 1)?;
    /// let depth = DepthMap::new(dims, vec![1.0, 2.0], 500.0)?;
    /// assert_eq!(depth.focal_length_px(), 500.0);
    /// assert_eq!(depth.values_meters(), &[1.0, 2.0]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(
        dimensions: Dimensions,
        values_meters: Vec<f32>,
        focal_length_px: f32,
    ) -> Result<Self, DepthError> {
        let expected = dimensions.pixel_count();
        if values_meters.len() != expected {
            return Err(DepthError::LengthMismatch {
                actual: values_meters.len(),
                expected,
            });
        }

        if !focal_length_px.is_finite() || focal_length_px <= 0.0 {
            return Err(DepthError::InvalidFocalLength { focal_length_px });
        }

        Ok(Self {
            dimensions,
            values_meters,
            focal_length_px,
        })
    }

    /// Returns the dimensions of the depth map.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, Dimensions};
    ///
    /// let dims = Dimensions::new(2, 2)?;
    /// let depth = DepthMap::new(dims, vec![1.0; 4], 500.0)?;
    /// assert_eq!(depth.dimensions(), dims);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    /// Returns the row-major metric depth values in metres.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, Dimensions};
    ///
    /// let dims = Dimensions::new(2, 1)?;
    /// let depth = DepthMap::new(dims, vec![1.5, 3.0], 500.0)?;
    /// assert_eq!(depth.values_meters(), &[1.5, 3.0]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn values_meters(&self) -> &[f32] {
        &self.values_meters
    }

    /// Returns the horizontal focal length in pixels associated with this depth
    /// map.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, Dimensions};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let depth = DepthMap::new(dims, vec![2.0], 612.5)?;
    /// assert_eq!(depth.focal_length_px(), 612.5);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn focal_length_px(&self) -> f32 {
        self.focal_length_px
    }

    /// Returns the minimum and maximum finite positive depth values in metres,
    /// if any exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, Dimensions};
    ///
    /// let dims = Dimensions::new(3, 1)?;
    /// let depth = DepthMap::new(dims, vec![1.0, f32::INFINITY, 4.0], 500.0)?;
    /// assert_eq!(depth.finite_bounds(), Some((1.0, 4.0)));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn finite_bounds(&self) -> Option<(f32, f32)> {
        let mut min_v = f32::INFINITY;
        let mut max_v = f32::NEG_INFINITY;

        for &d in &self.values_meters {
            if d.is_finite() && d > 0.0 {
                min_v = min_v.min(d);
                max_v = max_v.max(d);
            }
        }

        if min_v.is_finite() && max_v.is_finite() {
            Some((min_v, max_v))
        } else {
            None
        }
    }

    /// Normalises inverse depth (`1 / depth`) into `[0.0, 1.0]`, where `1.0` is
    /// nearest and `0.0` is farthest (including infinite or invalid sky
    /// pixels).
    ///
    /// When `range` is `None`, bounds are derived from the finite positive
    /// depths in this map.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMap, DepthRange, Dimensions};
    ///
    /// let dims = Dimensions::new(3, 1)?;
    /// let depth = DepthMap::new(dims, vec![1.0, 2.0, f32::INFINITY], 500.0)?;
    /// let range = DepthRange::new(1.0, 4.0)?;
    /// let norm = depth.normalised_inverse_depth(Some(range));
    /// assert!((norm[0] - 1.0).abs() < 1e-6);
    /// assert!((norm[1] - (1.0 / 3.0)).abs() < 1e-6);
    /// assert_eq!(norm[2], 0.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn normalised_inverse_depth(&self, range: Option<DepthRange>) -> Vec<f32> {
        let (inv_min, inv_max) = match range {
            Some(r) => (1.0 / r.max_meters(), 1.0 / r.min_meters()),
            None => match self.finite_bounds() {
                Some((min_d, max_d)) => (1.0 / max_d, 1.0 / min_d),
                None => return vec![0.0; self.values_meters.len()],
            },
        };

        let span = inv_max - inv_min;
        if span <= f32::EPSILON {
            return self
                .values_meters
                .iter()
                .map(|&d| if d.is_finite() && d > 0.0 { 0.5 } else { 0.0 })
                .collect();
        }

        self.values_meters
            .iter()
            .map(|&d| {
                if !d.is_finite() || d <= 0.0 {
                    return 0.0;
                }
                ((1.0 / d - inv_min) / span).clamp(0.0, 1.0)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::zero_min(0.0, 5.0)]
    #[case::negative_min(-1.0, 5.0)]
    #[case::inverted(5.0, 2.0)]
    #[case::equal(2.0, 2.0)]
    #[case::nan(f32::NAN, 2.0)]
    #[case::inf(1.0, f32::INFINITY)]
    fn depth_range_rejects_invalid_bounds(#[case] min_m: f32, #[case] max_m: f32) {
        assert_matches!(
            DepthRange::new(min_m, max_m),
            Err(DepthError::InvalidRange { .. })
        );
    }

    #[test]
    fn depth_map_validates_length_and_focal_length() -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 2)?;
        assert_matches!(
            DepthMap::new(dims, vec![1.0; 3], 500.0),
            Err(DepthError::LengthMismatch {
                actual: 3,
                expected: 4
            })
        );
        assert_matches!(
            DepthMap::new(dims, vec![1.0; 4], 0.0),
            Err(DepthError::InvalidFocalLength { .. })
        );
        Ok(())
    }

    #[test]
    fn flat_depth_map_normalises_finite_pixels_to_midpoint()
    -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 1)?;
        let depth = DepthMap::new(dims, vec![2.0, f32::INFINITY], 500.0)?;
        assert_eq!(depth.normalised_inverse_depth(None), vec![0.5, 0.0]);
        Ok(())
    }
}
