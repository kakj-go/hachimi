use std::{env, path::PathBuf};

fn main() {
    let mut arguments = env::args_os().skip(1);
    let mut marker = None;
    let mut launcher = None;
    let mut uninstall = false;
    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--marker" => marker = arguments.next().map(PathBuf::from),
            "--launcher" => launcher = arguments.next().map(PathBuf::from),
            "--uninstall" => uninstall = true,
            _ => {
                eprintln!("unknown sandbox setup argument");
                std::process::exit(2);
            }
        }
    }
    let Some(marker) = marker else {
        eprintln!("--marker is required");
        std::process::exit(2);
    };
    if uninstall {
        match hachimi_sandbox::uninstall_sandbox(&marker) {
            Ok(()) => return,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    let Some(launcher) = launcher else {
        eprintln!("--launcher is required");
        std::process::exit(2);
    };
    match hachimi_sandbox::install_sandbox_marker(&marker, &launcher) {
        Ok(installed) => match serde_json::to_string(&installed) {
            Ok(value) => println!("{value}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
