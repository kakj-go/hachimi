use hachimi_protocol::{CheckoutId, ProjectId, RunId, SessionId};
use sqlx::Row;

use super::{AgentStore, AgentStoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectToolContextIds {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub checkout_id: CheckoutId,
}

impl AgentStore {
    pub async fn project_tool_context_matches(
        &self,
        session_id: &SessionId,
        checkout_id: &CheckoutId,
    ) -> Result<bool, AgentStoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_tool_contexts WHERE session_id = ? AND checkout_id = ?",
        )
        .bind(session_id.as_str())
        .bind(checkout_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(count == 1)
    }

    pub async fn get_project_tool_context_ids(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectToolContextIds>, AgentStoreError> {
        let row = sqlx::query(
            "SELECT session_id, run_id, checkout_id FROM project_tool_contexts WHERE project_id = ?",
        )
        .bind(project_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ProjectToolContextIds {
            session_id: SessionId::new(row.get::<String, _>("session_id")),
            run_id: RunId::new(row.get::<String, _>("run_id")),
            checkout_id: CheckoutId::new(row.get::<String, _>("checkout_id")),
        }))
    }

    pub async fn bind_project_tool_context(
        &self,
        project_id: &ProjectId,
        session_id: &SessionId,
        run_id: &RunId,
        checkout_id: &CheckoutId,
        updated_at_ms: i64,
    ) -> Result<ProjectToolContextIds, AgentStoreError> {
        sqlx::query(
            "INSERT INTO project_tool_contexts (project_id, session_id, run_id, checkout_id, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(project_id) DO UPDATE SET session_id = excluded.session_id, run_id = excluded.run_id, checkout_id = excluded.checkout_id, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(project_id.as_str())
        .bind(session_id.as_str())
        .bind(run_id.as_str())
        .bind(checkout_id.as_str())
        .bind(updated_at_ms)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(ProjectToolContextIds {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            checkout_id: checkout_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{SessionRecord, SessionSearchRequest};

    use super::*;
    use crate::agent_store::tests::{run, seeded_store};

    #[tokio::test]
    async fn project_tool_context_is_idempotent_and_hidden_from_session_discovery() {
        let (store, visible_session) = seeded_store().await;
        let timestamp = visible_session.updated_at_ms + 1;
        let tool_session = SessionRecord {
            id: SessionId::from("project-tools-session"),
            context: visible_session.context.clone(),
            entry_profile: visible_session.entry_profile,
            title: "Project Tools".into(),
            archived: false,
            pinned: false,
            parent_session_id: None,
            source_run_id: None,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        };
        store
            .create_session(&tool_session)
            .await
            .expect("tool session");
        let tool_run = run(&tool_session, "project-tools-run");
        store
            .create_run_idempotent("project-tools", "project-tools-run", &tool_run)
            .await
            .expect("tool run");
        let project_id = tool_session.context.project_id().expect("project").clone();
        let checkout_id = tool_session
            .context
            .checkout_id()
            .expect("checkout")
            .clone();

        let first = store
            .bind_project_tool_context(
                &project_id,
                &tool_session.id,
                &tool_run.id,
                &checkout_id,
                timestamp,
            )
            .await
            .expect("first binding");
        let second = store
            .bind_project_tool_context(
                &project_id,
                &tool_session.id,
                &tool_run.id,
                &checkout_id,
                timestamp + 1,
            )
            .await
            .expect("idempotent binding");
        assert_eq!(first, second);
        assert_eq!(
            store
                .get_project_tool_context_ids(&project_id)
                .await
                .expect("context lookup"),
            Some(first)
        );
        assert!(
            store
                .project_tool_context_matches(&tool_session.id, &checkout_id)
                .await
                .expect("bound context")
        );
        assert!(
            !store
                .project_tool_context_matches(
                    &tool_session.id,
                    &CheckoutId::from("different-checkout"),
                )
                .await
                .expect("wrong checkout")
        );
        assert!(
            !store
                .project_tool_context_matches(&visible_session.id, &checkout_id)
                .await
                .expect("ordinary session")
        );

        let listed = store.list_sessions(None).await.expect("sessions");
        assert_eq!(listed, vec![visible_session.clone()]);
        let workbench = store
            .list_workbench_session_items(None)
            .await
            .expect("workbench sessions");
        assert_eq!(workbench.len(), 1);
        assert_eq!(workbench[0].session, visible_session);
        let searched = store
            .search_sessions(&SessionSearchRequest {
                project_id: Some(project_id),
                query: Some("Project Tools".into()),
                archived: None,
                before: None,
                limit: 20,
            })
            .await
            .expect("session search");
        assert!(searched.items.is_empty());
    }
}
