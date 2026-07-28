use std::{env, ffi::OsString, path::PathBuf};

fn main() {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        eprintln!("usage: hachimi-sandbox-launcher -- <program> [args...]");
        std::process::exit(2);
    }
    let Some(executable) = arguments.next().map(PathBuf::from) else {
        eprintln!("sandbox executable is missing");
        std::process::exit(2);
    };
    let child_arguments = arguments.collect::<Vec<OsString>>();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match hachimi_sandbox::run_restricted_process(&executable, &child_arguments, &cwd) {
        Ok(code) => match i32::try_from(code) {
            Ok(code) => std::process::exit(code),
            Err(_) => {
                eprintln!("restricted process exited with Windows status 0x{code:08X}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(126);
        }
    }
}
