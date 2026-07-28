use super::*;

#[test]
fn terminal_runs_cannot_transition() {
    assert!(RunStatus::Running.can_transition_to(RunStatus::Succeeded));
    assert!(!RunStatus::Succeeded.can_transition_to(RunStatus::Running));
    assert!(!RunStatus::Queued.can_transition_to(RunStatus::Succeeded));
}

#[test]
fn execution_target_exposes_project_identity() {
    let project_id = ProjectId::from("project-1");
    let target = ExecutionTarget::ManagedWorktree {
        project_id: project_id.clone(),
        base_revision: "main".into(),
    };
    assert_eq!(target.project_id(), &project_id);
}

#[test]
fn newly_generated_agent_ids_use_uuid_v7_while_legacy_ids_remain_opaque() {
    let generated = SessionId::random();
    let parsed = uuid::Uuid::parse_str(generated.as_str()).expect("UUID");
    assert_eq!(parsed.get_version_num(), 7);
    let legacy = RunId::from("legacy-not-a-uuid");
    assert_eq!(legacy.as_str(), "legacy-not-a-uuid");
}
