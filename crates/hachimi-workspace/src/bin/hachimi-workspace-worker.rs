use std::{env, io::BufRead, path::PathBuf};

use hachimi_workspace::{
    SearchServerRequest, WORKER_TOKEN_ENV, WatchServerRequest, WorkerContext, WorkspaceErrorCode,
    WorkspaceErrorRecord, WorkspaceRequestEnvelope, WorkspaceResponseEnvelope,
};
#[tokio::main]
async fn main() {
    if env::args_os().any(|argument| argument == "--search-server") {
        if let Err(error) = run_search() {
            eprintln!("{error}");
            std::process::exit(2);
        }
        return;
    }
    if env::args_os().any(|argument| argument == "--watch-server") {
        if let Err(error) = run_watch() {
            eprintln!("{error}");
            std::process::exit(2);
        }
        return;
    }
    let fallback_request_id = "invalid-request".to_owned();
    let result = run().await;
    let response = match result {
        Ok(response) => response,
        Err(error) => WorkspaceResponseEnvelope {
            request_id: fallback_request_id,
            output: None,
            error: Some(WorkspaceErrorRecord {
                code: WorkspaceErrorCode::InvalidRequest,
                message: error,
            }),
        },
    };
    match serde_json::to_string(&response) {
        Ok(encoded) => println!("{encoded}"),
        Err(_) => std::process::exit(2),
    }
}

fn run_search() -> Result<(), String> {
    let arguments = Arguments::parse()?;
    let token = env::var(WORKER_TOKEN_ENV).map_err(|_| "worker token is missing".to_owned())?;
    let context = WorkerContext::new_with_alias(
        arguments.root,
        arguments.root_identity,
        arguments.checkout_id,
        arguments.generation,
        token,
    )
    .map_err(|error| format!("workspace root validation failed: {error}"))?;
    let mut input = std::io::BufReader::new(std::io::stdin());
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if line.len() > 64 * 1024 {
        return Err("search request exceeds the 64 KiB protocol limit".into());
    }
    let request: SearchServerRequest =
        serde_json::from_str(&line).map_err(|error| error.to_string())?;
    hachimi_workspace::run_search_server(&context, &request, input, std::io::stdout())
        .map_err(|error| error.to_string())
}

fn run_watch() -> Result<(), String> {
    let arguments = Arguments::parse()?;
    let token = env::var(WORKER_TOKEN_ENV).map_err(|_| "worker token is missing".to_owned())?;
    let context = WorkerContext::new_with_alias(
        arguments.root,
        arguments.root_identity,
        arguments.checkout_id,
        arguments.generation,
        token,
    )
    .map_err(|error| format!("workspace root validation failed: {error}"))?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if line.len() > 64 * 1024 {
        return Err("watch request exceeds the 64 KiB protocol limit".into());
    }
    let request: WatchServerRequest =
        serde_json::from_str(&line).map_err(|error| error.to_string())?;
    hachimi_workspace::run_watch_server(&context, &request, std::io::stdout())
        .map_err(|error| error.to_string())
}

async fn run() -> Result<WorkspaceResponseEnvelope, String> {
    let arguments = Arguments::parse()?;
    let token = env::var(WORKER_TOKEN_ENV).map_err(|_| "worker token is missing".to_owned())?;
    let context = WorkerContext::new_with_alias(
        arguments.root,
        arguments.root_identity,
        arguments.checkout_id,
        arguments.generation,
        token,
    )
    .map_err(|error| format!("workspace root validation failed: {error}"))?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if line.len() > 4 * 1024 * 1024 {
        return Err("worker request exceeds the 4 MiB protocol limit".into());
    }
    let request: WorkspaceRequestEnvelope =
        serde_json::from_str(&line).map_err(|error| error.to_string())?;
    Ok(context.handle(request).await)
}

struct Arguments {
    root: PathBuf,
    root_identity: Option<hachimi_sandbox::WindowsFileIdentity>,
    checkout_id: String,
    generation: u64,
}

impl Arguments {
    fn parse() -> Result<Self, String> {
        let mut arguments = env::args_os().skip(1);
        let mut root = None;
        let mut root_volume_serial = None;
        let mut root_file_id = None;
        let mut checkout_id = None;
        let mut generation = None;
        while let Some(argument) = arguments.next() {
            match argument.to_string_lossy().as_ref() {
                "--root" => root = arguments.next().map(PathBuf::from),
                "--root-volume-serial" => {
                    root_volume_serial = arguments
                        .next()
                        .and_then(|value| value.to_string_lossy().parse::<u32>().ok());
                }
                "--root-file-id" => {
                    root_file_id = arguments
                        .next()
                        .and_then(|value| value.to_string_lossy().parse::<u64>().ok());
                }
                "--checkout-id" => {
                    checkout_id = arguments
                        .next()
                        .map(|value| value.to_string_lossy().into_owned());
                }
                "--generation" => {
                    generation = arguments
                        .next()
                        .and_then(|value| value.to_string_lossy().parse::<u64>().ok());
                }
                "--watch-server" | "--search-server" => {}
                _ => return Err("unknown workspace worker argument".into()),
            }
        }
        let root_identity = match (root_volume_serial, root_file_id) {
            (Some(volume_serial_number), Some(file_index)) => {
                Some(hachimi_sandbox::WindowsFileIdentity {
                    volume_serial_number,
                    file_index,
                })
            }
            (None, None) => None,
            _ => return Err("root identity arguments are incomplete".into()),
        };
        Ok(Self {
            root: root.ok_or_else(|| "--root is required".to_owned())?,
            root_identity,
            checkout_id: checkout_id.ok_or_else(|| "--checkout-id is required".to_owned())?,
            generation: generation.ok_or_else(|| "--generation is required".to_owned())?,
        })
    }
}
