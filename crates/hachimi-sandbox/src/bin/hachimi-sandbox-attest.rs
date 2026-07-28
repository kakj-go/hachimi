use std::{env, path::PathBuf};

use hachimi_sandbox::{SandboxBackend, SandboxStatus, WindowsSandboxReadinessProbe};

fn main() {
    let mut arguments = env::args_os().skip(1);
    let mut marker = None;
    let mut launcher = None;
    let mut canary = None;
    let mut root = None;
    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--marker" => marker = arguments.next().map(PathBuf::from),
            "--launcher" => launcher = arguments.next().map(PathBuf::from),
            "--canary" => canary = arguments.next().map(PathBuf::from),
            "--root" => root = arguments.next().map(PathBuf::from),
            _ => {
                eprintln!("unknown sandbox attestation argument");
                std::process::exit(2);
            }
        }
    }
    let (Some(marker), Some(launcher), Some(canary), Some(root)) = (marker, launcher, canary, root)
    else {
        eprintln!("--marker, --launcher, --canary and --root are required");
        std::process::exit(2);
    };
    let report = WindowsSandboxReadinessProbe::new(marker)
        .with_runtime(launcher, canary, root)
        .capability_report();
    match serde_json::to_string(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    if SandboxStatus::from_report(&report) != SandboxStatus::Enforced {
        std::process::exit(1);
    }
}
