// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex/codex-rs/exec-server/src/server/{process_handler,session_registry}.rs
// @ 4c43465133428898aa84f0bfc02c306ed65fb66a.
// Modified for Hachimi: AppServer mutation fencing, persistent process metadata,
// restricted Windows launch, and a transport-neutral Process domain.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use hachimi_control_plane::{
    AppServerContext, AppServerDomainError, ProcessAppRequest, ProcessAppResponse,
};
use hachimi_process::{ProcessError, ProcessLaunchSpec};
use hachimi_protocol::{
    ClientId, MutationContext, ProcessReadSnapshot, ProcessSessionId, ProcessSessionRecord,
    ProcessSpawnRequest, ProcessStatus, SessionId,
};
use hachimi_sandbox::{SandboxBackend, SandboxStatus};
use sha2::{Digest, Sha256};

use super::DesktopAppDomainHandler;
use crate::workbench_commands::sandbox_sidecar_path;

const MAX_COMMAND_ITEMS: usize = 128;
const MAX_COMMAND_BYTES: usize = 8_192;
const DEFAULT_OUTPUT_CAP: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_CAP: usize = 16 * 1024 * 1024;
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const ALLOWED_ENVIRONMENT: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "WINDIR",
    "TEMP",
    "TMP",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "HACHIMI_WORKSPACE_WORKER_TOKEN",
];

fn validate_process_sandbox(
    status: SandboxStatus,
    restricted_backend: bool,
) -> Result<(), AppServerDomainError> {
    if status != SandboxStatus::Enforced && restricted_backend {
        return Err(AppServerDomainError::new(
            "sandbox_not_enforced",
            "process execution is disabled until Windows sandbox attestation succeeds",
        ));
    }
    Ok(())
}

fn process_run_binding(
    context: &MutationContext,
) -> Result<Option<(hachimi_protocol::RunId, u64)>, AppServerDomainError> {
    match (context.expected_run_id.clone(), context.expected_generation) {
        (Some(run_id), Some(generation)) => Ok(Some((run_id, generation))),
        (None, None) => Ok(None),
        _ => Err(AppServerDomainError::new(
            "process_run_binding_incomplete",
            "process Run and generation must either both be present or both be absent",
        )),
    }
}

fn validate_direct_terminal(request: &ProcessSpawnRequest) -> Result<(), AppServerDomainError> {
    if !request.tty || !request.stream_stdin || !request.stream_output {
        return Err(AppServerDomainError::new(
            "process_direct_terminal_invalid",
            "direct user processes must be interactive terminals with stdin and output streaming",
        ));
    }
    Ok(())
}

impl DesktopAppDomainHandler {
    fn sandbox_backend(&self) -> Option<Arc<dyn SandboxBackend>> {
        (self.sandbox_runtime.snapshot().report.backend != "desktop-e2e-deterministic")
            .then(|| Arc::clone(&self.sandbox_runtime) as Arc<dyn SandboxBackend>)
    }

    async fn process_record(
        &self,
        id: &ProcessSessionId,
    ) -> Result<ProcessSessionRecord, AppServerDomainError> {
        match self.processes.get(id).await {
            Ok(record) => Ok(record),
            Err(ProcessError::NotFound(_)) => self
                .store
                .get_process_session(id)
                .await
                .map_err(process_domain_error)?
                .ok_or_else(|| {
                    AppServerDomainError::new("process_not_found", "process session does not exist")
                }),
            Err(error) => Err(process_domain_error(error)),
        }
    }

    async fn assert_process_run_binding(
        &self,
        session_id: &SessionId,
        run_id: &hachimi_protocol::RunId,
        generation: u64,
    ) -> Result<(), AppServerDomainError> {
        self.store
            .assert_run_precondition(run_id, run_id, generation)
            .await
            .map_err(process_domain_error)?;
        let session = self
            .store
            .get_session(session_id)
            .await
            .map_err(process_domain_error)?
            .ok_or_else(|| {
                AppServerDomainError::new("process_session_not_found", "session does not exist")
            })?;
        if session.id != *session_id {
            return Err(AppServerDomainError::new(
                "process_session_mismatch",
                "process request is not bound to the expected session",
            ));
        }
        Ok(())
    }

