use std::{env, fs, path::PathBuf, time::Duration};

use hachimi_workspace::{WorkspaceHostClient, WorkspaceOperation, WorkspaceOutput};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    repositories: Vec<StagingRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingRepository {
    platform_label: String,
    checkout_path: PathBuf,
    remote_name: String,
    remote_url_hash: String,
    source_ref: String,
    expected_commit_oid: String,
}

#[tokio::test]
#[ignore = "fetches and pushes protected disposable Forge staging repositories"]
async fn git_product_host_fetches_and_pushes_staging_remotes() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("forge")
    );
    let path = env::var("HACHIMI_STAGING_FORGE_CONFIG").expect("staging config path");
    let config: StagingConfig =
        serde_json::from_slice(&fs::read(path).expect("read staging config"))
            .expect("parse staging config");
    for repository in config.repositories {
        let client = WorkspaceHostClient::new(
            env!("CARGO_BIN_EXE_hachimi-workspace-worker"),
            &repository.checkout_path,
            format!("release-{}", repository.platform_label),
            1,
        );
        let remotes = client
            .execute(
                WorkspaceOperation::GitRemotes,
                Duration::from_secs(30),
                CancellationToken::new(),
            )
            .await
            .expect("list staging Git remotes");
        let WorkspaceOutput::GitRemotes { remotes } = remotes else {
            panic!("unexpected Git remotes response")
        };
        let remote = remotes
            .iter()
            .find(|remote| remote.name == repository.remote_name)
            .expect("configured staging remote");
        assert_eq!(remote.remote_url_hash, repository.remote_url_hash);

        let fetched = client
            .execute(
                WorkspaceOperation::Exec {
                    program: "git".into(),
                    args: vec![
                        "fetch".into(),
                        "--prune".into(),
                        "--".into(),
                        repository.remote_name.clone(),
                    ],
                    cwd: ".".into(),
                    timeout_ms: 120_000,
                },
                Duration::from_secs(130),
                CancellationToken::new(),
            )
            .await
            .expect("fetch staging remote");
        assert!(matches!(
            fetched,
            WorkspaceOutput::Process {
                exit_code: Some(0),
                ..
            }
        ));

        let pushed = client
            .execute(
                WorkspaceOperation::GitPush {
                    remote_name: repository.remote_name.clone(),
                    expected_remote_url_hash: repository.remote_url_hash.clone(),
                    source_ref: repository.source_ref.clone(),
                    target_ref: format!("refs/heads/{}", repository.source_ref),
                    expected_commit_oid: repository.expected_commit_oid.clone(),
                },
                Duration::from_secs(130),
                CancellationToken::new(),
            )
            .await
            .expect("push staging remote");
        let WorkspaceOutput::GitPush { response } = pushed else {
            panic!("unexpected Git push response")
        };
        assert!(response.confirmed);
        assert_eq!(response.commit_oid, repository.expected_commit_oid);
    }
}
