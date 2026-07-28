// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/core-skills/src/{loader,model}.rs.
// Modified for Hachimi: parse a bounded YAML scalar subset without product/plugin
// coupling and retain Skill-relative icon references behind SkillHost containment.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};

use hachimi_protocol::{SkillDiagnostic, SkillDiagnosticSeverity, SkillInterface, SkillPolicy};

use crate::{SkillHostError, diagnostic, reject_reparse_chain};

const METADATA_RELATIVE_PATH: &str = "agents/openai.yaml";
const MAX_INTERFACE_VALUE: usize = 1_024;

pub(crate) fn read_interface_and_policy(
    skill_dir: &Path,
) -> Result<(Option<SkillInterface>, SkillPolicy, Vec<SkillDiagnostic>), SkillHostError> {
    let metadata_path = skill_dir.join(METADATA_RELATIVE_PATH);
    if !metadata_path.is_file() {
        return Ok((None, SkillPolicy::default(), Vec::new()));
    }
    let content = fs::read_to_string(&metadata_path)?;
    if content.len() as u64 > crate::MAX_FILE_BYTES {
        return Ok((
            None,
            SkillPolicy::default(),
            vec![metadata_diagnostic("skill_metadata_too_large")],
        ));
    }
    let sections = parse_scalar_sections(&content);
    let mut diagnostics = Vec::new();
    let interface = parse_interface(skill_dir, sections.get("interface"), &mut diagnostics);
    let policy = parse_policy(sections.get("policy"), &mut diagnostics);
    Ok((interface, policy, diagnostics))
}

fn parse_interface(
    skill_dir: &Path,
    fields: Option<&BTreeMap<String, String>>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) -> Option<SkillInterface> {
    let fields = fields?;
    let mut interface = SkillInterface {
        display_name: bounded_text(
            fields.get("display_name"),
            "skill_display_name_invalid",
            diagnostics,
        ),
        short_description: bounded_text(
            fields.get("short_description"),
            "skill_short_description_invalid",
            diagnostics,
        ),
        icon_small: relative_resource(
            skill_dir,
            fields.get("icon_small"),
            "skill_icon_small_invalid",
            diagnostics,
        ),
        icon_large: relative_resource(
            skill_dir,
            fields.get("icon_large"),
            "skill_icon_large_invalid",
            diagnostics,
        ),
        brand_color: bounded_text(
            fields.get("brand_color"),
            "skill_brand_color_invalid",
            diagnostics,
        ),
        default_prompt: bounded_text(
            fields.get("default_prompt"),
            "skill_default_prompt_invalid",
            diagnostics,
        ),
    };
    if interface
        .brand_color
        .as_deref()
        .is_some_and(|value| !valid_brand_color(value))
    {
        interface.brand_color = None;
        diagnostics.push(metadata_diagnostic("skill_brand_color_invalid"));
    }
    (interface != SkillInterface::default()).then_some(interface)
}

fn parse_policy(
    fields: Option<&BTreeMap<String, String>>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) -> SkillPolicy {
    let allow_implicit_invocation = fields
        .and_then(|fields| fields.get("allow_implicit_invocation"))
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => {
                diagnostics.push(metadata_diagnostic("skill_invocation_policy_invalid"));
                None
            }
        });
    let workload = fields
        .and_then(|fields| fields.get("workload"))
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "general" => Some(hachimi_protocol::WorkloadKind::General),
            "coding" => Some(hachimi_protocol::WorkloadKind::Coding),
            "office" => Some(hachimi_protocol::WorkloadKind::Office),
            _ => {
                diagnostics.push(metadata_diagnostic("skill_workload_invalid"));
                None
            }
        });
    SkillPolicy {
        allow_implicit_invocation,
        workload,
    }
}

fn bounded_text(
    value: Option<&String>,
    diagnostic_code: &'static str,
    diagnostics: &mut Vec<SkillDiagnostic>,
) -> Option<String> {
    let value = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() || value.len() > MAX_INTERFACE_VALUE {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    }
    Some(value)
}

