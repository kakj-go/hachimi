ALTER TABLE agent_permission_policies
ADD COLUMN skill_allowlist_json TEXT NOT NULL DEFAULT '[]';
