use serde_json::json;

pub(super) fn resolution_message(
    error: &hachimi_computer::ComputerTargetResolutionError,
) -> String {
    let detail = match error {
        hachimi_computer::ComputerTargetResolutionError::NoApplicationMatch { app_name } => json!({
            "code": "computer_application_not_found",
            "appName": app_name,
            "nextAction": "Call computer_list_windows and retry with an available appName."
        }),
        hachimi_computer::ComputerTargetResolutionError::AmbiguousApplication {
            app_name,
            candidates,
        } => json!({
            "code": "computer_application_ambiguous",
            "appName": app_name,
            "candidates": candidates,
            "nextAction": "User selection is required. Ask which application name they mean in an ordinary assistant follow-up. Never ask for a PID or window handle."
        }),
        hachimi_computer::ComputerTargetResolutionError::NoWindowMatch {
            app_id,
            window_title,
        } => json!({
            "code": "computer_window_not_found",
            "appName": app_id,
            "windowTitle": window_title,
            "nextAction": "Call computer_list_windows and retry with an available windowTitle."
        }),
        hachimi_computer::ComputerTargetResolutionError::AmbiguousWindow { app_id, titles } => {
            json!({
                "code": "computer_window_ambiguous",
                "appName": app_id,
                "windowTitles": titles,
                "nextAction": "User selection is required. Ask which window title they mean in an ordinary assistant follow-up, then retry with windowTitle. Never ask for a PID or window handle."
            })
        }
    };
    detail.to_string()
}

pub(super) const fn attention_code(
    error: &hachimi_computer::ComputerTargetResolutionError,
) -> Option<&'static str> {
    match error {
        hachimi_computer::ComputerTargetResolutionError::AmbiguousApplication { .. } => {
            Some("computer_application_ambiguous")
        }
        hachimi_computer::ComputerTargetResolutionError::AmbiguousWindow { .. } => {
            Some("computer_window_ambiguous")
        }
        hachimi_computer::ComputerTargetResolutionError::NoApplicationMatch { .. }
        | hachimi_computer::ComputerTargetResolutionError::NoWindowMatch { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguity_requires_structured_attention_without_an_unavailable_tool() {
        let error = hachimi_computer::ComputerTargetResolutionError::AmbiguousApplication {
            app_name: "Editor".into(),
            candidates: vec!["Code".into(), "Notepad".into()],
        };
        let message = resolution_message(&error);

        assert_eq!(
            attention_code(&error),
            Some("computer_application_ambiguous")
        );
        assert!(message.contains("ordinary assistant follow-up"));
        assert!(!message.contains("request_user_input"));
    }

    #[test]
    fn missing_target_remains_retryable_without_user_attention() {
        let error = hachimi_computer::ComputerTargetResolutionError::NoApplicationMatch {
            app_name: "Missing".into(),
        };

        assert_eq!(attention_code(&error), None);
        assert!(resolution_message(&error).contains("computer_list_windows"));
    }
}
