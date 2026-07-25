//! Metadata for the read-only SenseVoice model bundled with Hachimi.

use std::{fs, path::Path};

pub const DEFAULT_SPEECH_MODEL_NAME: &str = "SenseVoice-Small INT8";
pub const DEFAULT_SPEECH_MODEL_LANGUAGES: &[&str] = &["zh-CN", "en-US", "ja-JP", "ko-KR", "yue"];

pub fn installed_size(model_dir: &Path) -> u32 {
    ["model.int8.onnx", "tokens.txt"]
        .into_iter()
        .filter_map(|name| fs::metadata(model_dir.join(name)).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>()
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_size_counts_only_runtime_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        fs::write(directory.path().join("model.int8.onnx"), [0_u8; 4]).expect("model");
        fs::write(directory.path().join("tokens.txt"), [0_u8; 3]).expect("tokens");
        fs::write(directory.path().join("manifest.json"), [0_u8; 20]).expect("manifest");
        assert_eq!(installed_size(directory.path()), 7);
    }
}
