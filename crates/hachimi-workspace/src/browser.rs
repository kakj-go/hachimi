use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_process_policy::{ProcessPolicy, std_command};
use hachimi_protocol::{
    FsEntry, FsEntryKind, FsFileChunk, FsListPage, FsSearchId, FsSearchResult, FsSearchSnapshot,
};
use sha2::{Digest, Sha256};

use crate::{WorkerContext, WorkspaceError, WorkspaceErrorCode, WorkspaceOutput, relative_display};

const DEFAULT_CHUNK_BYTES: u32 = 256 * 1024;
const MAX_CHUNK_BYTES: u32 = 1024 * 1024;
const MAX_FUZZY_FILES: usize = 20_000;

impl WorkerContext {
    pub(crate) fn list_directory_page(
        &self,
        path: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        if !resolved.is_dir() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::NotFound,
                "workspace path is not a directory",
            ));
        }
        let mut entries = std::fs::read_dir(&resolved)
            .map_err(crate::io_error)?
            .map(|entry| entry.map_err(crate::io_error))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let limit = usize::from(limit.clamp(1, 500));
        let mut page_entries = Vec::with_capacity(limit);
        let mut has_more = false;
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if cursor.is_some_and(|cursor| name.as_str() <= cursor) {
                continue;
            }
            if page_entries.len() == limit {
                has_more = true;
                break;
            }
            page_entries.push(fs_entry(&self.root, &entry.path())?);
        }
        let next_cursor = has_more
            .then(|| page_entries.last().map(|entry| entry.name.clone()))
            .flatten();
        let etag = directory_etag(&page_entries);
        Ok(WorkspaceOutput::DirectoryPage {
            page: FsListPage {
                path: relative_display(&self.root, &resolved),
                entries: page_entries,
                next_cursor,
                etag,
            },
        })
    }

    pub(crate) fn read_file_chunk(
        &self,
        path: &str,
        offset: u64,
        limit: u32,
        if_match: Option<&str>,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        let mut file = File::open(&resolved).map_err(crate::io_error)?;
        let metadata = file.metadata().map_err(crate::io_error)?;
        if !metadata.is_file() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::NotFound,
                "workspace path is not a file",
            ));
        }
        let etag = file_etag(&mut file)?;
        let hashed_metadata = file.metadata().map_err(crate::io_error)?;
        if !same_file_version(&metadata, &hashed_metadata) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "workspace file changed while its etag was being computed",
            ));
        }
        if if_match.is_some_and(|expected| expected != etag) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "workspace file changed while it was being read",
            ));
        }
        if offset > metadata.len() {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "file chunk offset exceeds the file size",
            ));
        }
        let limit = if limit == 0 {
            DEFAULT_CHUNK_BYTES
        } else {
            limit.clamp(1, MAX_CHUNK_BYTES)
        };
        file.seek(SeekFrom::Start(offset))
            .map_err(crate::io_error)?;
        let mut bytes = vec![0_u8; usize::try_from(limit).unwrap_or(usize::MAX)];
        let read = file.read(&mut bytes).map_err(crate::io_error)?;
        bytes.truncate(read);
        let completed_metadata = file.metadata().map_err(crate::io_error)?;
        if !same_file_version(&hashed_metadata, &completed_metadata) {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::Conflict,
                "workspace file changed while its chunk was being read",
            ));
        }
        let next_offset = offset.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
        let utf8_text = (!binary)
            .then(|| String::from_utf8(bytes.clone()).ok())
            .flatten();
        Ok(WorkspaceOutput::FileChunk {
            chunk: FsFileChunk {
                path: relative_display(&self.root, &resolved),
                offset,
                next_offset,
                byte_size: metadata.len(),
                eof: next_offset >= metadata.len(),
                binary,
                data_base64: STANDARD.encode(bytes),
                utf8_text,
                etag,
            },
        })
    }

    pub(crate) fn fuzzy_file_search(
        &self,
        query: &str,
        max_results: u16,
        search_id: FsSearchId,
        generation: u64,
    ) -> Result<WorkspaceOutput, WorkspaceError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 512 {
            return Err(WorkspaceError::new(
                WorkspaceErrorCode::InvalidRequest,
                "file search query must contain 1-512 characters",
            ));
        }
        let (paths, truncated_scan) = candidate_files(&self.root)?;
        let mut results = paths
            .into_iter()
            .filter_map(|path| {
                fuzzy_score(&path, query).map(|(score, match_indices)| FsSearchResult {
                    path,
                    score,
                    match_indices,
                })
            })
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.path.cmp(&right.path))
        });
        let limit = usize::from(max_results.clamp(1, 200));
        let truncated = truncated_scan || results.len() > limit;
        results.truncate(limit);
        Ok(WorkspaceOutput::FileSearch {
            snapshot: FsSearchSnapshot {
                search_id,
                generation,
                query: query.to_owned(),
                results,
                complete: true,
                truncated,
            },
        })
    }
}

fn fs_entry(root: &Path, path: &Path) -> Result<FsEntry, WorkspaceError> {
    let metadata = std::fs::symlink_metadata(path).map_err(crate::io_error)?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        FsEntryKind::Symlink
    } else if metadata.is_file() {
        FsEntryKind::File
    } else if metadata.is_dir() {
        FsEntryKind::Directory
    } else {
        FsEntryKind::Other
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let has_children = metadata.is_dir()
        && std::fs::read_dir(path)
            .ok()
            .and_then(|mut entries| entries.next())
            .is_some();
    Ok(FsEntry {
        path: relative_display(root, path),
        hidden: name.starts_with('.'),
        name,
        kind,
        byte_size: metadata.is_file().then_some(metadata.len()),
        modified_at_ms: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_millis()).ok()),
        has_children,
        git_status: None,
    })
}

