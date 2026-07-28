// SPDX-License-Identifier: Apache-2.0
// Adapted from openai/codex codex-rs/windows-sandbox-rs/sandbox_smoketests.py
// at commit 4c43465133428898aa84f0bfc02c306ed65fb66a.

#![cfg(windows)]

#[cfg(feature = "windows-smoke")]
use std::process::Stdio;

use hachimi_sandbox::{
    PathAccess, PathSecurityError, resolve_checkout_path, validate_checkout_root,
};
#[cfg(feature = "windows-smoke")]
use hachimi_sandbox::{
    deny_restricted_code_read, grant_restricted_code_access, install_sandbox_marker,
    prepare_git_mutation_acl, prepare_workspace_acl, restore_git_mutation_acl,
    run_restricted_process,
};

#[cfg(feature = "windows-smoke")]
fn release_launcher() -> String {
    std::env::var("HACHIMI_SANDBOX_LAUNCHER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_hachimi-sandbox-launcher").into())
}

#[cfg(feature = "windows-smoke")]
fn release_canary() -> String {
    std::env::var("HACHIMI_SANDBOX_CANARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_hachimi-sandbox-canary").into())
}

#[test]
fn windows_path_matrix_rejects_aliases_and_hard_links() {
    let directory = tempfile::tempdir().expect("directory");
    let root = validate_checkout_root(directory.path()).expect("NTFS root");
    std::fs::write(root.join("safe.txt"), "safe").expect("safe");
    assert!(resolve_checkout_path(&root, "safe.txt", PathAccess::Read, false).is_ok());
    assert!(resolve_checkout_path(&root, "SAFE.TXT", PathAccess::Read, false).is_ok());
    assert!(resolve_checkout_path(&root, ".\\safe.txt", PathAccess::Read, false).is_ok());
    assert!(resolve_checkout_path(&root, "./safe.txt", PathAccess::Read, false).is_ok());
    if let Some(short_root) = short_path(&root) {
        let normalized_short = validate_checkout_root(&short_root).expect("8.3 checkout alias");
        assert_eq!(
            normalized_short.to_string_lossy().to_lowercase(),
            root.to_string_lossy().to_lowercase()
        );
    }
    if let Ok(other_drive) = tempfile::tempdir_in(std::env::current_dir().expect("current dir")) {
        let other_root =
            validate_checkout_root(other_drive.path()).expect("second local NTFS root");
        assert!(other_root.is_absolute());
    }
    for path in [
        "..\\escape.txt",
        "C:\\escape.txt",
        "\\\\server\\share\\file.txt",
        "\\\\?\\C:\\file.txt",
        "\\\\.\\pipe\\hachimi",
        "\\\\?\\Volume{00000000-0000-0000-0000-000000000000}\\file.txt",
        "safe.txt:secret",
        "NUL.txt",
        "CONIN$.txt",
        "COM1.log",
        "LPT9.log",
        "trailing.",
        "trailing ",
        "%TEMP%\\file.txt",
    ] {
        assert!(
            resolve_checkout_path(&root, path, PathAccess::Read, false).is_err(),
            "{path}"
        );
    }
    std::fs::hard_link(root.join("safe.txt"), root.join("alias.txt")).expect("hard link");
    assert!(matches!(
        resolve_checkout_path(&root, "alias.txt", PathAccess::Write, false),
        Err(PathSecurityError::HardLink)
    ));

    let outside = tempfile::tempdir().expect("outside directory");
    std::fs::write(outside.path().join("outside.txt"), "outside").expect("outside file");
    if std::os::windows::fs::symlink_file(
        outside.path().join("outside.txt"),
        root.join("outside-link.txt"),
    )
    .is_ok()
    {
        assert!(matches!(
            resolve_checkout_path(&root, "outside-link.txt", PathAccess::Read, false),
            Err(PathSecurityError::ReparsePoint)
        ));
    }
    let outside_junction = root.join("outside-junction");
    if create_junction(&outside_junction, outside.path()) {
        assert!(matches!(
            resolve_checkout_path(
                &root,
                "outside-junction\\outside.txt",
                PathAccess::Read,
                false
            ),
            Err(PathSecurityError::ReparsePoint)
        ));
    }
    if std::os::windows::fs::symlink_dir(&root, outside.path().join("checkout-alias")).is_ok() {
        assert!(matches!(
            validate_checkout_root(&outside.path().join("checkout-alias")),
            Err(PathSecurityError::ReparsePoint)
        ));
    }

    // Codex's Windows sandbox smoke suite also probes a reparse point whose
    // target is on another drive. A release runner can opt into the same
    // matrix without writing an arbitrary drive root by providing a dedicated
    // local NTFS test directory.
    if let Some(other_drive_root) = std::env::var_os("HACHIMI_SANDBOX_OTHER_NTFS_ROOT") {
        let other_drive = tempfile::tempdir_in(other_drive_root).expect("other-drive fixture");
        std::fs::write(other_drive.path().join("outside.txt"), "outside")
            .expect("other-drive fixture file");
        let other_drive_junction = root.join("other-drive-junction");
        assert!(
            create_junction(&other_drive_junction, other_drive.path()),
            "release runner declared an alternate NTFS root but could not create a junction"
        );
        assert!(matches!(
            resolve_checkout_path(
                &root,
                "other-drive-junction\\outside.txt",
                PathAccess::Read,
                false
            ),
            Err(PathSecurityError::ReparsePoint)
        ));
    }

    // Repeatedly replace one leaf with a reparse point. Validation may see the safe file, the
    // reparse point, or a transient I/O error, but it must never return the outside target.
    let race_path = root.join("race.txt");
    std::fs::write(&race_path, "inside").expect("race fixture");
    if std::os::windows::fs::symlink_file(
        outside.path().join("outside.txt"),
        root.join("race-link.txt"),
    )
    .is_ok()
    {
        for index in 0..64 {
            let _ = std::fs::remove_file(&race_path);
            if index % 2 == 0 {
                let _ = std::fs::rename(root.join("race-link.txt"), &race_path);
            } else {
                let _ = std::fs::write(&race_path, "inside");
            }
            if let Ok(path) = resolve_checkout_path(&root, "race.txt", PathAccess::Read, false) {
                assert!(path.starts_with(&root), "reparse swap escaped to {path:?}");
            }
            if race_path.is_symlink() {
                let _ = std::fs::rename(&race_path, root.join("race-link.txt"));
            }
        }
    }
}

