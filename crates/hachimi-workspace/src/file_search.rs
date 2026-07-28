// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/file-search/src/lib.rs and app-server/src/fuzzy_file_search.rs
// Commit: 4c43465133428898aa84f0bfc02c306ed65fb66a
// Modified for Hachimi: Checkout-bound JSONL sidecar, bounded scorer, and Run/search generation fencing.

use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::mpsc,
};

use hachimi_protocol::{FsSearchId, FsSearchResult, FsSearchSnapshot};
use serde::{Deserialize, Serialize};

use crate::{
    WorkerContext, WorkspaceError, WorkspaceErrorCode,
    browser::{candidate_files, fuzzy_score},
};

const SEARCH_BATCH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchServerRequest {
    pub checkout_id: String,
    pub run_generation: u64,
    pub worker_token: String,
    pub search_id: FsSearchId,
    pub search_generation: u64,
    pub query: String,
    pub max_results: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchServerCommand {
    Update { generation: u64, query: String },
    Cancel,
}

pub fn run_search_server(
    context: &WorkerContext,
    request: &SearchServerRequest,
    input: impl Read + Send + 'static,
    mut output: impl Write,
) -> Result<(), WorkspaceError> {
    validate_request(context, request)?;
    validate_query(&request.query)?;
    let (commands, command_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            let Ok(line) = line else {
                break;
            };
            if line.len() > 64 * 1024 {
                break;
            }
            let Ok(command) = serde_json::from_str::<SearchServerCommand>(&line) else {
                break;
            };
            if commands.send(command).is_err() {
                break;
            }
        }
        let _ = commands.send(SearchServerCommand::Cancel);
    });

    let (paths, truncated_scan) = candidate_files(&context.root)?;
    let limit = usize::from(request.max_results.clamp(1, 200));
    let mut generation = request.search_generation;
    let mut query = request.query.trim().to_owned();

    'session: loop {
        validate_query(&query)?;
        let mut results = Vec::new();
        let mut matched_count = 0_usize;
        let mut scanned = 0_usize;
        while scanned < paths.len() {
            if let Some(command) = newest_command(&command_receiver) {
                match command {
                    SearchServerCommand::Cancel => break 'session,
                    SearchServerCommand::Update {
                        generation: next_generation,
                        query: next_query,
                    } if next_generation > generation => {
                        validate_query(&next_query)?;
                        generation = next_generation;
                        query = next_query.trim().to_owned();
                        continue 'session;
                    }
                    SearchServerCommand::Update { .. } => {}
                }
            }
            let end = scanned.saturating_add(SEARCH_BATCH).min(paths.len());
            for path in &paths[scanned..end] {
                if let Some((score, match_indices)) = fuzzy_score(path, &query) {
                    matched_count = matched_count.saturating_add(1);
                    results.push(FsSearchResult {
                        path: path.clone(),
                        score,
                        match_indices,
                    });
                }
            }
            scanned = end;
            normalize_results(&mut results, limit);
            write_snapshot(
                &mut output,
                &FsSearchSnapshot {
                    search_id: request.search_id.clone(),
                    generation,
                    query: query.clone(),
                    results: results.clone(),
                    complete: false,
                    truncated: truncated_scan || matched_count > limit,
                },
            )?;
            std::thread::yield_now();
        }
        write_snapshot(
            &mut output,
            &FsSearchSnapshot {
                search_id: request.search_id.clone(),
                generation,
                query: query.clone(),
                results,
                complete: true,
                truncated: truncated_scan || matched_count > limit,
            },
        )?;
        loop {
            match command_receiver.recv() {
                Ok(SearchServerCommand::Cancel) | Err(_) => break 'session,
                Ok(SearchServerCommand::Update {
                    generation: next_generation,
                    query: next_query,
                }) if next_generation > generation => {
                    validate_query(&next_query)?;
                    generation = next_generation;
                    query = next_query.trim().to_owned();
                    continue 'session;
                }
                Ok(SearchServerCommand::Update { .. }) => {}
            }
        }
    }
    Ok(())
}

fn validate_request(
    context: &WorkerContext,
    request: &SearchServerRequest,
) -> Result<(), WorkspaceError> {
    if request.worker_token != context.worker_token || request.checkout_id != context.checkout_id {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::Unauthorized,
            "workspace search token or checkout binding is invalid",
        ));
    }
    if request.run_generation != context.run_generation {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::StaleGeneration,
            "workspace search belongs to a stale Run generation",
        ));
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), WorkspaceError> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 512 {
        return Err(WorkspaceError::new(
            WorkspaceErrorCode::InvalidRequest,
            "file search query must contain 1-512 characters",
        ));
    }
    Ok(())
}

fn newest_command(receiver: &mpsc::Receiver<SearchServerCommand>) -> Option<SearchServerCommand> {
    let mut newest = None;
    while let Ok(command) = receiver.try_recv() {
        if matches!(command, SearchServerCommand::Cancel) {
            return Some(command);
        }
        newest = Some(command);
    }
    newest
}

fn normalize_results(results: &mut Vec<FsSearchResult>, limit: usize) {
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    results.truncate(limit);
}

fn write_snapshot(
    output: &mut impl Write,
    snapshot: &FsSearchSnapshot,
) -> Result<(), WorkspaceError> {
    serde_json::to_writer(&mut *output, snapshot).map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })?;
    output.write_all(b"\n").map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })?;
    output.flush().map_err(|error| {
        WorkspaceError::new(WorkspaceErrorCode::HostDisconnected, error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::Path, time::Duration};

    use super::*;

    fn context(root: &Path) -> WorkerContext {
        WorkerContext::new(root, "checkout", 7, "token").expect("context")
    }

    struct DelayedInput {
        inner: Cursor<Vec<u8>>,
        delayed: bool,
    }

    impl Read for DelayedInput {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.delayed {
                self.delayed = true;
                // Candidate discovery starts a Git subprocess before the first snapshot. Keep the
                // cancellation input behind that work so this test observes both lifecycle phases.
                std::thread::sleep(Duration::from_secs(1));
            }
            self.inner.read(buffer)
        }
    }

    #[test]
    fn emits_incremental_and_completed_snapshots() {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::write(directory.path().join("agent_runtime.rs"), "body").expect("seed");
        let request = SearchServerRequest {
            checkout_id: "checkout".into(),
            run_generation: 7,
            worker_token: "token".into(),
            search_id: FsSearchId::random(),
            search_generation: 1,
            query: "agr".into(),
            max_results: 20,
        };
        let mut output = Vec::new();
        run_search_server(
            &context(directory.path()),
            &request,
            DelayedInput {
                inner: Cursor::new(b"{\"type\":\"cancel\"}\n".to_vec()),
                delayed: false,
            },
            &mut output,
        )
        .expect("search");
        let snapshots = String::from_utf8(output)
            .expect("utf8")
            .lines()
            .map(|line| serde_json::from_str::<FsSearchSnapshot>(line).expect("snapshot"))
            .collect::<Vec<_>>();
        assert!(snapshots.iter().any(|snapshot| !snapshot.complete));
        assert!(snapshots.iter().any(|snapshot| snapshot.complete));
    }

    #[test]
    fn stale_query_generation_is_ignored() {
        let receiver = {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(SearchServerCommand::Update {
                    generation: 1,
                    query: "old".into(),
                })
                .expect("send");
            sender
                .send(SearchServerCommand::Update {
                    generation: 2,
                    query: "new".into(),
                })
                .expect("send");
            receiver
        };
        assert_eq!(
            newest_command(&receiver),
            Some(SearchServerCommand::Update {
                generation: 2,
                query: "new".into(),
            })
        );
    }
}