fn directory_etag(entries: &[FsEntry]) -> String {
    let encoded = serde_json::to_vec(entries).unwrap_or_default();
    format!("sha256:{}", hex_digest(&encoded))
}

fn file_etag(file: &mut File) -> Result<String, WorkspaceError> {
    file.seek(SeekFrom::Start(0)).map_err(crate::io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(crate::io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", encode_hex(&digest.finalize())))
}

fn same_file_version(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.file_type() == right.file_type()
}

fn hex_digest(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

pub(crate) fn candidate_files(root: &Path) -> Result<(Vec<String>, bool), WorkspaceError> {
    let mut command = std_command(crate::git_program(), ProcessPolicy::HiddenCaptured);
    command
        .args([
            "-c",
            "core.quotepath=false",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(crate::restricted_process_cwd(root))
        .env("GIT_OPTIONAL_LOCKS", "0");
    crate::configure_restricted_std_git_environment(&mut command);
    let output = command.output();
    if let Ok(output) = output
        && output.status.success()
    {
        let mut paths = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|path| !path.is_empty())
            .take(MAX_FUZZY_FILES + 1)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let truncated = paths.len() > MAX_FUZZY_FILES;
        paths.truncate(MAX_FUZZY_FILES);
        return Ok((paths, truncated));
    }
    let mut paths = Vec::new();
    collect_files(root, root, &mut paths)?;
    let truncated = paths.len() > MAX_FUZZY_FILES;
    paths.truncate(MAX_FUZZY_FILES);
    Ok((paths, truncated))
}

fn collect_files(root: &Path, path: &Path, output: &mut Vec<String>) -> Result<(), WorkspaceError> {
    if output.len() > MAX_FUZZY_FILES {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)
        .map_err(crate::io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::io_error)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(crate::io_error)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_files(root, &entry.path(), output)?;
        } else if metadata.is_file() {
            output.push(relative_display(root, &entry.path()));
        }
    }
    Ok(())
}

pub(crate) fn fuzzy_score(path: &str, query: &str) -> Option<(i64, Vec<u32>)> {
    let folded_path = path.to_lowercase();
    let folded_query = query.to_lowercase();
    let mut indices = Vec::new();
    let mut search_from = 0_usize;
    let mut score = 0_i64;
    for query_character in folded_query.chars() {
        let suffix = folded_path.get(search_from..)?;
        let relative = suffix.find(query_character)?;
        let byte_index = search_from.saturating_add(relative);
        indices.push(u32::try_from(byte_index).unwrap_or(u32::MAX));
        score = score.saturating_add(100 - i64::try_from(relative).unwrap_or(100).min(100));
        if byte_index == 0
            || folded_path
                .as_bytes()
                .get(byte_index.saturating_sub(1))
                .is_some_and(|byte| matches!(byte, b'/' | b'\\' | b'_' | b'-' | b'.'))
        {
            score = score.saturating_add(50);
        }
        search_from = byte_index.saturating_add(query_character.len_utf8());
    }
    Some((
        score.saturating_sub(i64::try_from(path.len()).unwrap_or(i64::MAX) / 8),
        indices,
    ))
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::FsSearchId;

    use super::*;

    fn context(root: &Path) -> WorkerContext {
        WorkerContext::new(root, "checkout", 1, "token").expect("context")
    }

    #[test]
    fn directory_pages_are_stable_and_bounded() {
        let directory = tempfile::tempdir().expect("directory");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(directory.path().join(name), name).expect("seed");
        }
        let context = context(directory.path());
        let WorkspaceOutput::DirectoryPage { page: first } = context
            .list_directory_page("", None, 2)
            .expect("first page")
        else {
            panic!("directory page");
        };
        assert_eq!(first.entries.len(), 2);
        assert_eq!(first.entries[0].name, "a.txt");
        let WorkspaceOutput::DirectoryPage { page: second } = context
            .list_directory_page("", first.next_cursor.as_deref(), 2)
            .expect("second page")
        else {
            panic!("directory page");
        };
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.entries[0].name, "c.txt");
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn chunks_use_etags_and_preserve_binary_bytes() {
        let directory = tempfile::tempdir().expect("directory");
        let bytes = [0_u8, 1, 2, 3, 255];
        std::fs::write(directory.path().join("binary.dat"), bytes).expect("seed");
        let context = context(directory.path());
        let WorkspaceOutput::FileChunk { chunk } = context
            .read_file_chunk("binary.dat", 0, 0, None)
            .expect("chunk")
        else {
            panic!("file chunk");
        };
        assert!(chunk.binary);
        assert!(chunk.eof);
        assert_eq!(STANDARD.decode(&chunk.data_base64).expect("base64"), bytes);
        std::fs::write(directory.path().join("binary.dat"), b"changed").expect("change");
        assert_eq!(
            context
                .read_file_chunk("binary.dat", 0, 0, Some(&chunk.etag))
                .expect_err("stale etag")
                .code,
            WorkspaceErrorCode::Conflict
        );
    }

    #[test]
    fn fuzzy_search_returns_paths_without_file_contents() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::create_dir(directory.path().join("source")).expect("source");
        std::fs::write(
            directory.path().join("source").join("agent_runtime.rs"),
            "secret body",
        )
        .expect("seed");
        let context = context(directory.path());
        let WorkspaceOutput::FileSearch { snapshot } = context
            .fuzzy_file_search("agr", 20, FsSearchId::random(), 4)
            .expect("search")
        else {
            panic!("search snapshot");
        };
        assert_eq!(snapshot.generation, 4);
        assert_eq!(snapshot.results.len(), 1);
        assert_eq!(snapshot.results[0].path, "source/agent_runtime.rs");
    }
}
