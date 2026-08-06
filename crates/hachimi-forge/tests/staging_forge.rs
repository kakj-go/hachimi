use std::{collections::BTreeSet, env, fs};

use hachimi_forge::ForgeClient;
use hachimi_protocol::{ForgeChangeMutation, ForgeKind, ForgeRepositoryIdentity};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    secret_refs: Vec<String>,
    repositories: Vec<StagingRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingRepository {
    platform_label: String,
    forge_kind: ForgeKind,
    api_base_url: String,
    fault_api_base_url: String,
    owner: String,
    repository: String,
    remote_url_hash: String,
    secret_ref: String,
    source_ref: String,
    target_ref: String,
    expected_commit_oid: String,
    merge_source_ref: String,
    merge_commit_oid: String,
}

impl StagingRepository {
    fn identity(&self) -> ForgeRepositoryIdentity {
        ForgeRepositoryIdentity {
            forge_kind: self.forge_kind,
            api_base_url: self.api_base_url.clone(),
            owner: self.owner.clone(),
            repository: self.repository.clone(),
            remote_url_hash: self.remote_url_hash.clone(),
            secret_ref: Some(self.secret_ref.clone()),
        }
    }
}

#[tokio::test]
#[ignore = "mutates protected disposable Forge staging repositories"]
async fn forge_product_adapters_conform_against_staging() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("forge")
    );
    let path = env::var("HACHIMI_STAGING_FORGE_CONFIG").expect("staging config path");
    let config: StagingConfig =
        serde_json::from_slice(&fs::read(path).expect("read staging config"))
            .expect("parse staging config");
    let labels = config
        .repositories
        .iter()
        .map(|value| value.platform_label.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        labels,
        BTreeSet::from(["github", "gitlab", "gitee", "gitea", "forgejo"])
    );
    assert_eq!(config.repositories.len(), 5);
    let client = ForgeClient::system().expect("Forge client");
    for repository in &config.repositories {
        assert!(config.secret_refs.contains(&repository.secret_ref));
        let identity = repository.identity();
        let mut fault_identity = identity.clone();
        fault_identity.api_base_url = repository.fault_api_base_url.clone();
        let marker = format!("hachimi staging {}", repository.platform_label);
        let created = client
            .mutate_with_outcome(
                &fault_identity,
                &ForgeChangeMutation::Create {
                    title: marker.clone(),
                    body: "Created by the Hachimi release conformance gate.".into(),
                    source_ref: repository.source_ref.clone(),
                    target_ref: repository.target_ref.clone(),
                },
                None,
                &repository.expected_commit_oid,
            )
            .await
            .expect("create PR/MR after the fault proxy discards its mutation response");
        assert!(
            created.reconciled_after_unknown_response,
            "the fault proxy must discard the create response"
        );
        let created = created.record;
        let queried = client
            .query(&identity, created.number)
            .await
            .expect("query PR/MR");
        assert_eq!(queried.source_ref, repository.source_ref);
        let updated = client
            .mutate(
                &identity,
                &ForgeChangeMutation::Update {
                    number: queried.number,
                    title: format!("{marker} updated"),
                    body: "Updated by the Hachimi release conformance gate.".into(),
                    source_ref: repository.source_ref.clone(),
                    target_ref: repository.target_ref.clone(),
                },
                Some(&queried.revision),
                &repository.expected_commit_oid,
            )
            .await
            .expect("update PR/MR");
        let closed = client
            .mutate_with_outcome(
                &fault_identity,
                &ForgeChangeMutation::Close {
                    number: updated.number,
                },
                Some(&updated.revision),
                &repository.expected_commit_oid,
            )
            .await
            .expect("close PR/MR after the fault proxy discards its mutation response");
        assert!(
            closed.reconciled_after_unknown_response,
            "the fault proxy must discard the close response"
        );

        let merge = client
            .mutate(
                &identity,
                &ForgeChangeMutation::Create {
                    title: format!("{marker} merge"),
                    body: "Disposable merge verification.".into(),
                    source_ref: repository.merge_source_ref.clone(),
                    target_ref: repository.target_ref.clone(),
                },
                None,
                &repository.merge_commit_oid,
            )
            .await
            .expect("create merge PR/MR");
        let merged = client
            .mutate_with_outcome(
                &fault_identity,
                &ForgeChangeMutation::Merge {
                    number: merge.number,
                    merge_title: Some(format!("{marker} merge")),
                    merge_message: Some("Hachimi release conformance merge".into()),
                },
                Some(&merge.revision),
                &repository.merge_commit_oid,
            )
            .await
            .expect("merge PR/MR after the fault proxy discards its mutation response");
        assert!(
            merged.reconciled_after_unknown_response,
            "the fault proxy must discard the merge response"
        );
    }
}
