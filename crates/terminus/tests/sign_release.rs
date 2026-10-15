//! `scripts/sign-release.sh` run end to end under `/bin/sh` and under dash,
//! with a stand-in `gh` and a throwaway Ed25519 key.
//!
//! Each test runs once per shell, as `<test>::sh` and `<test>::dash`. Without
//! dash or ssh-keygen on `PATH` these tests fail; none is skipped.
//!
//! The real `gh` on the owner's machine is logged in to the real release, so
//! no test may reach it. The environment is cleared, `HOME` is the fixture's,
//! and `PATH` holds the stand-in `gh`'s folder, then a folder of links to the
//! system tools the script calls, and nothing else. In that folder
//! `ssh-keygen` is a wrapper that logs its arguments and runs the system one.
//!
//! The private half of the pinned release key exists only on the owner's
//! machine, so each fixture puts a throwaway key at
//! `$HOME/.ssh/crenshawdev-release` and runs the script beside a copy of
//! `install.sh` that pins the throwaway key in its place.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Every program the script runs that is not a shell builtin, besides `gh`
/// and `ssh-keygen`, which the fixture writes.
const TOOLS: [&str; 4] = ["mktemp", "rm", "grep", "sha256sum"];

/// The release key's public line, as `install.sh` pins it.
const PINNED_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q";

const PLATFORMS: [&str; 4] = [
    "linux-x86_64",
    "linux-aarch64",
    "macos-x86_64",
    "macos-aarch64",
];

const TAG: &str = "v0.1.0";

const REPO: [&str; 2] = ["--repo", "crenshawdev/terminus"];

/// The shell a test runs the script under.
#[derive(Clone, Copy)]
enum Shell {
    Sh,
    Dash,
}

impl Shell {
    fn path(self) -> PathBuf {
        match self {
            Shell::Sh => PathBuf::from("/bin/sh"),
            Shell::Dash => which("dash").expect(
                "dash is not installed; the sign-release.sh tests run under dash as well as /bin/sh and need it",
            ),
        }
    }
}

/// Each test as `<test>::sh` and `<test>::dash`.
macro_rules! both_shells {
    ($($test:ident),* $(,)?) => {$(
        mod $test {
            #[test]
            fn sh() {
                super::$test(super::Shell::Sh)
            }

            #[test]
            fn dash() {
                super::$test(super::Shell::Dash)
            }
        }
    )*};
}

/// Where `name` lives on this test process's `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

fn ssh_keygen() -> PathBuf {
    which("ssh-keygen").expect("ssh-keygen is not on PATH")
}

fn repo_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name)
}

/// The SHA-256 of `path` from the system `sha256sum`, as lowercase hex.
fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(output.status.success(), "sha256sum: {}", text(&output));
    String::from_utf8(output.stdout).unwrap()[..64].to_owned()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// One logged call, split into its arguments.
fn calls(log: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| line.split(' ').map(str::to_owned).collect())
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    shell: PathBuf,
    root: PathBuf,
    home: PathBuf,
    /// The draft's eight files, as the stand-in `gh` serves them.
    draft: PathBuf,
    /// Where the stand-in `gh` copies uploaded files.
    uploaded: PathBuf,
    /// The stand-in `gh`'s folder, first on `PATH`.
    fake: PathBuf,
    /// Links to [`TOOLS`] and the logging `ssh-keygen`, second on `PATH`.
    tools: PathBuf,
}

