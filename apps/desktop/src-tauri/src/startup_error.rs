use std::{fmt::Display, path::Path};

pub(super) fn show(error: &dyn Display, log_dir: &Path) {
    tracing::error!(code = "HCH-STARTUP-001", %error, "Hachimi core startup failed");
    let message = format!(
        "Hachimi 无法启动必要的本地运行环境。\n错误码：HCH-STARTUP-001\n日志位置：{}\n\nHachimi could not initialize its required local runtime.\nError code: HCH-STARTUP-001",
        log_dir.display()
    );
    let _ = rfd::MessageDialog::new()
        .set_title("Hachimi")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
