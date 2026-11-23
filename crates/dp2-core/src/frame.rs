// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::FrameError;

/// Non-zero video frame dimensions in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dimensions {
    width: u32,
    height: u32,
}

impl Dimensions {
    /// Creates validated non-zero frame dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::ZeroDimension`] if either `width` or `height` is
    /// zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    ///
    /// let dims = Dimensions::new(640, 480)?;
    /// assert_eq!(dims.width(), 640);
    /// assert_eq!(dims.height(), 480);
    /// assert_eq!(dims.pixel_count(), 307_200);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    pub const fn new(width: u32, height: u32) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::ZeroDimension { width, height });
        }

        Ok(Self { width, height })
    }

    /// Returns the frame width in pixels.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    ///
    /// let dims = Dimensions::new(320, 240)?;
    /// assert_eq!(dims.width(), 320);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the frame height in pixels.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    ///
    /// let dims = Dimensions::new(320, 240)?;
    /// assert_eq!(dims.height(), 240);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the total number of pixels (`width * height`).
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    ///
    /// let dims = Dimensions::new(4, 3)?;
    /// assert_eq!(dims.pixel_count(), 12);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn pixel_count(self) -> usize {
        (self.width as usize) * (self.height as usize)
    }
}

/// Supported interleaved 8-bit pixel layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PixelFormat {
    /// 4 bytes per pixel: red, green, blue, alpha.
    Rgba8,
    /// 4 bytes per pixel: blue, green, red, alpha.
    Bgra8,
    /// 3 bytes per pixel: red, green, blue.
    Rgb8,
    /// 3 bytes per pixel: blue, green, red.
    Bgr8,
}

impl PixelFormat {
    /// Returns the byte width of one pixel in this format.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::PixelFormat;
    ///
    /// assert_eq!(PixelFormat::Rgba8.bytes_per_pixel(), 4);
    /// assert_eq!(PixelFormat::Rgb8.bytes_per_pixel(), 3);
    /// ```
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 => 4,
            Self::Rgb8 | Self::Bgr8 => 3,
        }
    }
}

/// A decoded video frame in host memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    dimensions: Dimensions,
    format: PixelFormat,
    stride: usize,
    timestamp_ns: u64,
    sequence: u64,
    data: Vec<u8>,
}

