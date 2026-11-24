// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use dp2_core::{DepthError, DepthEstimator, ModelDescriptor, ModelFactory, ModelId};

use crate::{DepthPro2Config, FocalLengthPolicy, OnnxDepthPro2Estimator};

/// A configured Depth Pro 2 ONNX model entry stored in [`ModelCatalog`].
#[derive(Debug, Clone, PartialEq)]
pub struct ModelEntry {
    /// Human- and machine-readable metadata for UI and Varlink listing.
    pub descriptor: ModelDescriptor,
    /// ONNX Runtime configuration used to instantiate this model.
    pub config: DepthPro2Config,
}

/// Catalog of Depth Pro 2 ONNX models implementing [`ModelFactory`] for runtime
/// model-size switching.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    entries: BTreeMap<ModelId, ModelEntry>,
}

impl ModelCatalog {
    /// Creates an empty model catalog.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::ModelFactory;
    /// use dp2_depth::ModelCatalog;
    ///
    /// let catalog = ModelCatalog::new();
    /// assert!(catalog.list_models().is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a single [`ModelEntry`], replacing any entry with the same id.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{ModelDescriptor, ModelFactory, ModelId};
    /// use dp2_depth::{DepthPro2Config, ModelCatalog, ModelEntry};
    ///
    /// let mut catalog = ModelCatalog::new();
    /// catalog.register(ModelEntry {
    ///     descriptor: ModelDescriptor {
    ///         id: ModelId::new("s-stream4"),
    ///         name: "Depth Pro 2 S (Streaming)".into(),
    ///         description: "Temporally consistent 24M parameter model".into(),
    ///         size_label: "S (24M)".into(),
    ///         available: true,
    ///     },
    ///     config: DepthPro2Config::new("models/depthpro2-s-stream4.onnx"),
    /// });
    /// assert_eq!(catalog.list_models().len(), 1);
    /// ```
    pub fn register(&mut self, entry: ModelEntry) {
        self.entries.insert(entry.descriptor.id.clone(), entry);
    }

    /// Infers a [`ModelEntry`] from a model path and optional explicit id.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use dp2_core::ModelId;
    /// use dp2_depth::{DepthPro2Config, ModelCatalog};
    ///
    /// let entry = ModelCatalog::entry_from_config(
    ///     None,
    ///     DepthPro2Config::new(Path::new("models/depthpro2-b-stream4.onnx")),
    /// );
    /// assert_eq!(entry.descriptor.id, ModelId::new("b-stream4"));
    /// assert_eq!(entry.descriptor.size_label, "B (97M)");
    /// ```
    #[must_use]
    pub fn entry_from_config(id: Option<ModelId>, config: DepthPro2Config) -> ModelEntry {
        let stem = config
            .model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model");
        let lower = stem.to_ascii_lowercase();
        let inferred_id = id.unwrap_or_else(|| {
            let trimmed = lower
                .strip_prefix("depthpro2-")
                .or_else(|| lower.strip_prefix("depth-pro-2-"))
                .unwrap_or(&lower);
            ModelId::new(trimmed)
        });

        let (name, size_label, description) = classify_model_stem(&lower, inferred_id.as_str());
        let available = config.model_path.is_file();

        ModelEntry {
            descriptor: ModelDescriptor {
                id: inferred_id,
                name,
                description,
                size_label,
                available,
            },
            config,
        }
    }

