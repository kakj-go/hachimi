use crate::WorkspaceOperation;

pub(crate) fn workspace_operation_effect(
    operation: &WorkspaceOperation,
) -> hachimi_protocol::ToolEffect {
    match operation {
        WorkspaceOperation::WriteFile { .. }
        | WorkspaceOperation::ReplaceText { .. }
        | WorkspaceOperation::ApplyPatch { .. }
        | WorkspaceOperation::GitStage { .. }
        | WorkspaceOperation::GitUnstage { .. }
        | WorkspaceOperation::GitCommit { .. }
        | WorkspaceOperation::GitCreateEmptyInitialCommit { .. } => {
            hachimi_protocol::ToolEffect::WorkspaceWrite
        }
        WorkspaceOperation::Exec { .. } => hachimi_protocol::ToolEffect::Process,
        WorkspaceOperation::ReadFile { .. }
        | WorkspaceOperation::ListDirectory { .. }
        | WorkspaceOperation::ListDirectoryPage { .. }
        | WorkspaceOperation::ReadFileChunk { .. }
        | WorkspaceOperation::FuzzyFileSearch { .. }
        | WorkspaceOperation::SearchText { .. }
        | WorkspaceOperation::GitStatus
        | WorkspaceOperation::GitDiff
        | WorkspaceOperation::GitReviewDiff { .. }
        | WorkspaceOperation::GitDiffStructured { .. }
        | WorkspaceOperation::GitDiffFileChunk { .. }
        | WorkspaceOperation::GitStatusSnapshot
        | WorkspaceOperation::GitWorkspaceSnapshot { .. }
        | WorkspaceOperation::GitProjectInspect { .. }
        | WorkspaceOperation::ReadGitBlob { .. } => hachimi_protocol::ToolEffect::ReadOnly,
    }
}