impl VideoFrame {
    /// Creates a video frame with an explicit row stride.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::StrideTooSmall`] if `stride` is narrower than one
    /// packed row, or [`FrameError::BufferTooSmall`] if `data` cannot
    /// contain `height` rows of `stride` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(2, 2)?;
    /// let frame = VideoFrame::new(dims, PixelFormat::Rgb8, 8, 1_000, 1, vec![0; 16])?;
    /// assert_eq!(frame.stride(), 8);
    /// assert_eq!(frame.sequence(), 1);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    pub fn new(
        dimensions: Dimensions,
        format: PixelFormat,
        stride: usize,
        timestamp_ns: u64,
        sequence: u64,
        data: Vec<u8>,
    ) -> Result<Self, FrameError> {
        let min_stride = (dimensions.width() as usize) * format.bytes_per_pixel();
        if stride < min_stride {
            return Err(FrameError::StrideTooSmall { stride, min_stride });
        }

        let expected = stride * (dimensions.height() as usize);
        if data.len() < expected {
            return Err(FrameError::BufferTooSmall {
                actual: data.len(),
                expected,
            });
        }

        Ok(Self {
            dimensions,
            format,
            stride,
            timestamp_ns,
            sequence,
            data,
        })
    }

    /// Creates a tightly packed video frame whose stride equals `width *
    /// bytes_per_pixel`.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::BufferTooSmall`] if `data` is shorter than a
    /// packed frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(2, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Rgba8, 0, 0, vec![255; 8])?;
    /// assert_eq!(frame.stride(), 8);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    pub fn packed(
        dimensions: Dimensions,
        format: PixelFormat,
        timestamp_ns: u64,
        sequence: u64,
        data: Vec<u8>,
    ) -> Result<Self, FrameError> {
        let stride = (dimensions.width() as usize) * format.bytes_per_pixel();
        Self::new(dimensions, format, stride, timestamp_ns, sequence, data)
    }

    /// Returns the frame dimensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(2, 2)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Rgb8, 0, 0, vec![0; 12])?;
    /// assert_eq!(frame.dimensions(), dims);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    /// Returns the frame pixel format.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Bgra8, 0, 0, vec![0; 4])?;
    /// assert_eq!(frame.format(), PixelFormat::Bgra8);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Returns the row stride in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(2, 1)?;
    /// let frame = VideoFrame::new(dims, PixelFormat::Rgb8, 8, 0, 0, vec![0; 8])?;
    /// assert_eq!(frame.stride(), 8);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Returns the capture timestamp in nanoseconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Rgb8, 42_000, 0, vec![0; 3])?;
    /// assert_eq!(frame.timestamp_ns(), 42_000);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }

    /// Returns the monotonically increasing frame sequence number.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Rgb8, 0, 7, vec![0; 3])?;
    /// assert_eq!(frame.sequence(), 7);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the raw pixel buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Rgb8, 0, 0, vec![1, 2, 3])?;
    /// assert_eq!(frame.data(), &[1, 2, 3]);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Converts the frame into a tightly packed `Rgb8` byte vector.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::packed(dims, PixelFormat::Bgra8, 0, 0, vec![30, 20, 10, 255])?;
    /// assert_eq!(frame.to_rgb8(), vec![10, 20, 30]);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn to_rgb8(&self) -> Vec<u8> {
        let width = self.dimensions.width() as usize;
        let height = self.dimensions.height() as usize;
        let bpp = self.format.bytes_per_pixel();
        let mut out = Vec::with_capacity(width * height * 3);

        for y in 0..height {
            let row = &self.data[y * self.stride..y * self.stride + width * bpp];
            for pixel in row.chunks_exact(bpp) {
                match self.format {
                    PixelFormat::Rgba8 | PixelFormat::Rgb8 => {
                        out.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
                    }
                    PixelFormat::Bgra8 | PixelFormat::Bgr8 => {
                        out.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                    }
                }
            }
        }

        out
    }

    /// Builds a tightly packed frame in `format` from tightly packed `Rgb8`
    /// pixels.
    ///
    /// Alpha channels in 4-byte formats are filled with `255`.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::BufferTooSmall`] if `rgb` contains fewer than
    /// `dimensions.pixel_count() * 3` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat, VideoFrame};
    ///
    /// let dims = Dimensions::new(1, 1)?;
    /// let frame = VideoFrame::from_rgb8(dims, PixelFormat::Bgra8, 10, 2, &[10, 20, 30])?;
    /// assert_eq!(frame.data(), &[30, 20, 10, 255]);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    pub fn from_rgb8(
        dimensions: Dimensions,
        format: PixelFormat,
        timestamp_ns: u64,
        sequence: u64,
        rgb: &[u8],
    ) -> Result<Self, FrameError> {
        let expected_rgb = dimensions.pixel_count() * 3;
        if rgb.len() < expected_rgb {
            return Err(FrameError::BufferTooSmall {
                actual: rgb.len(),
                expected: expected_rgb,
            });
        }

        let mut data = Vec::with_capacity(dimensions.pixel_count() * format.bytes_per_pixel());
        let (pixels, _) = rgb[..expected_rgb].as_chunks::<3>();
        for pixel in pixels {
            match format {
                PixelFormat::Rgba8 => data.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
                PixelFormat::Bgra8 => data.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]),
                PixelFormat::Rgb8 => data.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]),
                PixelFormat::Bgr8 => data.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]),
            }
        }

        Self::packed(dimensions, format, timestamp_ns, sequence, data)
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::zero_width(0, 10)]
    #[case::zero_height(10, 0)]
    #[case::both_zero(0, 0)]
    fn dimensions_rejects_zero(#[case] width: u32, #[case] height: u32) {
        assert_matches!(
            Dimensions::new(width, height),
            Err(FrameError::ZeroDimension { .. })
        );
    }

    #[test]
    fn stride_and_buffer_validation() -> Result<(), FrameError> {
        let dims = Dimensions::new(2, 2)?;
        assert_matches!(
            VideoFrame::new(dims, PixelFormat::Rgba8, 4, 0, 0, vec![0; 16]),
            Err(FrameError::StrideTooSmall {
                stride: 4,
                min_stride: 8
            })
        );
        assert_matches!(
            VideoFrame::new(dims, PixelFormat::Rgba8, 8, 0, 0, vec![0; 12]),
            Err(FrameError::BufferTooSmall {
                actual: 12,
                expected: 16
            })
        );
        Ok(())
    }

    #[rstest]
    #[case::rgba(PixelFormat::Rgba8, vec![10, 20, 30, 255])]
    #[case::bgra(PixelFormat::Bgra8, vec![30, 20, 10, 255])]
    #[case::rgb(PixelFormat::Rgb8, vec![10, 20, 30])]
    #[case::bgr(PixelFormat::Bgr8, vec![30, 20, 10])]
    fn rgb8_round_trip(
        #[case] format: PixelFormat,
        #[case] expected_bytes: Vec<u8>,
    ) -> Result<(), FrameError> {
        let dims = Dimensions::new(1, 1)?;
        let frame = VideoFrame::from_rgb8(dims, format, 123, 4, &[10, 20, 30])?;
        assert_eq!(frame.data(), expected_bytes.as_slice());
        assert_eq!(frame.to_rgb8(), vec![10, 20, 30]);
        Ok(())
    }

    #[test]
    fn to_rgb8_respects_row_padding() -> Result<(), FrameError> {
        let dims = Dimensions::new(2, 2)?;
        let data = vec![
            1, 2, 3, 4, 5, 6, 99, 99, // row 0 with 2 padding bytes
            7, 8, 9, 10, 11, 12, 88, 88, // row 1 with 2 padding bytes
        ];
        let frame = VideoFrame::new(dims, PixelFormat::Rgb8, 8, 0, 0, data)?;
        assert_eq!(frame.to_rgb8(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        Ok(())
    }
}
