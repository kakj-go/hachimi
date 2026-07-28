// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/core/src/{agents_md,agents_md_manager}.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: Checkout-bound Workspace Host reads, explicit source
// records, a deterministic byte budget, and StepContext revision hashing.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use hachimi_workspace::{
    WorkspaceErrorCode, WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::AgentInstructionLayer;

pub const DEFAULT_AGENTS_MD_BUDGET: usize = 32 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const AGENTS_FILE: &str = "AGENTS.md";
const OVERRIDE_FILE: &str = "AGENTS.override.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsMdSnapshot {
    pub layers: Vec<AgentInstructionLayer>,
    pub revision: String,
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum AgentsMdError {
    #[error("AGENTS.md cwd must be a checkout-relative directory")]
    InvalidCwd,
    #[error("Workspace Host failed while reading AGENTS.md: {0}")]
    Host(String),
    #[error("Workspace Host returned an invalid AGENTS.md response")]
    InvalidResponse,
}

pub type AgentsReadFuture =
    Pin<Box<dyn Future<Output = Result<Option<AgentsFile>, AgentsMdError>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsFile {
    pub path: String,
    pub markdown: String,
    pub content_hash: String,
}

pub trait AgentsFileReader: Send + Sync {
    fn read(&self, path: String, cancellation: CancellationToken) -> AgentsReadFuture;
}

impl AgentsFileReader for WorkspaceHostClient {
    fn read(&self, path: String, cancellation: CancellationToken) -> AgentsReadFuture {
        let host = self.clone();
        Box::pin(async move {
            match host
                .execute(
                    WorkspaceOperation::ReadFile { path: path.clone() },
                    READ_TIMEOUT,
                    cancellation,
                )
                .await
            {
                Ok(WorkspaceOutput::File {
                    path,
                    content,
                    sha256,
                    ..
                }) => Ok(Some(AgentsFile {
                    path,
                    markdown: content,
                    content_hash: sha256,
                })),
                Ok(_) => Err(AgentsMdError::InvalidResponse),
                Err(error) if error.code == WorkspaceErrorCode::NotFound => Ok(None),
                Err(error) => Err(AgentsMdError::Host(format!(
                    "{:?}: {}",
                    error.code, error.message
                ))),
            }
        })
    }
}

#[derive(Clone)]
pub struct AgentsMdLoader {
    reader: Arc<dyn AgentsFileReader>,
    max_bytes: usize,
}

impl std::fmt::Debug for AgentsMdLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentsMdLoader")
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

impl AgentsMdLoader {
    #[must_use]
    pub fn new(reader: Arc<dyn AgentsFileReader>) -> Self {
        Self {
            reader,
            max_bytes: DEFAULT_AGENTS_MD_BUDGET,
        }
    }

    #[must_use]
    pub fn with_budget(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub async fn load(
        &self,
        cwd: &str,
        cancellation: CancellationToken,
    ) -> Result<AgentsMdSnapshot, AgentsMdError> {
        let directories = relative_ancestors(cwd)?;
        let mut layers = Vec::new();
        let mut remaining = self.max_bytes;
        let mut truncated = false;
        for directory in directories {
            if cancellation.is_cancelled() {
                return Err(AgentsMdError::Host("cancelled".into()));
            }
            let override_path = join_relative(&directory, OVERRIDE_FILE);
            let normal_path = join_relative(&directory, AGENTS_FILE);
            let file = match self
                .reader
                .read(override_path, cancellation.child_token())
                .await?
            {
                Some(file) => Some(file),
                None => {
                    self.reader
                        .read(normal_path, cancellation.child_token())
                        .await?
                }
            };
            let Some(file) = file else { continue };
            if remaining == 0 {
                truncated = true;
                break;
            }
            let (markdown, clipped) = bounded_utf8(&file.markdown, remaining);
            remaining = remaining.saturating_sub(markdown.len());
            truncated |= clipped;
            layers.push(AgentInstructionLayer {
                relative_directory: directory.clone(),
                source_path: file.path,
                content_hash: file.content_hash,
                markdown,
            });
            if clipped {
                break;
            }
        }
        let revision = hash_layers(&layers, truncated);
        Ok(AgentsMdSnapshot {
            layers,
            revision,
            truncated,
        })
    }
}

fn relative_ancestors(cwd: &str) -> Result<Vec<String>, AgentsMdError> {
    let normalized = cwd.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.split('/').any(|part| part == "..")
    {
        return Err(AgentsMdError::InvalidCwd);
    }
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    let mut directories = vec![String::new()];
    let mut current = String::new();
    for part in parts {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        directories.push(current.clone());
    }
    Ok(directories)
}

fn join_relative(directory: &str, file: &str) -> String {
    if directory.is_empty() {
        file.into()
    } else {
        format!("{directory}/{file}")
    }
}

fn bounded_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_owned(), false);
    }
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

fn hash_layers(layers: &[AgentInstructionLayer], truncated: bool) -> String {
    let bytes = serde_json::to_vec(&(layers, truncated)).unwrap_or_default();
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct MemoryReader(Mutex<BTreeMap<String, AgentsFile>>);

    impl AgentsFileReader for MemoryReader {
        fn read(&self, path: String, _cancellation: CancellationToken) -> AgentsReadFuture {
            let value = self.0.lock().expect("reader").get(&path).cloned();
            Box::pin(async move { Ok(value) })
        }
    }

    #[tokio::test]
    async fn root_to_cwd_layers_use_override_and_budget() {
        let reader = Arc::new(MemoryReader::default());
        reader.0.lock().expect("reader").extend([
            (
                "AGENTS.md".into(),
                AgentsFile {
                    path: "AGENTS.md".into(),
                    markdown: "root".into(),
                    content_hash: "root-hash".into(),
                },
            ),
            (
                "src/AGENTS.md".into(),
                AgentsFile {
                    path: "src/AGENTS.md".into(),
                    markdown: "ignored".into(),
                    content_hash: "ignored-hash".into(),
                },
            ),
            (
                "src/AGENTS.override.md".into(),
                AgentsFile {
                    path: "src/AGENTS.override.md".into(),
                    markdown: "override".into(),
                    content_hash: "override-hash".into(),
                },
            ),
        ]);
        let snapshot = AgentsMdLoader::new(reader)
            .with_budget(11)
            .load("src/lib", CancellationToken::new())
            .await
            .expect("snapshot");
        assert_eq!(snapshot.layers.len(), 2);
        assert_eq!(snapshot.layers[1].source_path, "src/AGENTS.override.md");
        assert!(snapshot.truncated);
        assert_ne!(snapshot.revision, hash_layers(&[], false));
    }

    #[test]
    fn rejects_escape_and_absolute_cwd() {
        assert!(relative_ancestors("../outside").is_err());
        assert!(relative_ancestors("C:/outside").is_err());
        assert!(relative_ancestors("/outside").is_err());
    }
}
