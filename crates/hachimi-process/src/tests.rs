use std::{collections::BTreeMap, path::Path, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{
    CheckoutId, ClientId, ProcessSessionId, ProcessSessionRecord, ProcessStatus, RunId, SessionId,
};

use super::{ProcessError, ProcessLaunchSpec, ProcessRegistry};

fn record(root: &Path, interactive: bool, output_limit: u64) -> ProcessSessionRecord {
    ProcessSessionRecord {
        id: ProcessSessionId::random(),
        session_id: SessionId::from("session-process"),
        run_id: Some(RunId::from("run-process")),
        checkout_id: CheckoutId::from("checkout-process"),
        run_generation: Some(7),
        owner_client_id: ClientId("workbench".into()),
        command_summary: root.to_string_lossy().into_owned(),
        interactive,
        status: ProcessStatus::Starting,
        exit_code: None,
        output_limit_bytes: output_limit,
        created_at_ms: 1,
        updated_at_ms: 1,
        reconnect_expires_at_ms: None,
    }
}

fn powershell(command: &str) -> Vec<String> {
    vec![
        "powershell.exe".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        command.into(),
    ]
}

fn spec(root: &Path, command: Vec<String>, tty: bool, output_limit: usize) -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        record: record(root, tty, u64::try_from(output_limit).unwrap()),
        restricted_launcher: None,
        command,
        cwd: root.to_owned(),
        environment: BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
            (
                "SYSTEMROOT".into(),
                std::env::var("SYSTEMROOT").unwrap_or_default(),
            ),
        ]),
        tty,
        stream_stdin: tty,
        output_bytes_cap: output_limit,
        timeout: Some(Duration::from_secs(10)),
        size: hachimi_protocol::ProcessTerminalSize::default(),
        reconnect_ttl: Duration::from_millis(100),
    }
}

#[cfg(windows)]
#[tokio::test]
async fn pipe_output_is_byte_bounded_and_replayable() {
    let root = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let launched = registry
        .spawn(spec(
            root.path(),
            powershell("[Console]::Out.Write('abcde'); [Console]::Error.Write('12345')"),
            false,
            3,
        ))
        .await
        .unwrap();
    let snapshot = loop {
        let snapshot = registry
            .read(&launched.id, None, None, Some(Duration::from_secs(1)))
            .await
            .unwrap();
        if snapshot.closed {
            break snapshot;
        }
    };
    let decoded = snapshot
        .chunks
        .iter()
        .map(|chunk| STANDARD.decode(&chunk.delta_base64).unwrap())
        .collect::<Vec<_>>();
    assert!(decoded.contains(&b"abc".to_vec()));
    assert!(decoded.contains(&b"123".to_vec()));
    assert!(snapshot.chunks.iter().all(|chunk| chunk.cap_reached));
    assert_eq!(snapshot.process.status, ProcessStatus::Exited);
}

#[cfg(windows)]
#[tokio::test]
async fn pty_supports_stdin_resize_and_idempotent_write() {
    let root = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let launched = registry
        .spawn(spec(
            root.path(),
            powershell("$line=[Console]::In.ReadLine(); [Console]::Out.Write(('echo:' + $line))"),
            true,
            4096,
        ))
        .await
        .unwrap();
    registry
        .resize(
            &ClientId("workbench".into()),
            &launched.id,
            hachimi_protocol::ProcessTerminalSize {
                rows: 30,
                cols: 100,
            },
        )
        .await
        .unwrap();
    let input = STANDARD.encode(b"hello\r\n");
    registry
        .write_base64(
            &ClientId("workbench".into()),
            &launched.id,
            "write-1",
            Some(&input),
            false,
        )
        .await
        .unwrap();
    registry
        .write_base64(
            &ClientId("workbench".into()),
            &launched.id,
            "write-1",
            Some(&input),
            false,
        )
        .await
        .unwrap();
    let mut after = None;
    let mut output = Vec::new();
    loop {
        let snapshot = registry
            .read(&launched.id, after, None, Some(Duration::from_secs(2)))
            .await
            .unwrap();
        for chunk in snapshot.chunks {
            output.extend(STANDARD.decode(chunk.delta_base64).unwrap());
        }
        after = Some(snapshot.next_sequence);
        if snapshot.closed {
            break;
        }
    }
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("echo:hello"), "{text:?}");
    assert_eq!(text.matches("echo:hello").count(), 1);
}

