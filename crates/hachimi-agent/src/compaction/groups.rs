use std::collections::BTreeSet;

use hachimi_protocol::{
    CompactionCheckpoint, ItemPayload, ItemStatus, RunId, TranscriptItem, TranscriptItemKind,
};

use super::{CompactionPolicy, bounded_head_tail, render_item, transcript_text};

#[derive(Debug)]
pub(super) struct ConversationGroup<'a> {
    items: Vec<&'a TranscriptItem>,
    protected: bool,
    warnings: Vec<String>,
}

impl<'a> ConversationGroup<'a> {
    fn new(item: &'a TranscriptItem) -> Self {
        Self {
            items: vec![item],
            protected: false,
            warnings: Vec::new(),
        }
    }

    fn first_sequence(&self) -> u64 {
        self.items.first().map_or(0, |item| item.sequence)
    }

    fn last_sequence(&self) -> u64 {
        self.items.last().map_or(0, |item| item.sequence)
    }

    fn run_id(&self) -> Option<&RunId> {
        self.items.first().and_then(|item| item.run_id.as_ref())
    }
}

#[derive(Debug)]
pub(super) struct PreparedSource {
    pub(super) rendered: String,
    pub(super) latest_user_goal: Option<String>,
    pub(super) covered_through_sequence: u64,
    pub(super) source_items: usize,
    pub(super) source_chars: usize,
    pub(super) recent_tail_items: usize,
    pub(super) warnings: Vec<String>,
}

pub(super) fn prepare_source(
    transcript: &[TranscriptItem],
    current_run_id: Option<&RunId>,
    previous: Option<&CompactionCheckpoint>,
    policy: CompactionPolicy,
    force: bool,
    _include_current_run: bool,
) -> Option<PreparedSource> {
    let previous_sequence = previous
        .map(|checkpoint| checkpoint.covered_through_sequence)
        .unwrap_or_default();
    let (groups, mut warnings) = conversation_groups(
        transcript
            .iter()
            .filter(|item| item.sequence > previous_sequence),
        current_run_id,
    );
    let eligible = groups
        .into_iter()
        .filter(|group| !group.protected)
        .collect::<Vec<_>>();
    if eligible.len() <= 2 {
        return None;
    }

    // Retain at least the last two complete turns. The legacy item limit is
    // still honored as a larger lower bound without ever splitting a turn.
    let mut retained_groups = 0_usize;
    let mut retained_items = 0_usize;
    for group in eligible.iter().rev() {
        if retained_groups >= 2 && retained_items >= policy.recent_tail_items {
            break;
        }
        retained_groups = retained_groups.saturating_add(1);
        retained_items = retained_items.saturating_add(group.items.len());
    }
    if retained_groups >= eligible.len() {
        return None;
    }

    let coverable = &eligible[..eligible.len().saturating_sub(retained_groups)];
    let available_chars = coverable
        .iter()
        .flat_map(|group| group.items.iter())
        .filter_map(|item| render_item(item, policy.max_item_chars))
        .map(|rendered| rendered.chars().count())
        .sum::<usize>();
    if !force && available_chars < policy.automatic_trigger_chars {
        return None;
    }

    let mut rendered = String::new();
    let mut selected = Vec::new();
    for group in coverable {
        let group_text = render_group(group, policy.max_item_chars);
        if group_text.is_empty() {
            continue;
        }
        let separator_chars = usize::from(!rendered.is_empty()) * 2;
        let next_chars = group_text.chars().count().saturating_add(separator_chars);
        if !selected.is_empty()
            && rendered.chars().count().saturating_add(next_chars) > policy.max_source_chars
        {
            break;
        }
        if !rendered.is_empty() {
            rendered.push_str("\n\n");
        }
        let remaining = policy
            .max_source_chars
            .saturating_sub(rendered.chars().count());
        rendered.push_str(&bounded_head_tail(&group_text, remaining));
        selected.push(group);
        if rendered.chars().count() >= policy.max_source_chars {
            break;
        }
    }
    let last = selected.last()?;
    let latest_user_goal = selected
        .iter()
        .rev()
        .flat_map(|group| group.items.iter().rev())
        .find(|item| item.kind == TranscriptItemKind::User)
        .and_then(|item| transcript_text(item))
        .map(|text| bounded_head_tail(&text, 8 * 1024));
    for group in &selected {
        warnings.extend(group.warnings.iter().cloned());
    }
    warnings.sort();
    warnings.dedup();
    Some(PreparedSource {
        source_chars: rendered.chars().count(),
        source_items: selected.iter().map(|group| group.items.len()).sum(),
        recent_tail_items: retained_items,
        covered_through_sequence: last.last_sequence(),
        rendered,
        latest_user_goal,
        warnings,
    })
}