/// A draft of 0.1.0 with four binaries of distinct bytes and their correct
/// manifests, a throwaway key in HOME, and the script beside a copy of
/// `install.sh` that pins that key.
fn fixture(shell: Shell) -> Fixture {
    let shell = shell.path();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let home = root.join("home");
    let draft = root.join("draft");
    let uploaded = root.join("uploaded");
    let fake = root.join("fake");
    let tools = root.join("tools");
    let scripts = root.join("scripts");
    for folder in [
        &home.join(".ssh"),
        &draft,
        &uploaded,
        &fake,
        &tools,
        &scripts,
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }

    let key = home.join(".ssh/crenshawdev-release");
    let made = Command::new(ssh_keygen())
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"])
        .arg(&key)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(made.status.success(), "ssh-keygen: {}", text(&made));

    std::fs::copy(
        repo_file("scripts/sign-release.sh"),
        scripts.join("sign-release.sh"),
    )
    .unwrap();
    let shipped = std::fs::read_to_string(repo_file("install.sh")).unwrap();
    assert!(shipped.contains(PINNED_KEY));
    let fx = Fixture {
        _dir: dir,
        shell,
        root,
        home,
        draft,
        uploaded,
        fake,
        tools,
    };
    std::fs::write(
        fx.root.join("install.sh"),
        shipped.replace(PINNED_KEY, &fx.public_key()),
    )
    .unwrap();

    for platform in PLATFORMS {
        let binary = fx.draft.join(format!("terminus-{platform}"));
        std::fs::write(&binary, format!("a stand-in build for {platform}\n")).unwrap();
        std::fs::write(
            fx.draft.join(format!("terminus-{platform}.manifest")),
            format!("version 0.1.0\nsha256 {}\n", sha256(&binary)),
        )
        .unwrap();
    }

    let cp = which("cp").expect("cp is not on PATH");
    let gh = r#"#!/bin/sh
printf '%s\n' "$*" >> '@LOG@'
case $1/$2 in
    release/download)
        dir=
        while [ "$#" -gt 0 ]; do
            case $1 in --dir) dir=$2; shift ;; esac
            shift
        done
        exec '@CP@' '@DRAFT@'/* "$dir"/
        ;;
    release/upload)
        shift 3
        for file do
            [ ! -f "$file" ] || '@CP@' "$file" '@UPLOADED@'/ || exit 1
        done
        ;;
    release/edit) ;;
    *) printf 'stand-in gh: unexpected call: %s\n' "$*" >&2; exit 1 ;;
esac
"#
    .replace("@LOG@", &fx.gh_log().display().to_string())
    .replace("@CP@", &cp.display().to_string())
    .replace("@DRAFT@", &fx.draft.display().to_string())
    .replace("@UPLOADED@", &fx.uploaded.display().to_string());
    write_executable(&fx.fake.join("gh"), &gh);

    for tool in TOOLS {
        let target = which(tool).unwrap_or_else(|| panic!("{tool} is not on PATH"));
        std::os::unix::fs::symlink(target, fx.tools.join(tool)).unwrap();
    }
    let wrapper = format!(
        "#!/bin/sh\nprintf 'askpass=%s %s\\n' \"${{SSH_ASKPASS_REQUIRE-unset}}\" \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        fx.ssh_log().display(),
        ssh_keygen().display()
    );
    write_executable(&fx.tools.join("ssh-keygen"), &wrapper);
    fx
}

impl Fixture {
    /// The throwaway key's public line, as `install.sh` pins a key.
    fn public_key(&self) -> String {
        let public =
            std::fs::read_to_string(self.home.join(".ssh/crenshawdev-release.pub")).unwrap();
        public
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn gh_log(&self) -> PathBuf {
        self.root.join("gh.log")
    }

    fn ssh_log(&self) -> PathBuf {
        self.root.join("ssh-keygen.log")
    }

    fn gh_calls(&self) -> Vec<Vec<String>> {
        calls(&self.gh_log())
    }

    /// Each `ssh-keygen` call, with the `askpass=` value it saw first.
    fn ssh_calls(&self) -> Vec<Vec<String>> {
        calls(&self.ssh_log())
    }

    /// `sh scripts/sign-release.sh <tag>` from the fixture's root, under the
    /// test's shell, with the cleared environment.
    fn run(&self, tag: &str) -> Output {
        let path = std::env::join_paths([&self.fake, &self.tools]).unwrap();
        Command::new(&self.shell)
            .arg("scripts/sign-release.sh")
            .arg(tag)
            .current_dir(&self.root)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.home)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

/// A good draft: four manifests signed from stdin with the owner's key file
/// under `terminus-release`, each verified as `install.sh` verifies, then one
/// upload of the four signatures and one edit that publishes the release as
/// latest.
fn a_good_draft_is_signed_and_published(shell: Shell) {
    let fx = fixture(shell);
    let output = fx.run(TAG);
    assert!(output.status.success(), "{}", text(&output));

    let home = fx.home.display().to_string();
    let key = format!("{home}/.ssh/crenshawdev-release");
    let ssh = fx.ssh_calls();
    let signs: Vec<_> = ssh
        .iter()
        .filter(|call| call[1..3] == ["-Y", "sign"])
        .collect();
    assert_eq!(signs.len(), 4, "{ssh:?}");
    for call in signs {
        assert_eq!(
            call,
            &[
                "askpass=never",
                "-Y",
                "sign",
                "-f",
                &key,
                "-n",
                "terminus-release"
            ],
        );
    }
    let verifies: Vec<_> = ssh
        .iter()
        .filter(|call| call[1..3] == ["-Y", "verify"])
        .collect();
    assert_eq!(verifies.len(), 4, "{ssh:?}");
    for call in verifies {
        assert_eq!(call.len(), 11, "{call:?}");
        assert!(call[4].ends_with("/allowed_signers"), "{call:?}");
        assert_eq!(
            call[5..9],
            ["-I", "crenshawdev-release", "-n", "terminus-release"]
        );
        assert_eq!(call[9], "-s", "{call:?}");
    }
    assert_eq!(ssh.len(), 8, "{ssh:?}");

    let gh = fx.gh_calls();
    assert_eq!(gh.len(), 3, "{gh:?}");
    for call in &gh {
        assert!(call.windows(2).any(|pair| pair == REPO), "{call:?}");
    }
    assert_eq!(gh[0][..3], ["release", "download", TAG]);
    assert_eq!(gh[1][..3], ["release", "upload", TAG]);
    let named: Vec<_> = gh[1][3..7]
        .iter()
        .map(|path| path.rsplit('/').next().unwrap())
        .collect();
    let sigs: Vec<_> = PLATFORMS
        .iter()
        .map(|platform| format!("terminus-{platform}.manifest.sig"))
        .collect();
    assert_eq!(named, sigs);
    assert_eq!(gh[1][7..], ["--repo", "crenshawdev/terminus", "--clobber"]);
    assert_eq!(
        gh[2],
        [
            "release",
            "edit",
            TAG,
            "--repo",
            "crenshawdev/terminus",
            "--draft=false",
            "--prerelease=false",
            "--latest"
        ]
    );

    let allowed = fx.root.join("allowed_signers");
    std::fs::write(
        &allowed,
        format!("crenshawdev-release {}\n", fx.public_key()),
    )
    .unwrap();
    for platform in PLATFORMS {
        let verified = Command::new(ssh_keygen())
            .args(["-Y", "verify", "-f"])
            .arg(&allowed)
            .args(["-I", "crenshawdev-release", "-n", "terminus-release", "-s"])
            .arg(
                fx.uploaded
                    .join(format!("terminus-{platform}.manifest.sig")),
            )
            .env_clear()
            .stdin(
                std::fs::File::open(fx.draft.join(format!("terminus-{platform}.manifest")))
                    .unwrap(),
            )
            .output()
            .unwrap();
        assert!(verified.status.success(), "{platform}: {}", text(&verified));
    }
}

/// A tag the script cannot use is refused before `gh` or `ssh-keygen` runs,
/// so nothing that looks like a flag ever reaches `gh`.
fn a_bad_tag_is_refused(shell: Shell) {
    let fx = fixture(shell);
    for tag in ["0.1.0", "v0.1", "v0.01.0", "v0.1.0-rc1", "--repo=x/y", ""] {
        let output = fx.run(tag);
        assert!(!output.status.success(), "{tag}: {}", text(&output));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "terminus: usage: sh scripts/sign-release.sh v<X.Y.Z>\n",
            "{tag}"
        );
    }
    assert!(fx.gh_calls().is_empty(), "{:?}", fx.gh_calls());
    assert!(fx.ssh_calls().is_empty(), "{:?}", fx.ssh_calls());
}

/// Run the script on `fx`'s draft and assert it was refused before anything
/// was signed: non-zero, the draft downloaded and nothing more asked of `gh`,
/// no `ssh-keygen -Y sign` and nothing uploaded. Returns stderr.
fn assert_refused_before_signing(fx: &Fixture) -> String {
    let output = fx.run(TAG);
    assert!(!output.status.success(), "{}", text(&output));
    let gh = fx.gh_calls();
    assert_eq!(gh.len(), 1, "{gh:?}");
    assert_eq!(gh[0][..2], ["release", "download"]);
    let ssh = fx.ssh_calls();
    assert!(
        !ssh.iter().any(|call| call[1..3] == ["-Y", "sign"]),
        "{ssh:?}"
    );
    assert_eq!(std::fs::read_dir(&fx.uploaded).unwrap().count(), 0);
    String::from_utf8(output.stderr).unwrap()
}

/// A draft missing any one of its eight files is refused, naming the file.
fn a_draft_missing_a_file_is_refused(shell: Shell) {
    for platform in PLATFORMS {
        for name in [
            format!("terminus-{platform}"),
            format!("terminus-{platform}.manifest"),
        ] {
            let fx = fixture(shell);
            std::fs::remove_file(fx.draft.join(&name)).unwrap();
            let stderr = assert_refused_before_signing(&fx);
            assert_eq!(stderr, format!("terminus: the draft {TAG} has no {name}\n"));
        }
    }
}

/// A binary that is not the one its manifest names is refused before any
/// passphrase is asked for.
fn a_binary_that_does_not_match_its_manifest_is_refused(shell: Shell) {
    let fx = fixture(shell);
    let binary = fx.draft.join("terminus-linux-aarch64");
    let mut bytes = std::fs::read(&binary).unwrap();
    bytes.extend_from_slice(b"appended");
    std::fs::write(&binary, bytes).unwrap();
    let stderr = assert_refused_before_signing(&fx);
    assert!(
        stderr.starts_with("terminus: terminus-linux-aarch64: SHA-256 mismatch"),
        "{stderr}"
    );
}

/// A manifest `install.sh` would read as another version is never signed.
fn a_manifest_naming_another_version_is_refused(shell: Shell) {
    let fx = fixture(shell);
    let binary = fx.draft.join("terminus-macos-x86_64");
    std::fs::write(
        fx.draft.join("terminus-macos-x86_64.manifest"),
        format!("version 0.2.0\nsha256 {}\n", sha256(&binary)),
    )
    .unwrap();
    let stderr = assert_refused_before_signing(&fx);
    assert!(
        stderr.starts_with(
            "terminus: terminus-macos-x86_64.manifest: not exactly the lines version 0.1.0"
        ),
        "{stderr}"
    );
}

both_shells!(
    a_good_draft_is_signed_and_published,
    a_bad_tag_is_refused,
    a_draft_missing_a_file_is_refused,
    a_binary_that_does_not_match_its_manifest_is_refused,
    a_manifest_naming_another_version_is_refused,
);

/// The script reads the pinned key from `install.sh` and keeps no copy of it.
#[test]
fn sign_release_holds_no_copy_of_the_key() {
    let script = std::fs::read_to_string(repo_file("scripts/sign-release.sh")).unwrap();
    let base64 = PINNED_KEY.split(' ').nth(1).unwrap();
    assert!(!script.contains(base64));
}
