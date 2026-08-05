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
            "nextAction": "Call request_user_input and ask the user which application name they mean. Never ask for a PID or window handle."
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
                "nextAction": "Retry with windowTitle. If the user's goal does not identify one, call request_user_input and ask for the window title. Never ask for a PID or window handle."
            })
        }
    };
    detail.to_string()
}
