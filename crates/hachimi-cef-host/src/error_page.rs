use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use cef::Errorcode;
use hachimi_protocol::{BrowserNavigationError, BrowserNavigationErrorKind};

pub fn navigation_error(
    error_code: Errorcode,
    description: String,
    failed_url: String,
) -> BrowserNavigationError {
    let kind = if error_code == Errorcode::NAME_NOT_RESOLVED {
        BrowserNavigationErrorKind::Dns
    } else if error_code == Errorcode::CONNECTION_REFUSED {
        BrowserNavigationErrorKind::ConnectionRefused
    } else if error_code == Errorcode::TIMED_OUT {
        BrowserNavigationErrorKind::Timeout
    } else if error_code == Errorcode::INTERNET_DISCONNECTED {
        BrowserNavigationErrorKind::Offline
    } else if error_code == Errorcode::CERT_COMMON_NAME_INVALID || is_certificate_error(error_code)
    {
        BrowserNavigationErrorKind::Tls
    } else if error_code == Errorcode::BLOCKED_BY_CLIENT
        || error_code == Errorcode::BLOCKED_BY_ADMINISTRATOR
        || error_code == Errorcode::BLOCKED_BY_RESPONSE
    {
        BrowserNavigationErrorKind::Blocked
    } else {
        BrowserNavigationErrorKind::Other
    };
    BrowserNavigationError {
        kind,
        code: raw_error_code(error_code),
        description,
        failed_url,
    }
}

fn is_certificate_error(error_code: Errorcode) -> bool {
    let code = raw_error_code(error_code);
    (-299..=-200).contains(&code)
}

fn raw_error_code(error_code: Errorcode) -> i32 {
    let raw: cef::sys::cef_errorcode_t = error_code.into();
    raw as i32
}

pub fn error_page(error: &BrowserNavigationError) -> String {
    let target =
        serde_json::to_string(&error.failed_url).unwrap_or_else(|_| "\"about:blank\"".into());
    let host = url::Url::parse(&error.failed_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| error.failed_url.clone());
    let title = match error.kind {
        BrowserNavigationErrorKind::Dns => "Address not found",
        BrowserNavigationErrorKind::Tls => "Your connection is not private",
        BrowserNavigationErrorKind::ConnectionRefused => "Connection refused",
        BrowserNavigationErrorKind::Timeout => "The site took too long to respond",
        BrowserNavigationErrorKind::Offline => "You are offline",
        BrowserNavigationErrorKind::Blocked => "Navigation blocked",
        BrowserNavigationErrorKind::Crashed => "Page crashed",
        BrowserNavigationErrorKind::Other => "This page could not be loaded",
    };
    let html = format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="color-scheme" content="light dark">
<title>{title}</title><style>
:root{{font:14px/1.5 system-ui,sans-serif;color-scheme:light dark}}body{{margin:0;display:grid;min-height:100vh;place-items:center;background:#f7f7f7;color:#202124}}main{{width:min(560px,calc(100% - 48px));padding:32px}}h1{{font-size:24px;margin:0 0 12px}}p{{color:#5f6368;overflow-wrap:anywhere}}code{{font:12px/1.4 ui-monospace,monospace}}button{{margin-top:16px;padding:8px 14px;border:1px solid #dadce0;border-radius:6px;background:#fff;color:#1a73e8;font:inherit;cursor:pointer}}@media(prefers-color-scheme:dark){{body{{background:#202124;color:#e8eaed}}p{{color:#bdc1c6}}button{{background:#303134;border-color:#5f6368;color:#8ab4f8}}}}
</style><main><h1>{title}</h1><p>{host}</p><p>{description}</p><code>{code}</code><br><button id="retry">Retry</button></main>
<script>document.getElementById('retry').onclick=()=>location.href={target};</script>"#,
        title = escape_html(title),
        host = escape_html(&host),
        description = escape_html(&error.description),
        code = error.code,
    );
    format!(
        "data:text/html;charset=utf-8;base64,{}",
        BASE64_STANDARD.encode(html)
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_errors_have_distinct_user_visible_kinds() {
        assert_eq!(
            navigation_error(
                Errorcode::NAME_NOT_RESOLVED,
                "dns".into(),
                "https://missing.invalid/".into()
            )
            .kind,
            BrowserNavigationErrorKind::Dns
        );
        assert_eq!(
            navigation_error(
                Errorcode::CERT_COMMON_NAME_INVALID,
                "certificate".into(),
                "https://example.com/".into()
            )
            .kind,
            BrowserNavigationErrorKind::Tls
        );
    }
}
