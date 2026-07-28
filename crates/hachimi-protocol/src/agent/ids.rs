use serde::{Deserialize, Serialize};
use specta::Type;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[must_use]
            pub fn random() -> Self {
                Self(uuid::Uuid::now_v7().to_string())
            }

            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(ProjectId);
string_id!(CheckoutId);
string_id!(SessionId);
string_id!(RunId);
string_id!(ItemId);
string_id!(ToolCallId);
string_id!(AttachmentId);
string_id!(ApprovalId);
string_id!(TaskRunId);
string_id!(PlanId);
string_id!(ArtifactId);
string_id!(CompactionCheckpointId);
string_id!(McpServerId);
string_id!(SkillId);
string_id!(SkillActivationId);
string_id!(SkillSubscriptionId);
string_id!(UserInputRequestId);
string_id!(ProcessSessionId);
string_id!(ReviewId);
string_id!(ReviewFindingId);
string_id!(RealtimeSessionId);
string_id!(PlanStepId);
string_id!(EventSubscriptionId);
string_id!(FsWatchId);
string_id!(FsSearchId);
string_id!(SideEffectExecutionId);
string_id!(AvatarId);
string_id!(ScheduleId);
string_id!(ScheduleGrantId);