    fn check_process_owner(
        record: &ProcessSessionRecord,
        client_id: &ClientId,
        mutation: &MutationContext,
    ) -> Result<(), AppServerDomainError> {
        if record.owner_client_id != *client_id {
            return Err(AppServerDomainError::new(
                "process_owner_mismatch",
                "process owner mismatch",
            ));
        }
        if mutation.expected_run_id.as_ref() != record.run_id.as_ref()
            || mutation.expected_generation != record.run_generation
        {
            return Err(AppServerDomainError::new(
                "process_run_precondition_failed",
                "the process Run or generation changed",
            ));
        }
        Ok(())
    }

    async fn spawn_process(
        &self,
        context: &AppServerContext,
        request: ProcessSpawnRequest,
    ) -> Result<ProcessSessionRecord, AppServerDomainError> {
        Self::validate_mutation(context, &request.context)?;
        if request.command.is_empty()
            || request.command.len() > MAX_COMMAND_ITEMS
            || request.command.iter().any(|part| {
                part.is_empty() || part.len() > MAX_COMMAND_BYTES || part.contains('\0')
            })
        {
            return Err(AppServerDomainError::new(
                "process_command_invalid",
                "command must contain 1-128 bounded arguments",
            ));
        }
        let run_binding = process_run_binding(&request.context)?;
        let restricted = run_binding.is_some();
        let _activity = if restricted {
            Some(self.enter_sandbox_activity()?)
        } else {
            validate_direct_terminal(&request)?;
            None
        };
        if restricted {
            let sandbox_snapshot = self.sandbox_runtime.snapshot();
            validate_process_sandbox(
                SandboxStatus::from_report(&sandbox_snapshot.report),
                self.sandbox_backend().is_some(),
            )?;
        }
        let checkout = self
            .resolve_session_checkout(&request.session_id, &request.checkout_id)
            .await?;
        if let Some((run_id, generation)) = &run_binding {
            let workspace = self
                .resolve_session_workspace(&request.session_id, &request.checkout_id)
                .await?;
            if workspace.run.id != *run_id || workspace.run.generation != *generation {
                return Err(AppServerDomainError::new(
                    "process_run_precondition_failed",
                    "the active Run or generation changed",
                ));
            }
            self.assert_process_run_binding(&request.session_id, run_id, *generation)
                .await?;
        } else if !self
            .store
            .project_tool_context_matches(&request.session_id, &request.checkout_id)
            .await
            .map_err(process_domain_error)?
        {
            return Err(AppServerDomainError::new(
                "process_direct_user_forbidden",
                "direct user terminals require a bound project tool context",
            ));
        }
        let environment = if restricted {
            resolve_process_environment(request.environment.clone())?
        } else {
            resolve_user_terminal_environment(request.environment.clone())?
        };
        let output_cap = usize::try_from(
            request
                .output_bytes_cap
                .unwrap_or(u64::try_from(DEFAULT_OUTPUT_CAP).unwrap_or(u64::MAX)),
        )
        .unwrap_or(DEFAULT_OUTPUT_CAP)
        .clamp(1, MAX_OUTPUT_CAP);
        let now = super::now_ms();
        let fingerprint = process_request_fingerprint(&request)?;
        let candidate = ProcessSessionRecord {
            id: ProcessSessionId::random(),
            session_id: request.session_id.clone(),
            run_id: run_binding.as_ref().map(|(run_id, _)| run_id.clone()),
            checkout_id: request.checkout_id,
            run_generation: run_binding.as_ref().map(|(_, generation)| *generation),
            owner_client_id: context.client.client_id.clone(),
            command_summary: process_command_summary(&request.command),
            interactive: request.tty,
            status: ProcessStatus::Starting,
            exit_code: None,
            output_limit_bytes: u64::try_from(output_cap).unwrap_or(u64::MAX),
            created_at_ms: now,
            updated_at_ms: now,
            reconnect_expires_at_ms: None,
        };
        self.idempotent(
            context,
            &request.context,
            "process.spawn",
            &format!("{}:{fingerprint}", request.session_id),
            || async {
                let launch = ProcessLaunchSpec {
                    record: candidate,
                    restricted_launcher: if restricted {
                        self.sandbox_backend()
                            .map(|_| sandbox_sidecar_path("hachimi-sandbox-launcher"))
                    } else {
                        None
                    },
                    command: request.command,
                    cwd: checkout.path.into(),
                    environment,
                    tty: request.tty,
                    stream_stdin: request.stream_stdin,
                    output_bytes_cap: output_cap,
                    timeout: process_timeout(request.timeout_ms),
                    size: request.size.unwrap_or_default(),
                    reconnect_ttl: Duration::from_secs(60),
                };
                let launched = self
                    .processes
                    .spawn(launch)
                    .await
                    .map_err(process_domain_error)?;
                if let Err(error) = self.store.upsert_process_session(&launched).await {
                    let _ = self
                        .processes
                        .terminate(&context.client.client_id, &launched.id)
                        .await;
                    return Err(process_domain_error(error));
                }
                Ok(launched)
            },
        )
        .await
    }