fn relative_resource(
    skill_dir: &Path,
    value: Option<&String>,
    diagnostic_code: &'static str,
    diagnostics: &mut Vec<SkillDiagnostic>,
) -> Option<String> {
    let value = value?.trim().replace('\\', "/");
    if value.is_empty() || value.len() > MAX_INTERFACE_VALUE {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    }
    let relative = Path::new(&value);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    }
    let candidate = skill_dir.join(relative);
    let Ok(root) = fs::canonicalize(skill_dir) else {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    };
    let Ok(candidate) = fs::canonicalize(candidate) else {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    };
    if !candidate.is_file()
        || !candidate.starts_with(&root)
        || reject_reparse_chain(&root, &candidate).is_err()
    {
        diagnostics.push(metadata_diagnostic(diagnostic_code));
        return None;
    }
    Some(value)
}

fn valid_brand_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn parse_scalar_sections(content: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut current: Option<String> = None;
    for raw in content.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len().saturating_sub(trimmed.len());
        if indent == 0 && trimmed.ends_with(':') {
            let section = trimmed.trim_end_matches(':');
            current = matches!(section, "interface" | "policy").then(|| section.to_owned());
            continue;
        }
        if indent < 2 {
            current = None;
            continue;
        }
        let Some(section) = &current else { continue };
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let allowed = match section.as_str() {
            "interface" => matches!(
                key,
                "display_name"
                    | "short_description"
                    | "icon_small"
                    | "icon_large"
                    | "brand_color"
                    | "default_prompt"
            ),
            "policy" => matches!(key, "allow_implicit_invocation" | "workload"),
            _ => false,
        };
        if allowed {
            sections
                .entry(section.clone())
                .or_default()
                .insert(key.to_owned(), unquote(value.trim()));
        }
    }
    sections
}

fn unquote(value: &str) -> String {
    value.trim_matches(['\'', '"']).to_owned()
}

fn metadata_diagnostic(code: &str) -> SkillDiagnostic {
    diagnostic(
        code,
        "Optional Skill interface or invocation metadata is invalid",
        Some(METADATA_RELATIVE_PATH),
        SkillDiagnosticSeverity::Warning,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_interface_and_narrowing_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("office");
        fs::create_dir_all(skill.join("agents")).expect("metadata directory");
        fs::write(skill.join("icon.svg"), "<svg/>").expect("icon");
        fs::write(
            skill.join(METADATA_RELATIVE_PATH),
            "interface:\n  display_name: Office Artifacts\n  short_description: Structured office work\n  icon_small: icon.svg\n  brand_color: '#7A6FF0'\n  default_prompt: Create and validate the requested artifact\npolicy:\n  allow_implicit_invocation: false\n",
        )
        .expect("metadata");
        let (interface, policy, diagnostics) =
            read_interface_and_policy(&skill).expect("metadata result");
        let interface = interface.expect("interface");
        assert_eq!(interface.display_name.as_deref(), Some("Office Artifacts"));
        assert_eq!(interface.icon_small.as_deref(), Some("icon.svg"));
        assert!(!policy.allows_implicit_invocation());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn rejects_metadata_resource_escape_and_invalid_policy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("office");
        fs::create_dir_all(skill.join("agents")).expect("metadata directory");
        fs::write(
            skill.join(METADATA_RELATIVE_PATH),
            "interface:\n  icon_small: ../outside.svg\npolicy:\n  allow_implicit_invocation: maybe\n",
        )
        .expect("metadata");
        let (interface, policy, diagnostics) =
            read_interface_and_policy(&skill).expect("metadata result");
        assert!(interface.is_none());
        assert!(policy.allows_implicit_invocation());
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn metadata_cannot_declare_scope_grant_or_approval_authority() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("untrusted");
        fs::create_dir_all(skill.join("agents")).expect("metadata directory");
        fs::write(
            skill.join(METADATA_RELATIVE_PATH),
            "interface:\n  display_name: Untrusted Skill\npolicy:\n  allow_implicit_invocation: true\npermissions:\n  scope: workspace_write\n  grant: external_sandbox\n  approval: approved\nscope: admin\n",
        )
        .expect("metadata");

        let (interface, policy, diagnostics) =
            read_interface_and_policy(&skill).expect("metadata result");
        assert_eq!(
            interface.and_then(|value| value.display_name),
            Some("Untrusted Skill".into())
        );
        assert!(policy.allows_implicit_invocation());
        assert!(diagnostics.is_empty());

        let parsed = parse_scalar_sections(
            "permissions:\n  scope: workspace_write\n  grant: external_sandbox\n  approval: approved\nscope: admin\n",
        );
        assert!(parsed.is_empty());
    }
}