#[cfg(windows)]
#[tokio::test]
async fn duplicate_handle_and_detached_ttl_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let registry = ProcessRegistry::default();
    let launch = spec(
        root.path(),
        powershell("Start-Sleep -Seconds 30"),
        false,
        1024,
    );
    let launched = registry.spawn(launch.clone()).await.unwrap();
    assert!(matches!(
        registry.spawn(launch).await,
        Err(ProcessError::AlreadyExists(_))
    ));
    registry.detach_owner(&ClientId("workbench".into())).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let final_record = registry.get(&launched.id).await.unwrap();
    assert!(matches!(
        final_record.status,
        ProcessStatus::Expired | ProcessStatus::Terminated
    ));
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires the standard-user Windows sandbox release environment"]
async fn terminal_conpty_uses_the_restricted_launcher_and_kills_its_process_tree() {
    let root = tempfile::tempdir().expect("terminal root");
    hachimi_sandbox::grant_restricted_code_access(root.path(), true).expect("terminal root ACL");
    let launcher = std::env::var_os("HACHIMI_SANDBOX_LAUNCHER")
        .map(std::path::PathBuf::from)
        .expect("launcher env");
    let registry = ProcessRegistry::default();

    let mut echo = spec(
        root.path(),
        powershell("$line=[Console]::In.ReadLine(); [Console]::Out.Write(('restricted:' + $line))"),
        true,
        4096,
    );
    echo.restricted_launcher = Some(launcher.clone());
    let launched = registry.spawn(echo).await.expect("restricted terminal");
    registry
        .resize(
            &ClientId("workbench".into()),
            &launched.id,
            hachimi_protocol::ProcessTerminalSize {
                rows: 32,
                cols: 100,
            },
        )
        .await
        .expect("resize");
    registry
        .write_base64(
            &ClientId("workbench".into()),
            &launched.id,
            "restricted-write",
            Some(&STANDARD.encode(b"hello\r\n")),
            false,
        )
        .await
        .expect("stdin");
    let mut output = Vec::new();
    let mut after = None;
    loop {
        let snapshot = registry
            .read(&launched.id, after, None, Some(Duration::from_secs(3)))
            .await
            .expect("read");
        for chunk in snapshot.chunks {
            output.extend(STANDARD.decode(chunk.delta_base64).expect("base64"));
        }
        after = Some(snapshot.next_sequence);
        if snapshot.closed {
            break;
        }
    }
    assert!(
        String::from_utf8_lossy(&output).contains("restricted:hello"),
        "restricted ConPTY did not round-trip stdin: {}",
        String::from_utf8_lossy(&output)
    );

    let marker = root.path().join("grandchild-survived.txt");
    let escaped_marker = marker
        .to_string_lossy()
        .replace('`', "``")
        .replace('"', "`\"");
    let command = format!(
        "$child=Start-Process powershell.exe -PassThru -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 3; Set-Content -LiteralPath \"{escaped_marker}\" -Value escaped'; Start-Sleep -Seconds 30"
    );
    let mut tree = spec(root.path(), powershell(&command), false, 4096);
    tree.restricted_launcher = Some(launcher);
    let tree = registry.spawn(tree).await.expect("restricted tree");
    tokio::time::sleep(Duration::from_millis(500)).await;
    registry
        .terminate(&ClientId("workbench".into()), &tree.id)
        .await
        .expect("terminate tree");
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        !marker.exists(),
        "restricted terminal grandchild survived termination"
    );
}