    pub(super) async fn dispatch_process(
        &self,
        context: &AppServerContext,
        request: ProcessAppRequest,
    ) -> Result<ProcessAppResponse, AppServerDomainError> {
        let response = match request {
            ProcessAppRequest::Spawn(request) => {
                ProcessAppResponse::Process(self.spawn_process(context, request).await?)
            }
            ProcessAppRequest::Write(request) => {
                Self::validate_mutation(context, &request.context)?;
                let record = self.process_record(&request.process_session_id).await?;
                Self::check_process_owner(&record, &context.client.client_id, &request.context)?;
                self.processes
                    .write_base64(
                        &context.client.client_id,
                        &request.process_session_id,
                        &request.write_id,
                        request.delta_base64.as_deref(),
                        request.close_stdin,
                    )
                    .await
                    .map_err(process_domain_error)?;
                ProcessAppResponse::Acknowledged
            }
            ProcessAppRequest::Resize(request) => {
                Self::validate_mutation(context, &request.context)?;
                let record = self.process_record(&request.process_session_id).await?;
                Self::check_process_owner(&record, &context.client.client_id, &request.context)?;
                self.processes
                    .resize(
                        &context.client.client_id,
                        &request.process_session_id,
                        request.size,
                    )
                    .await
                    .map_err(process_domain_error)?;
                ProcessAppResponse::Acknowledged
            }
            ProcessAppRequest::Terminate(request) => {
                Self::validate_mutation(context, &request.context)?;
                let record = self.process_record(&request.process_session_id).await?;
                Self::check_process_owner(&record, &context.client.client_id, &request.context)?;
                self.processes
                    .terminate(&context.client.client_id, &request.process_session_id)
                    .await
                    .map_err(process_domain_error)?;
                let updated = self
                    .processes
                    .get(&request.process_session_id)
                    .await
                    .map_err(process_domain_error)?;
                self.store
                    .upsert_process_session(&updated)
                    .await
                    .map_err(process_domain_error)?;
                ProcessAppResponse::Process(updated)
            }
            ProcessAppRequest::Read(request) => {
                let record = self.process_record(&request.process_session_id).await?;
                if record.owner_client_id != context.client.client_id {
                    return Err(AppServerDomainError::new(
                        "process_owner_mismatch",
                        "process owner mismatch",
                    ));
                }
                let snapshot = self
                    .processes
                    .read(
                        &request.process_session_id,
                        request.after_sequence,
                        request.max_bytes.map(|value| value as usize),
                        request
                            .wait_ms
                            .map(|value| Duration::from_millis(u64::from(value))),
                    )
                    .await
                    .or_else(|error| match error {
                        ProcessError::NotFound(_) => Ok(ProcessReadSnapshot {
                            process: record,
                            chunks: Vec::new(),
                            next_sequence: request.after_sequence.unwrap_or_default(),
                            closed: true,
                        }),
                        other => Err(other),
                    })
                    .map_err(process_domain_error)?;
                ProcessAppResponse::Read(snapshot)
            }
            ProcessAppRequest::List(request) => {
                self.processes.attach_owner(&context.client.client_id).await;
                let mut records = self
                    .processes
                    .list(&context.client.client_id, request.include_terminal)
                    .await;
                for record in self
                    .store
                    .list_process_sessions(
                        request.session_id.as_ref(),
                        request.run_id.as_ref(),
                        request.include_terminal,
                    )
                    .await
                    .map_err(process_domain_error)?
                {
                    if record.owner_client_id == context.client.client_id
                        && !records.iter().any(|current| current.id == record.id)
                    {
                        records.push(record);
                    }
                }
                records.retain(|record| {
                    request
                        .session_id
                        .as_ref()
                        .is_none_or(|id| &record.session_id == id)
                        && request
                            .run_id
                            .as_ref()
                            .is_none_or(|id| record.run_id.as_ref() == Some(id))
                });
                records.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
                ProcessAppResponse::Processes(records)
            }
        };
        Ok(response)
    }
}