fn create_junction(link: &std::path::Path, target: &std::path::Path) -> bool {
    // Adapted from Codex windows-sandbox-rs/sandbox_smoketests.py::make_junction
    // at 4c43465133428898aa84f0bfc02c306ed65fb66a.
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.is_ok_and(|status| status.success()) && link.exists()
}

fn short_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let source = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let needed = unsafe { GetShortPathNameW(source.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut output = vec![0_u16; usize::try_from(needed).ok()?.saturating_add(1)];
    let written = unsafe {
        GetShortPathNameW(
            source.as_ptr(),
            output.as_mut_ptr(),
            u32::try_from(output.len()).ok()?,
        )
    };
    if written == 0 {
        return None;
    }
    output.truncate(usize::try_from(written).ok()?);
    Some(std::path::PathBuf::from(String::from_utf16_lossy(&output)))
}

#[cfg(feature = "windows-smoke")]
#[test]
#[ignore = "requires a Windows NTFS runner with ACL mutation rights"]
fn restricted_process_is_job_bound_and_obeys_checkout_acl() {
    assert!(
        is_elevated(),
        "Windows sandbox smoke requires an elevated administrator runner"
    );
    let directory = tempfile::tempdir().expect("directory");
    let launcher = release_launcher();
    let canary = release_canary();
    let marker = directory.path().join("setup.json");
    install_sandbox_marker(&marker, std::path::Path::new(&launcher)).expect("sandbox setup");
    grant_restricted_code_access(directory.path(), true).expect("restricted ACL");
    grant_restricted_code_access(
        std::path::Path::new(&canary)
            .parent()
            .expect("canary parent"),
        false,
    )
    .expect("canary execute ACL");
    let target = directory.path().join("allowed.txt");
    let write = std::process::Command::new(&launcher)
        .args(["--", canary.as_str(), "--touch"])
        .arg(&target)
        .current_dir(directory.path())
        .output()
        .expect("write canary");
    assert!(
        write.status.success(),
        "write canary failed with {:?}: {}",
        write.status.code(),
        String::from_utf8_lossy(&write.stderr)
    );
    assert!(target.is_file());
    let job = std::process::Command::new(&launcher)
        .args(["--", canary.as_str(), "--assert-job"])
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("job canary");
    assert!(
        job.status.success(),
        "job canary failed: {}",
        String::from_utf8_lossy(&job.stderr)
    );
    let child_job = std::process::Command::new(&launcher)
        .args([
            "--",
            canary.as_str(),
            "--spawn-child-assert-job",
            canary.as_str(),
        ])
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("child job canary");
    assert!(
        child_job.status.success(),
        "child process escaped the Job Object: {}",
        String::from_utf8_lossy(&child_job.stderr)
    );
    let sentinel_path = directory.path().join("host-handle-sentinel.txt");
    let sentinel = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&sentinel_path)
        .expect("sentinel handle");
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    let sentinel_handle = sentinel.as_raw_handle();
    assert_ne!(
        unsafe {
            SetHandleInformation(
                sentinel_handle.cast(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        },
        0,
        "failed to make the sentinel handle inheritable: {}",
        std::io::Error::last_os_error()
    );
    let handle_probe = run_restricted_process(
        std::path::Path::new(&canary),
        &[
            "--write-handle".into(),
            (sentinel_handle as usize).to_string().into(),
        ],
        directory.path(),
    )
    .expect("handle-list canary");
    assert_ne!(
        handle_probe, 0,
        "an inheritable host handle escaped the explicit handle allowlist"
    );
    drop(sentinel);
    assert_eq!(
        std::fs::read(&sentinel_path).expect("sentinel contents"),
        b"",
        "the AppContainer child wrote through an unlisted host handle"
    );
    let escaped_marker = directory.path().join("escaped-child.txt");
    let mut cancellable_tree = std::process::Command::new(&launcher)
        .args([
            "--",
            canary.as_str(),
            "--spawn-child-sleep-touch",
            canary.as_str(),
        ])
        .arg(&escaped_marker)
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("cancellable process tree");
    std::thread::sleep(std::time::Duration::from_millis(500));
    cancellable_tree.kill().expect("cancel launcher");
    let _ = cancellable_tree.wait();
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert!(
        !escaped_marker.exists(),
        "a restricted grandchild survived Job Object cancellation"
    );

    let forbidden = tempfile::tempdir().expect("forbidden directory");
    deny_restricted_code_read(forbidden.path()).expect("deny-read ACL");
    let forbidden_file = forbidden.path().join("secret.txt");
    std::fs::write(&forbidden_file, "secret").expect("forbidden fixture");
    let denied_read = std::process::Command::new(&launcher)
        .args(["--", canary.as_str(), "--read"])
        .arg(&forbidden_file)
        .current_dir(directory.path())
        .output()
        .expect("denied read canary");
    assert!(
        !denied_read.status.success(),
        "restricted process read a path without a Restricted Code ACL"
    );
    let denied_write = std::process::Command::new(&launcher)
        .args(["--", canary.as_str(), "--touch"])
        .arg(forbidden.path().join("write-denied.txt"))
        .current_dir(directory.path())
        .output()
        .expect("denied write canary");
    assert!(
        !denied_write.status.success(),
        "restricted process wrote outside its checkout ACL"
    );

    let listener =
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("network listener");
    let address = listener.local_addr().expect("listener address");
    let denied_network = std::process::Command::new(&launcher)
        .args(["--", canary.as_str(), "--network", &address.to_string()])
        .current_dir(directory.path())
        .output()
        .expect("network canary");
    assert!(
        !denied_network.status.success(),
        "AppContainer process connected without a network capability"
    );
}

#[cfg(feature = "windows-smoke")]
#[test]
#[ignore = "requires an elevated Windows NTFS runner with the installed AppContainer identity"]
fn linked_worktree_git_mutation_is_temporary_and_branch_scoped() {
    assert!(
        is_elevated(),
        "Windows sandbox smoke requires an elevated administrator runner"
    );
    let fixture = tempfile::tempdir().expect("fixture");
    let repository = fixture.path().join("repository");
    let worktree = fixture.path().join("linked-worktree");
    let run_temp = fixture.path().join("run-temp");
    std::fs::create_dir_all(&run_temp).expect("run temp");
    run_git(fixture.path(), &["init", "-b", "main", path(&repository)]);
    run_git(
        &repository,
        &["config", "user.name", "Hachimi Sandbox Test"],
    );
    run_git(
        &repository,
        &["config", "user.email", "sandbox@example.invalid"],
    );
    std::fs::write(repository.join("tracked.txt"), "initial\n").expect("tracked fixture");
    run_git(&repository, &["add", "tracked.txt"]);
    run_git(&repository, &["commit", "-m", "fixture"]);
    run_git(
        &repository,
        &["worktree", "add", "-b", "sandbox-linked", path(&worktree)],
    );
    let main_before = git_stdout(&repository, &["rev-parse", "main"]);
    let linked_before = git_stdout(&worktree, &["rev-parse", "HEAD"]);

    let launcher = std::path::PathBuf::from(release_launcher());
    let canary = std::path::PathBuf::from(release_canary());
    let marker = fixture.path().join("setup.json");
    install_sandbox_marker(&marker, &launcher).expect("sandbox setup");
    prepare_workspace_acl(&worktree, &run_temp, &canary).expect("read-only workspace ACL");
    let git = git_program();

    std::fs::write(worktree.join("tracked.txt"), "first mutation\n")
        .expect("first mutation fixture");
    assert!(
        !run_restricted_git(&launcher, &git, &worktree, &["add", "--", "tracked.txt"]),
        "linked-worktree index was writable before the temporary Git ACL"
    );

    let stage_acl = prepare_git_mutation_acl(&worktree).expect("temporary Git stage ACL");
    assert!(
        run_restricted_git(&launcher, &git, &worktree, &["add", "--", "tracked.txt"]),
        "restricted Git could not update the linked-worktree index"
    );
    restore_git_mutation_acl(&stage_acl).expect("restore read-only Git ACL after stage");
    assert!(
        !run_restricted_git(
            &launcher,
            &git,
            &worktree,
            &[
                "-c",
                "user.name=Hachimi Sandbox Test",
                "-c",
                "user.email=sandbox@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--no-verify",
                "-m",
                "must remain read only between leases",
            ],
        ),
        "linked-worktree metadata remained writable after the stage lease"
    );

    let commit_acl = prepare_git_mutation_acl(&worktree).expect("temporary Git commit ACL");
    assert!(
        run_restricted_git(
            &launcher,
            &git,
            &worktree,
            &[
                "-c",
                "user.name=Hachimi Sandbox Test",
                "-c",
                "user.email=sandbox@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--no-verify",
                "-m",
                "restricted linked commit",
            ],
        ),
        "restricted Git could not write the linked branch/object database"
    );
    restore_git_mutation_acl(&commit_acl).expect("restore read-only Git ACL after commit");

    let linked_after = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    assert_ne!(linked_after, linked_before, "linked branch did not advance");
    assert_eq!(
        git_stdout(&repository, &["rev-parse", "main"]),
        main_before,
        "fixed linked-worktree mutation changed the main branch"
    );
    run_git(
        &repository,
        &["cat-file", "-e", &format!("{linked_after}^{{commit}}")],
    );
    assert!(
        git_stdout(&repository, &["status", "--porcelain"]).is_empty(),
        "linked-worktree mutation changed the primary working tree/index"
    );

    std::fs::write(worktree.join("tracked.txt"), "after restore\n").expect("post-restore fixture");
    assert!(
        !run_restricted_git(&launcher, &git, &worktree, &["add", "--", "tracked.txt"]),
        "linked-worktree index remained writable after ACL restoration"
    );
    assert!(
        git_stdout(&worktree, &["diff", "--cached", "--name-only"]).is_empty(),
        "post-restore mutation unexpectedly reached the linked index"
    );
}

#[cfg(feature = "windows-smoke")]
fn run_restricted_git(
    launcher: &std::path::Path,
    git: &std::path::Path,
    cwd: &std::path::Path,
    arguments: &[&str],
) -> bool {
    let git_dir = std::path::PathBuf::from(git_stdout(cwd, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .expect("canonical linked-worktree Git directory");
    let common_dir = std::path::PathBuf::from(git_stdout(cwd, &["rev-parse", "--git-common-dir"]));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        cwd.join(common_dir)
    }
    .canonicalize()
    .expect("canonical Git common directory");
    let git_dir_relative = git_dir
        .strip_prefix(&common_dir)
        .expect("linked-worktree Git directory must be inside the common directory");
    let checkout_alias = SubstDrive::new(cwd).expect("temporary Git checkout alias");
    let common_dir_alias = SubstDrive::new(&common_dir).expect("temporary Git common alias");
    let git_dir_alias = std::path::Path::new(&common_dir_alias.root).join(git_dir_relative);
    let output = std::process::Command::new(launcher)
        .arg("--")
        .arg(git)
        .args([
            "--git-dir",
            path(&git_dir_alias),
            "--work-tree",
            checkout_alias.root.as_str(),
        ])
        .args(arguments)
        .current_dir(&checkout_alias.root)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            eprintln!(
                "restricted Git {arguments:?} failed with {}\nstdout: {}\nstderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            false
        }
        Err(error) => {
            eprintln!("restricted Git {arguments:?} could not start: {error}");
            false
        }
    }
}

#[cfg(feature = "windows-smoke")]
struct SubstDrive {
    drive: String,
    root: String,
}

#[cfg(feature = "windows-smoke")]
impl SubstDrive {
    fn new(target: &std::path::Path) -> Result<Self, String> {
        let drive = (b'P'..=b'Z')
            .rev()
            .map(|letter| format!("{}:", char::from(letter)))
            .find(|drive| !std::path::Path::new(&format!("{drive}\\")).exists())
            .ok_or_else(|| "no unused drive letter is available".to_owned())?;
        let output = std::process::Command::new("subst.exe")
            .arg(&drive)
            .arg(target)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "subst failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(Self {
            root: format!("{drive}\\"),
            drive,
        })
    }
}

#[cfg(feature = "windows-smoke")]
impl Drop for SubstDrive {
    fn drop(&mut self) {
        let _ = std::process::Command::new("subst.exe")
            .args([self.drive.as_str(), "/D"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(feature = "windows-smoke")]
fn git_program() -> std::path::PathBuf {
    let output = std::process::Command::new("where.exe")
        .arg("git.exe")
        .output()
        .expect("locate git.exe");
    assert!(
        output.status.success(),
        "git.exe is required by the smoke runner"
    );
    String::from_utf8(output.stdout)
        .expect("git.exe path")
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .map(std::path::PathBuf::from)
        .expect("git.exe path")
}

#[cfg(feature = "windows-smoke")]
fn run_git(cwd: &std::path::Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "windows-smoke")]
fn git_stdout(cwd: &std::path::Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git UTF-8 output")
        .trim()
        .to_owned()
}

#[cfg(feature = "windows-smoke")]
fn path(path: &std::path::Path) -> &str {
    path.to_str().expect("UTF-8 smoke path")
}

#[cfg(feature = "windows-smoke")]
fn is_elevated() -> bool {
    std::process::Command::new("net.exe")
        .args(["session"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