fn conversation_groups<'a>(
    items: impl Iterator<Item = &'a TranscriptItem>,
    current_run_id: Option<&RunId>,
) -> (Vec<ConversationGroup<'a>>, Vec<String>) {
    let mut groups = Vec::<ConversationGroup<'a>>::new();
    let mut seen_tool_calls = BTreeSet::new();
    let mut warnings = Vec::new();
    for item in items.filter(|item| is_groupable_item(item.kind)) {
        let starts_group = item.kind == TranscriptItemKind::User
            || groups
                .last()
                .is_some_and(|group| group.run_id() != item.run_id.as_ref());
        if starts_group || groups.is_empty() {
            groups.push(ConversationGroup::new(item));
        } else if let Some(group) = groups.last_mut() {
            group.items.push(item);
        }
        let group = groups.last_mut().expect("conversation group exists");
        if item
            .run_id
            .as_ref()
            .is_some_and(|run_id| Some(run_id) == current_run_id)
        {
            group.protected = true;
            warnings.push("current_run_excluded_from_compaction_source".into());
        }
        if matches!(
            item.kind,
            TranscriptItemKind::Approval | TranscriptItemKind::UserInputRequest
        ) {
            group.protected = true;
            group
                .warnings
                .push("interactive_authority_excluded_from_compaction_source".into());
        }
        if let ItemPayload::ToolExecution {
            tool_call_id,
            result,
            ..
        } = &item.payload
        {
            if !seen_tool_calls.insert(tool_call_id.clone()) {
                group.protected = true;
                group
                    .warnings
                    .push("duplicate_tool_call_repaired_in_model_view".into());
            }
            if result.is_none()
                || matches!(item.status, ItemStatus::Pending | ItemStatus::InProgress)
            {
                group.protected = true;
                group
                    .warnings
                    .push("incomplete_tool_call_repaired_in_model_view".into());
            }
        }
    }
    for group in &groups {
        warnings.extend(group.warnings.iter().cloned());
    }
    (groups, warnings)
}

fn render_group(group: &ConversationGroup<'_>, max_item_chars: usize) -> String {
    let mut rendered = format!(
        "[group first_sequence={} last_sequence={}]",
        group.first_sequence(),
        group.last_sequence()
    );
    for item in &group.items {
        if let Some(text) = render_item(item, max_item_chars) {
            rendered.push('\n');
            rendered.push_str(&text);
        }
    }
    rendered
}

const fn is_groupable_item(kind: TranscriptItemKind) -> bool {
    matches!(
        kind,
        TranscriptItemKind::User
            | TranscriptItemKind::Assistant
            | TranscriptItemKind::Plan
            | TranscriptItemKind::ToolExecution
            | TranscriptItemKind::Approval
            | TranscriptItemKind::UserInputRequest
            | TranscriptItemKind::Reasoning
            | TranscriptItemKind::CommandExecution
            | TranscriptItemKind::FileChange
            | TranscriptItemKind::McpCall
            | TranscriptItemKind::DynamicToolCall
            | TranscriptItemKind::CollabToolCall
            | TranscriptItemKind::Review
    )
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        AgentMessagePhase, ItemId, ItemRelations, SessionId, ToolCallId, ToolExecutionResult,
    };

    use super::*;

    fn item(sequence: u64, kind: TranscriptItemKind, run: &str) -> TranscriptItem {
        let payload = match kind {
            TranscriptItemKind::User => ItemPayload::User {
                text: format!("goal {sequence}"),
                attachment_ids: Vec::new(),
            },
            TranscriptItemKind::Assistant => ItemPayload::Assistant {
                text: format!("answer {sequence}"),
                phase: AgentMessagePhase::Unknown,
            },
            TranscriptItemKind::ToolExecution => ItemPayload::ToolExecution {
                tool_call_id: ToolCallId::new(format!("call-{sequence}")),
                name: "search".into(),
                arguments: serde_json::json!({}),
                step_revision: 1,
                tool_plan_hash: "plan".into(),
                registry_revision: "registry".into(),
                result: Some(ToolExecutionResult {
                    status: "succeeded".into(),
                    model_content: "result".into(),
                    structured_content: serde_json::json!({}),
                    stable_result_code: "ok".into(),
                }),
            },
            _ => panic!("unsupported fixture"),
        };
        TranscriptItem {
            id: ItemId::new(format!("item-{sequence}")),
            session_id: SessionId::new("session"),
            run_id: Some(RunId::new(run)),
            sequence,
            kind,
            status: ItemStatus::Completed,
            payload,
            relations: ItemRelations::default(),
            created_at_ms: sequence as i64,
        }
    }

    #[test]
    fn user_assistant_and_tool_sequence_is_one_group() {
        let transcript = [
            item(1, TranscriptItemKind::User, "old"),
            item(2, TranscriptItemKind::Assistant, "old"),
            item(3, TranscriptItemKind::ToolExecution, "old"),
        ];
        let (groups, warnings) = conversation_groups(transcript.iter(), None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].items.len(), 3);
        assert!(warnings.is_empty());
    }

    #[test]
    fn current_run_and_incomplete_tool_groups_are_protected() {
        let mut incomplete = item(2, TranscriptItemKind::ToolExecution, "old");
        if let ItemPayload::ToolExecution { result, .. } = &mut incomplete.payload {
            *result = None;
        }
        let transcript = [
            item(1, TranscriptItemKind::User, "old"),
            incomplete,
            item(3, TranscriptItemKind::User, "current"),
        ];
        let current = RunId::new("current");
        let (groups, warnings) = conversation_groups(transcript.iter(), Some(&current));
        assert!(groups.iter().all(|group| group.protected));
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("incomplete_tool"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("current_run"))
        );
    }
}