fn process_domain_error(error: impl std::fmt::Display) -> AppServerDomainError {
    AppServerDomainError::new("process_failed", error.to_string())
}

fn process_command_summary(command: &[String]) -> String {
    command
        .iter()
        .map(|part| part.replace(['\r', '\n', '\0'], " "))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(512)
        .collect()
}

fn process_request_fingerprint(
    request: &ProcessSpawnRequest,
) -> Result<String, AppServerDomainError> {
    let bytes =
        serde_json::to_vec(request).map_err(super::domain_error("process_request_invalid"))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn resolve_process_environment(
    values: BTreeMap<String, Option<String>>,
) -> Result<BTreeMap<String, String>, AppServerDomainError> {
    let mut environment = BTreeMap::new();
    for (name, value) in values {
        let normalized = name.to_ascii_uppercase();
        if !ALLOWED_ENVIRONMENT.contains(&normalized.as_str()) {
            return Err(AppServerDomainError::new(
                "process_environment_denied",
                format!("environment variable {name} is not allowed"),
            ));
        }
        if name.contains('\0') || value.as_deref().is_some_and(|value| value.contains('\0')) {
            return Err(AppServerDomainError::new(
                "process_environment_invalid",
                "environment names and values must not contain NUL",
            ));
        }
        if let Some(value) = value {
            environment.insert(normalized, value);
        }
    }
    for name in ["PATH", "SYSTEMROOT"] {
        if !environment.contains_key(name)
            && let Some(value) = std::env::var_os(name)
        {
            environment.insert(name.into(), value.to_string_lossy().into_owned());
        }
    }
    Ok(environment)
}

fn resolve_user_terminal_environment(
    values: BTreeMap<String, Option<String>>,
) -> Result<BTreeMap<String, String>, AppServerDomainError> {
    let mut environment = std::env::vars()
        .filter_map(|(name, value)| {
            let normalized = name.to_ascii_uppercase();
            (!normalized.starts_with("HACHIMI_")
                && !normalized.is_empty()
                && !normalized.contains(['=', '\0'])
                && !value.contains('\0'))
            .then_some((normalized, value))
        })
        .collect::<BTreeMap<_, _>>();
    environment.extend(
        resolve_process_environment(values)?
            .into_iter()
            .filter(|(name, _)| !name.starts_with("HACHIMI_")),
    );
    Ok(environment)
}

fn process_timeout(timeout_ms: Option<u64>) -> Option<Duration> {
    timeout_ms
        .filter(|value| *value > 0)
        .map(|value| Duration::from_millis(value).min(MAX_PROCESS_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{CheckoutId, ProcessStatus, RequestId, RunId};

    use super::*;

    fn mutation(run_id: Option<&str>, generation: Option<u64>) -> MutationContext {
        MutationContext {
            request_id: RequestId("process-test-request".into()),
            client_id: ClientId("workbench".into()),
            protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
            idempotency_key: "process-test".into(),
            expected_run_id: run_id.map(RunId::from),
            expected_generation: generation,
        }
    }

    fn direct_record(owner: &str) -> ProcessSessionRecord {
        ProcessSessionRecord {
            id: ProcessSessionId::random(),
            session_id: SessionId::from("project-tool-session"),
            run_id: None,
            checkout_id: CheckoutId::from("project-tool-checkout"),
            run_generation: None,
            owner_client_id: ClientId(owner.into()),
            command_summary: "powershell.exe".into(),
            interactive: true,
            status: ProcessStatus::Running,
            exit_code: None,
            output_limit_bytes: 1024,
            created_at_ms: 1,
            updated_at_ms: 1,
            reconnect_expires_at_ms: None,
        }
    }

    #[test]
    fn direct_process_binding_requires_both_run_fields_or_neither() {
        assert_eq!(process_run_binding(&mutation(None, None)).unwrap(), None);
        assert_eq!(
            process_run_binding(&mutation(Some("run-1"), Some(7))).unwrap(),
            Some((RunId::from("run-1"), 7))
        );
        for context in [mutation(Some("run-1"), None), mutation(None, Some(7))] {
            assert_eq!(
                process_run_binding(&context).unwrap_err().code,
                "process_run_binding_incomplete"
            );
        }
    }

    #[test]
    fn process_spawn_requires_enforced_sandbox_for_restricted_backend() {
        assert!(validate_process_sandbox(SandboxStatus::Enforced, true).is_ok());
        assert!(validate_process_sandbox(SandboxStatus::Disabled, false).is_ok());
        assert_eq!(
            validate_process_sandbox(SandboxStatus::Degraded, true)
                .unwrap_err()
                .code,
            "sandbox_not_enforced"
        );
    }

    #[test]
    fn direct_processes_must_be_interactive_terminals() {
        let mut request = ProcessSpawnRequest {
            context: mutation(None, None),
            session_id: SessionId::from("project-tool-session"),
            checkout_id: CheckoutId::from("project-tool-checkout"),
            command: vec!["powershell.exe".into()],
            tty: true,
            stream_stdin: true,
            stream_output: true,
            output_bytes_cap: None,
            timeout_ms: None,
            environment: BTreeMap::new(),
            size: None,
        };
        assert!(validate_direct_terminal(&request).is_ok());
        request.stream_stdin = false;
        assert_eq!(
            validate_direct_terminal(&request).unwrap_err().code,
            "process_direct_terminal_invalid"
        );
    }

    #[test]
    fn direct_terminal_inherits_user_environment_without_internal_capabilities() {
        let environment = resolve_user_terminal_environment(BTreeMap::from([
            ("PATH".into(), Some("terminal-path".into())),
            (
                "HACHIMI_WORKSPACE_WORKER_TOKEN".into(),
                Some("explicit-value".into()),
            ),
        ]))
        .unwrap();
        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("terminal-path")
        );
        assert_eq!(environment.get("HACHIMI_WORKSPACE_WORKER_TOKEN"), None);
        for name in std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| name.starts_with("HACHIMI_"))
        {
            assert!(!environment.contains_key(&name));
        }
    }

    #[test]
    fn direct_process_mutations_require_the_process_owner() {
        let record = direct_record("window:workbench");
        let context = mutation(None, None);
        assert!(
            DesktopAppDomainHandler::check_process_owner(
                &record,
                &ClientId("window:workbench".into()),
                &context,
            )
            .is_ok()
        );
        assert_eq!(
            DesktopAppDomainHandler::check_process_owner(
                &record,
                &ClientId("window:other".into()),
                &context,
            )
            .unwrap_err()
            .code,
            "process_owner_mismatch"
        );
    }
}