    /// Scans `dir` for `*.onnx` files and registers each discovered Depth Pro 2
    /// model using the shared ONNX Runtime settings.
    ///
    /// Non-existent directories are treated as empty catalogs so
    /// socket-activated services can start cleanly before weights are
    /// downloaded.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::ModelFactory;
    /// use dp2_depth::{FocalLengthPolicy, ModelCatalog};
    ///
    /// let dir = tempfile::tempdir()?;
    /// std::fs::write(dir.path().join("depthpro2-s-stream4.onnx"), b"")?;
    /// std::fs::write(dir.path().join("depthpro2-b-stream4.onnx"), b"")?;
    /// let catalog =
    ///     ModelCatalog::from_directory(dir.path(), None, &FocalLengthPolicy::default(), None);
    /// assert_eq!(catalog.list_models().len(), 2);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub fn from_directory(
        dir: &Path,
        ort_dylib_path: Option<&Path>,
        focal_length: &FocalLengthPolicy,
        intra_threads: Option<usize>,
    ) -> Self {
        let mut catalog = Self::new();
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return catalog;
        };

        let mut paths: Vec<PathBuf> = read_dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("onnx"))
            })
            .collect();
        paths.sort();

        for path in paths {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if stem.contains("focal") && !stem.contains("auto") {
                continue;
            }
            let config = DepthPro2Config {
                model_path: path,
                ort_dylib_path: ort_dylib_path.map(Path::to_path_buf),
                focal_length: focal_length.clone(),
                intra_threads,
            };
            catalog.register(Self::entry_from_config(None, config));
        }

        catalog
    }

    /// Returns the first available model identifier in deterministic order, if
    /// any.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_depth::ModelCatalog;
    ///
    /// let catalog = ModelCatalog::new();
    /// assert_eq!(catalog.first_available_id(), None);
    /// ```
    #[must_use]
    pub fn first_available_id(&self) -> Option<ModelId> {
        self.entries
            .values()
            .find(|e| e.descriptor.available)
            .map(|e| e.descriptor.id.clone())
    }
}

impl ModelFactory for ModelCatalog {
    fn list_models(&self) -> Vec<ModelDescriptor> {
        self.entries
            .values()
            .map(|e| e.descriptor.clone())
            .collect()
    }

    fn load_model(&self, id: &ModelId) -> Result<Box<dyn DepthEstimator>, DepthError> {
        let entry = self
            .entries
            .get(id)
            .ok_or_else(|| DepthError::Inference(format!("unknown Depth Pro 2 model '{id}'")))?;
        let estimator = OnnxDepthPro2Estimator::from_config(&entry.config)?;
        Ok(Box::new(estimator))
    }
}

fn classify_model_stem(lower: &str, id: &str) -> (String, String, String) {
    let tokens: Vec<&str> = lower.split(['-', '_']).collect();
    let streaming = tokens.iter().any(|t| t.starts_with("stream"));
    let suffix = if streaming { " (Streaming)" } else { "" };

    if tokens.contains(&"s") {
        (
            format!("Depth Pro 2 S{suffix}"),
            "S (24M)".into(),
            "Fast 24M-parameter Depth Pro 2 model".into(),
        )
    } else if tokens.contains(&"b") {
        (
            format!("Depth Pro 2 B{suffix}"),
            "B (97M)".into(),
            "Balanced 97M-parameter Depth Pro 2 model".into(),
        )
    } else if tokens.contains(&"l") {
        (
            format!("Depth Pro 2 L{suffix}"),
            "L (334M)".into(),
            "High-detail 334M-parameter Depth Pro 2 model".into(),
        )
    } else if tokens.contains(&"h") {
        (
            format!("Depth Pro 2 H+{suffix}"),
            "H+ (872M)".into(),
            "Highest-accuracy 872M-parameter Depth Pro 2 model".into(),
        )
    } else {
        (
            format!("Depth Pro 2 ({id})"),
            "Custom".into(),
            "Custom Depth Pro 2 ONNX model".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn from_directory_discovers_models_and_skips_standalone_focal() -> Result<(), std::io::Error> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("depthpro2-s-stream4.onnx"), b"stub")?;
        std::fs::write(dir.path().join("depthpro2-b.onnx"), b"stub")?;
        std::fs::write(dir.path().join("focal.onnx"), b"stub")?;
        std::fs::write(dir.path().join("README.txt"), b"ignore")?;

        let catalog =
            ModelCatalog::from_directory(dir.path(), None, &FocalLengthPolicy::default(), None);
        let ids: Vec<ModelId> = catalog.list_models().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![ModelId::new("b"), ModelId::new("s-stream4")]);
        assert_eq!(catalog.first_available_id(), Some(ModelId::new("b")));
        Ok(())
    }
}
