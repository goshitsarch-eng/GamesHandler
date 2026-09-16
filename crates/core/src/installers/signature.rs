//! The Authenticode half of the easy installer (`installers.py:533-598`).
//!
//! The publisher gate and the signature it reads — `authenticode_root_path`,
//! `verify_installer_authenticity` and the tool wrappers they sit on — split
//! from `download.rs` because the two concerns share only [`run_capturing`]:
//! the download half decides *where bytes come from*, this half decides
//! *whose signature they carry*. The boundary is the one the reference's own
//! function order already draws: everything past `_validate_installer_magic`
//! and before `download_installer` is the signature path.
//!
//! The machinery this file runs the tools through —
//! [`run_capturing`]/[`CommandOutput`]/[`RunFailure`] — lives in
//! [`super::process`], shared with `wizard.rs`'s prefix-idle poller, which
//! needs the same spawn/bound shape for a different child.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::installers::process::{CommandOutput, RunFailure, run_capturing};
use crate::installers::{Installer, InstallerError};
use crate::paths::Env;
use crate::runners::LaunchEnv;

/// The bundled Microsoft root, if this build can find it
/// (`installers.py:537-548`).
///
/// The reference's four candidates, in order. The second is
/// `Path(__file__).with_name(...)` — the `.pem` sitting beside the Python
/// module — and has no counterpart here because there is no Python module; the
/// third is the repository's `data/` directory relative to the module, which
/// translates to a path relative to this crate's source; the fourth and fifth
/// are the tarball install's XDG data location (`$XDG_DATA_HOME/gamehandler`,
/// then `~/.local/share/gamehandler`); the sixth is Flatpak's
/// `/app/share/gamehandler`.
///
/// The first candidate is the environment override, and it is the one that
/// makes this testable and packagable: `GAMEHANDLER_AUTHENTICODE_ROOT` pointing
/// at a real `.pem` satisfies the check without anything having to be installed
/// beside the binary.
pub fn authenticode_root_path(env: &dyn Env) -> Result<PathBuf, InstallerError> {
    let override_path = env
        .var("GAMEHANDLER_AUTHENTICODE_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = override_path {
        candidates.push(path);
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(AUTHENTICODE_ROOT_NAME),
    );
    // The tarball's install.sh copies the root to $PREFIX/share/gamehandler/
    // (default ~/.local), which is XDG data space — resolve it through
    // XDG_DATA_HOME with the ~/.local/share fallback rather than making a
    // tarball install depend on the environment override.
    if let Some(xdg) = env
        .var("XDG_DATA_HOME")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        candidates.push(
            Path::new(&xdg)
                .join("gamehandler")
                .join(AUTHENTICODE_ROOT_NAME),
        );
    }
    if let Some(home) = env
        .var("HOME")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        candidates.push(
            Path::new(&home)
                .join(".local/share/gamehandler")
                .join(AUTHENTICODE_ROOT_NAME),
        );
    }
    candidates.push(Path::new("/app/share/gamehandler").join(AUTHENTICODE_ROOT_NAME));
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(InstallerError::AuthenticodeRootUnavailable)
}

/// The filename the bundled root is expected under (`installers.py:534`).
pub const AUTHENTICODE_ROOT_NAME: &str = "microsoft-identity-verification-root-ca-2020.pem";

/// Seconds the signature verifier is given before it is killed
/// (`installers.py:586`).
const SIGNATURE_TIMEOUT: Duration = Duration::from_secs(90);

/// Verify the Authenticode chain and the expected publisher before execution
/// (`installers.py:569-595`).
///
/// # The argv, and why it is two inserts rather than a conditional tail
///
/// The reference builds `[verifier, "verify", "-in", path]` and then, when the
/// recipe pins the Microsoft root, splices two option *pairs* in at index 2 —
/// so they end up between the subcommand and `-in`, and the result is
/// `[verifier, "verify", "-CAfile", root, "-TSA-CAfile", root, "-in", path]`.
/// Building it in that order matters: appending the options at the end would
/// put them after the file argument, which `osslsigncode` parses differently.
///
/// # The two independent conditions
///
/// The chain must verify — the output must contain `Signature verification:
/// ok` **and** the exit status must be 0 — and one of the recipe's publisher
/// strings must appear in the subject of a certificate the signature carries.
///
/// # Where the publisher string is read from (SEC-03, SEC-11)
///
/// The reference tests the publisher against the *whole* merged output
/// (`installers.py:594-595`: `any(publisher.casefold() in folded ...)` where
/// `folded = output.casefold()`), and the merged output carries fields the
/// **signer** chooses, not the certificate: `osslsigncode` 2.14 prints
/// `Text description:` and `URL description:` out of the signature's
/// authenticated attributes, and `-n`/`-u` set them at signing time. Measured
/// against the bundled 2.14 binary, a self-signed payload signed with
/// `-n "Valve Corp."` produced an output containing `Text description: Valve
/// Corp.` under a `Subject:` of `CN=Totally Unrelated Signer,O=Evil Example
/// Ltd`, so the reference's predicate accepted a certificate that has nothing
/// to do with Valve.
///
/// Reading the `Subject:` *field* was this port's first answer to that
/// (`SEC-03`), and it is not enough: `subject_values` tested
/// `line.trim().strip_prefix("Subject:")`, so the prefix test ran **after** the
/// trim and accepted any line that trims to a `Subject:`. The signer's own
/// `-n` value is printed verbatim and may contain newlines, so `-n "$(printf
/// 'Totally Unrelated\n\t\tSubject: C=US,O=Valve Corp.,CN=Valve Corp.')"`
/// appends a line that is indistinguishable from a certificate subject — and
/// the trimmed shape it needs is exactly what `verify`'s own indentation
/// provides. Measured on the bundled 2.14: that command signs a payload with a
/// certificate whose subject is `C=US, O=Evil Example Ltd, CN=Totally Unrelated
/// Signer`, `verify -CAfile` exits 0, prints the forged `Subject: C=US,O=Valve
/// Corp.,CN=Valve Corp.` line, and the gate returned `Ok(())` for it.
///
/// So the publisher is now read from the **certificates themselves** and never
/// from the verifier's report: [`signed_subject_values`] pulls the signature's
/// PKCS#7 blob out of the file and asks `openssl` for the RFC2253 subject of
/// every certificate in it. Nothing the signer types can reach that path — the
/// only bytes parsed out of it are the certificate extensions the signature
/// covers.
///
/// **Both are deliberate divergences from the reference**, on the brief's
/// decision order: `installers.py:594-595` folds the output and searches all of
/// it, and a port that reproduced that would reproduce both holes. The message
/// the user sees is unchanged, so no P-item is affected.
///
/// # The catalog strings are written in `osslsigncode`'s rendering
///
/// What the divergence above does **not** change is the shape of the catalog
/// strings, because they were authored against the `Subject:` lines the
/// reference's own verifier printed. That rendering is
/// `X509_NAME_print_ex(…, XN_FLAG_RFC2253)`: fields in reverse order,
/// `+`/`,`-joined, no space after each comma, and `\,` inside a value that
/// contains one. `C=US,O=Valve Corp.,CN=Valve Corp.` is that form.
///
/// The consequence is measured, not assumed, on the **GOG recipe's own
/// installer**: its signer's certificate is `CN=GOG  sp. z o.o,O=GOG  sp. z
/// o.o,L=WARSZAWA,C=PL` in that rendering, and the catalog's
/// `CN=GOG  sp. z o.o,O=GOG  sp. z o.o` is a substring of it. The one-call
/// shortcut that `openssl pkcs7 -print_certs -noout` offers renders the *same*
/// certificate as `C=PL, L=WARSZAWA, O=GOG  sp. z o.o, CN=GOG  sp. z o.o` —
/// field order, space after each comma — of which the catalog string is **not**
/// a substring, so that shortcut would refuse a signature that is genuinely
/// GOG's. `openssl x509 -nameopt RFC2253` reproduces the verifier's rendering
/// byte for byte (checked on the real Steam, Amazon and GOG certificates,
/// including one with `\,`-escaped commas and a twelve-field DN), which is why
/// the extraction below pays for one call per certificate. "Tidying" the
/// catalog's double space or its field order still stops matching a signature
/// that is genuinely GOG's, exactly as before.
///
/// # The trust anchor is `osslsigncode`'s, not this function's
///
/// Nine of the ten recipes set no `microsoft_trust_root`, so no `-CAfile` is
/// passed and the chain is checked against the host's store. That is not the
/// hole the paragraph above closes: measured with the same binary, a
/// self-signed payload **without** `-CAfile` exits 1, prints `Error:
/// self-signed certificate` and `Signature verification: failed`, and never
/// prints the success line this function requires. A certificate the signer
/// minted for itself therefore fails before the publisher test is reached.
pub fn verify_installer_authenticity(
    installer: &Installer,
    path: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<(), InstallerError> {
    let Some(verifier) = launch_env.which("osslsigncode") else {
        return Err(InstallerError::SignatureToolMissing);
    };
    let mut command: Vec<String> = vec![
        verifier.to_string_lossy().into_owned(),
        "verify".to_string(),
        "-in".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    if installer.microsoft_trust_root {
        let root = authenticode_root_path(launch_env)?
            .to_string_lossy()
            .into_owned();
        command.splice(
            2..2,
            [
                "-CAfile".to_string(),
                root.clone(),
                "-TSA-CAfile".to_string(),
                root,
            ],
        );
    }

    let output = run_signature_tool(&command, installer.name)?;

    let text = output.text;
    if output.status != Some(0) || !text.contains("Signature verification: ok") {
        return Err(InstallerError::SignatureInvalid {
            name: installer.name.to_string(),
            tail: output_tail(&text),
        });
    }
    // The certificates themselves, and nothing the verifier printed about
    // them. See the doc comment: naming the `Subject:` field is not enough,
    // because the signer's `-n` value can contain a newline and a line that
    // trims to a `Subject:`.
    let subjects = signed_subject_values(installer, path, &verifier, launch_env)?;
    if !installer.publishers.iter().any(|publisher| {
        let publisher = publisher.to_lowercase();
        subjects.iter().any(|subject| subject.contains(&publisher))
    }) {
        return Err(InstallerError::PublisherUnapproved {
            name: installer.name.to_string(),
        });
    }
    Ok(())
}

/// The case-folded subject of every certificate the installer's signature
/// carries (`SEC-11`).
///
/// Three steps, none of which reads the verifier's human-readable report:
///
/// 1. `osslsigncode extract-signature -in <installer> -out <p7b>` — the
///    Authenticode signature as the PKCS#7 blob it is, with no rendering in
///    between. Fails (exit 255, `No signature found`) when the file carries no
///    signature at all.
/// 2. `openssl pkcs7 -inform DER -in <p7b> -print_certs` — the certificates
///    that blob carries, as a PEM bundle. The "catalog strings are written in
///    `osslsigncode`'s rendering` section of [`verify_installer_authenticity`]
///    has the measurement for why the per-certificate step that follows is
///    worth its subprocesses.
/// 3. `openssl x509 -in <one certificate> -noout -subject -nameopt RFC2253`,
///    once per certificate, because **`openssl x509` reads exactly one
///    certificate from its input**: handed a bundle it prints the first one and
///    stops (measured — a six-certificate bundle yields one `subject=` line).
///
/// Every failure is a refusal, never a pass, and none of them is a panic: the
/// bytes come from a downloaded file and the tools come from the host, so both
/// are treated as untrusted input and as a dependency that may be absent.
fn signed_subject_values(
    installer: &Installer,
    path: &Path,
    verifier: &Path,
    launch_env: &dyn LaunchEnv,
) -> Result<Vec<String>, InstallerError> {
    let name = installer.name;

    // Looked up before the scratch directory exists, because a host without
    // `openssl` has nothing to extract *for*. `openssl` is the reader, a
    // different dependency from the verifier: a host can have one without the
    // other, so a missing reader is its own error rather than
    // `SignatureToolMissing`, which means the verifier is absent.
    let Some(reader) = launch_env.which("openssl") else {
        return Err(InstallerError::CertificateReaderMissing);
    };

    // `path` is the downloaded file, and this is the one place in this module
    // that writes next to it rather than into the destination directory: the
    // blob is derived data, is read once, and must not be visible to another
    // local user while it exists.
    let scratch = SignatureScratch::create().map_err(InstallerError::Io)?;
    let signature = scratch.path().join("signature.p7b");
    let certificate = scratch.path().join("certificate.pem");
    let signature_arg = signature.to_string_lossy().into_owned();
    let certificate_arg = certificate.to_string_lossy().into_owned();
    let installer_arg = path.to_string_lossy().into_owned();

    // 1. The signature blob, as PKCS#7.
    let command = tool_argv(
        verifier,
        &[
            "extract-signature",
            "-in",
            &installer_arg,
            "-out",
            &signature_arg,
        ],
    );
    let output = run_signature_tool(&command, name)?;
    if output.status != Some(0) || !signature.is_file() {
        // The `is_file` half is not redundant: the check is that there is a
        // blob to read, and a tool that reports success without writing one
        // would otherwise hand `openssl` a path that does not exist — a
        // different error than the one the user needs to see.
        return Err(InstallerError::SignatureUnreadable {
            name: name.to_string(),
            tail: output_tail(&output.text),
        });
    }

    // 2. The certificates the blob carries, as PEM.
    let command = tool_argv(
        &reader,
        &[
            "pkcs7",
            "-inform",
            "DER",
            "-in",
            &signature_arg,
            "-print_certs",
        ],
    );
    let output = run_signature_tool(&command, name)?;
    let blocks = pem_certificates(&output.text);
    if output.status != Some(0) || blocks.is_empty() {
        return Err(InstallerError::CertificateReadFailed {
            name: name.to_string(),
            tail: output_tail(&output.text),
        });
    }

    // 3. Each certificate's subject, in the one rendering the catalog strings
    //    are written in (see [`verify_installer_authenticity`]). One
    //    `openssl x509` per certificate, and the file it reads is rewritten
    //    rather than duplicated: `openssl x509` will not read past the first
    //    certificate of a multi-certificate file, so handing it the whole
    //    bundle is not an option.
    let mut subjects = Vec::with_capacity(blocks.len());
    for block in &blocks {
        std::fs::write(&certificate, block).map_err(InstallerError::Io)?;
        let command = tool_argv(
            &reader,
            &[
                "x509",
                "-in",
                &certificate_arg,
                "-noout",
                "-subject",
                "-nameopt",
                "RFC2253",
            ],
        );
        let output = run_signature_tool(&command, name)?;
        if output.status != Some(0) {
            return Err(InstallerError::CertificateReadFailed {
                name: name.to_string(),
                tail: output_tail(&output.text),
            });
        }
        subjects.extend(x509_subject_values(&output.text));
    }

    if subjects.is_empty() {
        // Every certificate read cleanly and none of them had a subject, which
        // is not a certificate subject at all. Refused rather than returned as
        // an empty list, so "no subjects" can never read as "no constraint".
        return Err(InstallerError::CertificateReadFailed {
            name: name.to_string(),
            tail: String::new(),
        });
    }
    Ok(subjects)
}

/// The argv of a signature tool: the program, then its arguments.
///
/// The same shape [`run_capturing`] takes, built here rather than at each call
/// site because the three calls in [`signed_subject_values`] mix an owned path
/// with borrowed flags and none of them is more readable for spelling that out.
fn tool_argv(program: &Path, arguments: &[&str]) -> Vec<String> {
    let mut argv = vec![program.to_string_lossy().into_owned()];
    argv.extend(arguments.iter().map(|argument| (*argument).to_string()));
    argv
}

/// Run one of the signature tools, mapping its failure modes onto the errors
/// this module already reports.
///
/// The bound is the verifier's own [`SIGNATURE_TIMEOUT`], reused rather than
/// given a constant of its own: this is the same work on the same file, and a
/// second number would be a second thing to justify. A spawn that fails is
/// [`InstallerError::Io`], as it is for the verifier itself.
fn run_signature_tool(argv: &[String], name: &str) -> Result<CommandOutput, InstallerError> {
    match run_capturing(argv, SIGNATURE_TIMEOUT) {
        Ok(output) => Ok(output),
        Err(RunFailure::TimedOut) => Err(InstallerError::SignatureTimedOut {
            name: name.to_string(),
        }),
        Err(RunFailure::Failed(error)) => Err(error.into()),
    }
}

/// The last eight lines a tool printed, joined with newlines — what
/// [`InstallerError::SignatureInvalid`] has always carried, now shared with the
/// two errors the extraction can raise.
fn output_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(8)..].join("\n")
}

/// Every certificate in a PEM bundle, as its own PEM block.
///
/// `openssl pkcs7 -print_certs` writes one `-----BEGIN CERTIFICATE-----` block
/// per certificate, and this splits on those markers rather than on a line
/// count or a byte length — the bundle is whatever the file's signature
/// carries, so its size is not knowable in advance. A `BEGIN` seen before the
/// previous `END` means the output is not the bundle this expects, and the
/// partial block is dropped rather than concatenated onto the next one; a
/// truncated trailing block is dropped the same way, and
/// [`signed_subject_values`] then has one certificate fewer to match, never one
/// forged certificate more.
fn pem_certificates(text: &str) -> Vec<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line == BEGIN {
            current.clear();
            current.push(line);
        } else if !current.is_empty() {
            current.push(line);
            if line == END {
                blocks.push(current.join("\n"));
                current.clear();
            }
        }
    }
    blocks
}

/// The case-folded value of every `subject=` line in `openssl x509`'s output.
///
/// `-noout -subject` makes that the whole of its stdout: one `subject=<dn>` line
/// per certificate read, in the `KEY=VALUE` spelling `-nameopt RFC2253` gives
/// (`subject=CN=Valve Corp.,O=Valve Corp.,…`, no space around the `=`). The
/// split is on the **first** `=` and the label is matched case-insensitively,
/// so a value that itself contains an `=` keeps all of it, and a future spelling
/// change to the label is a mismatch rather than a silently empty list — which
/// is the failure that would matter, because "no subjects" must never read as
/// "no constraint".
///
/// A value is taken to the end of its line. `openssl` never wraps one: it
/// escapes a newline inside a name as `\0A` rather than printing it (measured
/// with a certificate whose CN contains one), so unlike the verifier's report —
/// whose `-n` continuation lines are what `SEC-11` was — an `openssl x509`
/// subject line cannot be continued by data the signer chose.
fn x509_subject_values(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(label, _)| label.trim().eq_ignore_ascii_case("subject"))
        .map(|(_, value)| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

/// The scratch directory the extracted signature is written to, removed when it
/// goes out of scope.
///
/// A directory rather than a single file, and `0700` rather than the default,
/// for the reason [`create_partial`](super::download::create_partial) gives about the download's `.part`: the
/// contents are attacker-influenced bytes that a later step parses, so no other
/// local user should be able to read or replace them. It is created under
/// `std::env::temp_dir()` — `$TMPDIR` or `/tmp`, the same choice the tests'
/// `Scratch` makes — and named with this process's id and a counter, so two
/// concurrent verifications cannot collide and a leftover from a crashed run
/// cannot be opened as if it were ours (`create` is `O_EXCL`-equivalent: it
/// fails on a path that already exists, including a symlink).
///
/// The [`Drop`] is what makes "cleaned up on every path" true rather than
/// repeated three times: every return in [`signed_subject_values`] is an
/// ordinary `?`/`return`, so the guard runs on success and on all three of the
/// failures alike — including the one where a tool timed out and the caller is
/// unwinding.
struct SignatureScratch(PathBuf);

impl SignatureScratch {
    fn create() -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        /// `TMP_MAX`-ish: `mkstemp` gives up after a bounded number of attempts.
        const ATTEMPTS: u32 = 1_000;
        /// The name's serial, per **process** rather than per call. Counting
        /// from 0 inside each call made the scheme correct only as long as one
        /// verification ran at a time: under the test suite two threads create
        /// these concurrently, and a directory freed by one's `Drop` could be
        /// recreated under the same name by the other — observed as a flake
        /// where an arm's `!exists()` assertion saw a sibling's directory.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let parent = std::env::temp_dir();
        let process = std::process::id();
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        for _ in 0..ATTEMPTS {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = parent.join(format!("gh-signature-{process}-{serial}"));
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create a temporary directory for the signature",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SignatureScratch {
    fn drop(&mut self) {
        // Best effort: a failure here must not mask the error that caused the
        // drop (`StagingDirectory` in `runners::proton` makes the same choice
        // for the same reason).
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::installers::installer_by_id;
    use crate::installers::tests_support::*;
    use crate::runners::env::tests::FakeLaunchEnv;

    /// `test_signature_requires_approved_publisher`
    /// (`tests/test_installers.py:135-145`) and
    /// `test_signature_accepts_verified_approved_publisher` (`:147-155`).
    ///
    /// Both arms in one test because they are one mechanism: the *only*
    /// difference between them is the publisher the signature names, so a test
    /// with one arm cannot tell "checks the publisher" from "rejects
    /// everything" or from "accepts everything".
    ///
    /// What the signature names is now read from its **certificates** rather
    /// than from the verifier's report, so every arm states it in
    /// `verifier_env_with_certificates`'s `subjects` and the report carries only
    /// the success line. That is deliberate: an arm that could pass by reading
    /// the report would be testing the hole `SEC-11` closed, and the third arm
    /// below is the one that pins the comparison's substring nature.
    #[test]
    fn the_signature_must_name_a_publisher_the_recipe_approves() {
        let scratch = Scratch::new("publisher");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        let approved = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-approved"),
            "Signature verification: ok",
            0,
            &[VALVE_CERTIFICATE],
        );
        assert!(verify_installer_authenticity(steam, &path, &approved).is_ok());

        let impostor = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-impostor"),
            "Signature verification: ok",
            0,
            &["CN=Impostor Corp.,O=Impostor Corp.,C=US"],
        );
        let error = verify_installer_authenticity(steam, &path, &impostor).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );

        // The substring nature of the publisher test, spelled out: a name that
        // merely *contains* an approved one is approved, and that is the
        // reference's behaviour rather than an accident of this port.
        let padded = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-padded"),
            "Signature verification: ok",
            0,
            &["CN=NotValve Corp.,O=NotValve Corp.,C=US"],
        );
        assert!(verify_installer_authenticity(steam, &path, &padded).is_ok());
    }

    /// **SEC-11** (and `SEC-03`'s subject, kept). The publisher must be the
    /// certificate's, so that signer-chosen text cannot supply one.
    ///
    /// The fixture reproduces, line for line, what the bundled `osslsigncode`
    /// 2.14 actually printed when a payload was signed with a certificate whose
    /// subject is `C=US, O=Evil Example Ltd, CN=Totally Unrelated Signer` and
    ///
    /// ```text
    /// -n "$(printf 'Totally Unrelated\n\t\tSubject: C=US,O=Valve Corp.,CN=Valve Corp.')"
    /// ```
    ///
    /// The `Text description:` value and the `Subject:` line below it are
    /// **one** authenticated attribute: the signer's text, continued on a line
    /// indented exactly as a certificate subject is. `subject_values` — the
    /// `line.trim().strip_prefix("Subject:")` that replaced `SEC-03`'s
    /// whole-output test — accepted that line, so the gate returned `Ok(())`
    /// for a binary signed by a certificate that has nothing to do with Valve.
    ///
    /// The `SEC-03` fixture could not see it: it put `Text description: {text}`
    /// on a single line, so the multi-line shape never occurred, and the
    /// verifier it drove was a fake whose report was the only thing it read.
    /// The multi-line shape *is* the attack, which is why it is spelled out
    /// here rather than described.
    ///
    /// The three arms isolate one variable each. The control differs from the
    /// attack in nothing but the forged line's **value** — same exit status,
    /// same success line, same genuine certificate — so a port that refused
    /// every report carrying a `Subject:`-shaped continuation would fail it. The
    /// third differs only in the **certificate**, and is the accepted-syntax
    /// arm: a report full of signer-chosen text is accepted when the
    /// certificate genuinely names an approved publisher, so the fix is not
    /// "refuse anything with a newline in it" either.
    #[test]
    fn a_signer_chosen_newline_cannot_forge_a_certificate_subject() {
        let scratch = Scratch::new("sec11-newline");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        /// The subject of the certificate the payload is really signed with.
        const UNRELATED: &str = "CN=Totally Unrelated Signer,O=Evil Example Ltd,C=US";

        // Printed by `osslsigncode verify`, trimmed to the lines that matter.
        // (The headings around them are omitted because the fake is a shell
        // script and `Signer's certificate:` carries an apostrophe that would
        // close its quoting.)
        let measured = |certificate: &str, forged: &str| {
            format!(
                "Signature Index: 0  (Primary Signature)\n\
                 \t\tSubject: {certificate}\n\
                 \t\tIssuer : CN=Totally Unrelated Signer,O=Evil Example Ltd,C=US\n\
                 Authenticated attributes:\n\
                 \tText description: Totally Unrelated\n\
                 \t\tSubject: {forged}\n\
                 \tMessage digest: EB9375AEDB7BC4E6CCF12645DC65BA23A4883FC22E0D27FC80ED2504922F938F\n\
                 Signature verification: ok\n\
                 Number of verified signatures: 1\n\
                 Signing certificate chain verified using:\n\
                 \t\tSubject: {certificate}\n"
            )
        };

        // The attack: an approved publisher that exists in the signer's text
        // and in no certificate. RFC2253 order with no space after the comma,
        // which is the form 2.14 prints and the form the catalog matches.
        let attack = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-attack"),
            &measured(UNRELATED, "C=US,O=Valve Corp.,CN=Valve Corp."),
            0,
            &[UNRELATED],
        );
        let error = verify_installer_authenticity(steam, &path, &attack).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );
        // The *variant*, not only the sentence: a refusal for a missing reader
        // or an unreadable blob would carry a different message and still fail
        // the assertion above, so the arm has to say which refusal it expects.
        assert!(matches!(error, InstallerError::PublisherUnapproved { .. }));

        // The control: the same output with an unrelated description, still
        // refused — so the arm above is not passing for the wrong reason.
        let control = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-control"),
            &measured(
                UNRELATED,
                "C=US,O=Totally Unrelated Ltd,CN=Totally Unrelated Ltd",
            ),
            0,
            &[UNRELATED],
        );
        let error = verify_installer_authenticity(steam, &path, &control).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam is not signed by an approved publisher"
        );

        // And a genuine Valve signature — described as something else, with a
        // forged subject line in that description too — is still accepted: the
        // text the signer controls is not read in either direction, and the
        // certificate the catalog names is. `VALVE_CERTIFICATE` is the real
        // signer's subject from the downloaded installer.
        let genuine = verifier_env_with_certificates(
            scratch.path(),
            &scratch.path().join("argv-genuine"),
            &measured(VALVE_CERTIFICATE, "C=US,O=Impostor Corp.,CN=Impostor Corp."),
            0,
            &[VALVE_CERTIFICATE],
        );
        verify_installer_authenticity(steam, &path, &genuine).unwrap();
    }

    /// The publisher check reads the **certificates**, so the three ways that
    /// can fail are refusals with their own names rather than passes: a
    /// signature that cannot be extracted, a host with no reader, and a blob
    /// the reader cannot make certificates out of.
    ///
    /// All three are fail-closed by construction, which is the property that
    /// makes them worth asserting: the publisher test needs the certificates, so
    /// "no certificates" must never read as "no constraint". Every arm here
    /// would be a `Ok(())` for a port that fell back to the verifier's printed
    /// output when the extraction failed — which is exactly the fallback that
    /// would reinstate `SEC-11`, and the reason there is none.
    #[test]
    fn a_publisher_cannot_be_read_from_a_signature_that_cannot_be_read() {
        let scratch = Scratch::new("sec11-unreadable");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let approved_report = "Signature verification: ok";
        // Each arm gets its own directory, so the two fakes — which both write
        // a script named after their tool — cannot be overwritten by a later
        // arm's setup.
        let dir = |label: &str| scratch.path().join(label);

        // 1. `extract-signature` fails: the file carries no signature, as the
        //    real tool reports for an unsigned payload. The reader is installed
        //    and would answer, so the refusal cannot be a missing `openssl`.
        let verifier = fake_osslsigncode_refusing_extraction(
            &dir("no-signature"),
            &dir("no-signature").join("argv"),
            approved_report,
            0,
        );
        let reader = fake_openssl(
            &dir("no-signature"),
            &dir("no-signature").join("argv.openssl"),
            &[VALVE_CERTIFICATE],
            0,
        );
        let env = verifier_env_with_reader(&verifier, &reader);
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam's Authenticode signature could not be read\n\
             No signature found\nUnable to extract existing signature"
        );
        assert!(matches!(error, InstallerError::SignatureUnreadable { .. }));

        // 2. No reader at all. The chain verified, so this is not a bad
        //    signature and not a missing verifier: it is the reader the
        //    publisher check needs, and it has a name of its own.
        let env = verifier_env(&fake_osslsigncode(
            &dir("no-reader"),
            &dir("no-reader").join("argv"),
            approved_report,
            0,
        ));
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert_eq!(
            error.to_string(),
            "openssl is required to read a downloaded installer's signing certificates"
        );
        assert!(matches!(error, InstallerError::CertificateReaderMissing));

        // 3. The reader exits non-zero on the blob — what the real
        //    `openssl pkcs7` does with a garbage `.p7b` (exit 1, an ASN.1
        //    error). A verifier that would otherwise pass, so the arm cannot be
        //    passing on the chain gate.
        let verifier = fake_osslsigncode(
            &dir("garbage"),
            &dir("garbage").join("argv"),
            approved_report,
            0,
        );
        let reader = fake_openssl(
            &dir("garbage"),
            &dir("garbage").join("argv.openssl"),
            &[],
            1,
        );
        let env = verifier_env_with_reader(&verifier, &reader);
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
        assert!(
            error
                .to_string()
                .starts_with("Steam's signing certificates could not be read"),
            "{error}"
        );

        // 4. The reader exits **zero** and produces no certificate — a blob
        //    that parses and carries none. Its own arm because it is the shape
        //    a "the reader exited 0, so trust it" port would let through with an
        //    empty subject list, which would make every publisher test vacuous
        //    rather than refused.
        let env = verifier_env_with_certificates(
            &dir("empty"),
            &dir("empty").join("argv"),
            approved_report,
            0,
            &[],
        );
        let error = verify_installer_authenticity(steam, &path, &env).unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
    }

    /// The extracted signature is a file with attacker-influenced bytes in it,
    /// and it is removed on every path — success, a failed extraction, and a
    /// failed read.
    ///
    /// The path is not guessed: it is the `-out` argument the fake verifier
    /// recorded, so this asserts about the directory the code actually created.
    /// A `Drop` guard is what makes it true for all three paths at once; the
    /// three arms are here because "all three" is the claim.
    #[test]
    fn the_extracted_signature_is_removed_from_disk_on_every_path() {
        let scratch = Scratch::new("sec11-scratch");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let approved_report = "Signature verification: ok";

        // Where the extraction wrote, according to the extraction itself.
        let extraction_directory = |record: &Path| -> PathBuf {
            let recorded = std::fs::read_to_string(record).unwrap();
            let lines: Vec<&str> = recorded.lines().collect();
            let out = lines
                .iter()
                .position(|line| *line == "-out")
                .expect("the extraction was asked for an output file");
            let blob = Path::new(lines[out + 1]);
            assert!(
                blob.ends_with("signature.p7b"),
                "the recorded output is not the extraction's blob: {blob:?}"
            );
            let directory = blob.parent().unwrap().to_path_buf();
            assert!(directory.is_absolute(), "{directory:?}");
            directory
        };

        // The success path.
        let record = scratch.path().join("argv-ok");
        let env = verifier_env_with_certificates(
            scratch.path(),
            &record,
            approved_report,
            0,
            &[VALVE_CERTIFICATE],
        );
        verify_installer_authenticity(steam, &path, &env).unwrap();
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the call");

        // A failed extraction (the blob is written by the tool that then
        // reports failure only in the arm below, so this arm's tool never
        // writes one — the directory still has to go).
        let record = scratch.path().join("argv-refused");
        let verifier =
            fake_osslsigncode_refusing_extraction(scratch.path(), &record, approved_report, 0);
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv-refused.openssl"),
            &[VALVE_CERTIFICATE],
            0,
        );
        let error = verify_installer_authenticity(
            steam,
            &path,
            &verifier_env_with_reader(&verifier, &reader),
        )
        .unwrap_err();
        assert!(matches!(error, InstallerError::SignatureUnreadable { .. }));
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the refusal");

        // A failed read: the extraction wrote a blob, and the reader could not
        // read it.
        let record = scratch.path().join("argv-unreadable");
        let verifier = fake_osslsigncode(scratch.path(), &record, approved_report, 0);
        let reader = fake_openssl(scratch.path(), &record.with_extension("openssl"), &[], 1);
        let error = verify_installer_authenticity(
            steam,
            &path,
            &verifier_env_with_reader(&verifier, &reader),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            InstallerError::CertificateReadFailed { .. }
        ));
        let directory = extraction_directory(&record);
        assert!(!directory.exists(), "{directory:?} outlived the refusal");

        // And the guard's name is not shared with another case's: every
        // directory it creates is under the process's temp directory and is
        // named for this process, which is what keeps two concurrent
        // verifications — and a leftover from a crashed run — from being read
        // as each other's.
        assert!(directory.starts_with(std::env::temp_dir()), "{directory:?}");
    }

    /// The success line is required as well as the exit status, and the failure
    /// carries the last eight lines the tool printed.
    #[test]
    fn a_verifier_that_did_not_say_ok_is_a_failure_with_its_own_output() {
        let scratch = Scratch::new("bad-signature");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));

        // Exit 0 but no success line — the case a status check alone misses.
        let quiet = fake_osslsigncode(scratch.path(), &scratch.path().join("argv-quiet"), "", 0);
        let error = verify_installer_authenticity(steam, &path, &verifier_env(&quiet)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\n"
        );

        // A non-zero exit with output: the tail is what the user sees.
        let failing = fake_osslsigncode(
            scratch.path(),
            &scratch.path().join("argv-failing"),
            "Failed to open file",
            1,
        );
        let error =
            verify_installer_authenticity(steam, &path, &verifier_env(&failing)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Steam has an invalid Authenticode signature\nFailed to open file"
        );
    }

    /// `SEC-09`: the verifier runs with an empty environment, and the two
    /// conditions that make that safe hold.
    ///
    /// The assertion is an *observation of the child*, not of the source: a
    /// script prints its own exported environment, and the test asks whether any
    /// variable this test process carries reached it. Under the pre-fix body the
    /// answer is "most of them"; with `env_clear` it is none of them.
    ///
    /// Three names are excluded by construction rather than by luck: `sh` sets
    /// `PWD`, `SHLVL` and `OLDPWD` for itself, so they appear in the dump even
    /// when the environment it was given is empty. Asserting an empty dump
    /// instead would fail for a reason that has nothing to do with this fix.
    ///
    /// **Both children are asked**, not only the verifier: the publisher check
    /// spawns a second tool — the certificate reader — since `SEC-11`, and a
    /// second spawn site is a second chance for the same leak. The reader dumps
    /// its environment in its `pkcs7` arm, which is the arm the publisher test
    /// drives.
    #[test]
    fn the_verifier_is_spawned_without_the_launcher_s_environment() {
        let scratch = Scratch::new("env-clear");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let env_record = scratch.path().join("verifier-env");
        let reader_record = scratch.path().join("reader-env");
        let verifier = fake_osslsigncode_dumping_env(
            scratch.path(),
            &env_record,
            "Signature verification: ok",
            0,
        );
        // The report carries only the success line and the publisher comes from
        // the certificates, so no part of this test can pass by reading the
        // verifier's text (`SEC-11`) — and the call reaches both spawns.
        let reader = fake_openssl(scratch.path(), &reader_record, &[VALVE_CERTIFICATE], 0);
        let env = verifier_env_with_reader(&verifier, &reader);

        verify_installer_authenticity(steam, &path, &env).unwrap();

        const SET_BY_THE_SHELL_ITSELF: [&str; 4] = ["PWD", "SHLVL", "OLDPWD", "_"];
        let inherited = |dump: &Path, child: &str| {
            let dumped = std::fs::read_to_string(dump).unwrap();
            let names: Vec<&str> = dumped
                .lines()
                .filter_map(|line| line.strip_prefix("export "))
                .filter_map(|rest| rest.split('=').next())
                .collect();
            assert!(
                !names.is_empty(),
                "the fake {child} wrote no environment at all — the dump, not the \
                 environment, is what failed here"
            );
            let leaked: Vec<String> = std::env::vars()
                .map(|(name, _)| name)
                .filter(|name| !SET_BY_THE_SHELL_ITSELF.contains(&name.as_str()))
                .filter(|name| names.contains(&name.as_str()))
                .collect();
            let sample: Vec<&String> = leaked.iter().take(5).collect();
            assert!(
                leaked.is_empty(),
                "the {child} inherited {} variable(s) from the launcher, including {sample:?}",
                leaked.len()
            );
        };
        inherited(&env_record, "verifier");
        inherited(
            &PathBuf::from(format!("{}.env", reader_record.display())),
            "certificate reader",
        );

        // The two conditions that make clearing safe, each asserted where it can
        // be: the programs are resolved to absolute paths before the spawn, so
        // an empty `PATH` is not a problem for reaching them — asserted here by
        // the fact that the scripts above ran at all, since they are only
        // reachable by their own paths.
        assert!(verifier.is_absolute(), "{verifier:?}");
        assert!(reader.is_absolute(), "{reader:?}");
        // ...and everything the verifier needs is in the argv. The trust root is
        // the one input this could have got wrong, so it is checked rather than
        // argued: the pinned-root recipe passes an absolute `-CAfile`.
        let ubisoft = installer_by_id("ubisoft").unwrap();
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let argv_record = scratch.path().join("argv");
        let verifier = fake_osslsigncode(
            scratch.path(),
            &argv_record,
            "Signature verification: ok",
            0,
        );
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv.openssl"),
            &["CN=UBISOFT ENTERTAINMENT,O=UBISOFT ENTERTAINMENT,C=FR"],
            0,
        );
        let env = verifier_env_with_reader(&verifier, &reader)
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        verify_installer_authenticity(ubisoft, &path, &env).unwrap();
        let recorded = std::fs::read_to_string(&argv_record).unwrap();
        assert!(recorded.contains(&root.to_string_lossy().to_string()));
    }

    /// `shutil.which("osslsigncode")` finding nothing is its own error, and it
    /// is reached before any process is spawned.

    #[test]
    fn a_missing_verifier_is_an_error_that_names_the_dependency() {
        let scratch = Scratch::new("no-verifier");
        let steam = installer_by_id("steam").unwrap();
        let path = touch(&scratch.path().join("SteamSetup.exe"));
        let error = verify_installer_authenticity(steam, &path, &FakeLaunchEnv::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "osslsigncode is required to verify downloaded installers"
        );
    }

    /// The pinned root is spliced in **before** `-in`, which is where the
    /// reference puts it (`command[2:2] = [...]`).
    ///
    /// Ubisoft is the one recipe with `microsoft_trust_root`, so this is also
    /// the only place the two-kinds-of-argv difference is observable. The
    /// control arm is the Steam run above: its argv has three elements and no
    /// `-CAfile`, which a test of Ubisoft alone could not distinguish from "the
    /// option is always added and Steam's override failed".
    #[test]
    fn a_recipe_that_pins_the_microsoft_root_passes_it_to_the_verifier() {
        let scratch = Scratch::new("ca-file");
        let ubisoft = installer_by_id("ubisoft").unwrap();
        assert!(ubisoft.microsoft_trust_root);
        let root = touch(&scratch.path().join("microsoft-root.pem"));
        let record = scratch.path().join("argv");
        let script = fake_osslsigncode(scratch.path(), &record, "Signature verification: ok", 0);
        // The publisher lives in this certificate and not in the report, which
        // is why the two halves of the argv assertion below are made on a call
        // that gets past the publisher test (`SEC-11`).
        let reader = fake_openssl(
            scratch.path(),
            &scratch.path().join("argv.openssl"),
            &["CN=UBISOFT ENTERTAINMENT,O=UBISOFT ENTERTAINMENT,C=FR"],
            0,
        );
        let env = verifier_env_with_reader(&script, &reader)
            .with_vars(&[("GAMEHANDLER_AUTHENTICODE_ROOT", &root.to_string_lossy())]);
        let path = touch(&scratch.path().join("UbisoftConnectInstaller.exe"));

        verify_installer_authenticity(ubisoft, &path, &env).unwrap();

        let recorded = std::fs::read_to_string(&record).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], "verify");
        assert_eq!(lines[1], "-CAfile");
        assert_eq!(lines[2], root.to_string_lossy());
        assert_eq!(lines[3], "-TSA-CAfile");
        assert_eq!(lines[4], root.to_string_lossy());
        assert_eq!(lines[5], "-in");
        assert_eq!(lines[6], path.to_string_lossy());

        // The bundled candidate resolves in a source tree: `data/` in this
        // repository carries the root, so a build of this crate verifies
        // Ubisoft without any override. That is what the third candidate
        // (`<manifest>/../../data/<name>`) is for, and this is the assertion
        // that it points where the file actually is.
        //
        // The fourth candidate (`/app/share/gamehandler/<name>`) is the
        // installed-Flatpak path and cannot be exercised here; whether the
        // packaging puts the `.pem` there is a packaging question, and it is
        // reported rather than assumed.
        let bare = FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy());
        let bundled = authenticode_root_path(&bare).unwrap();
        assert_eq!(
            bundled,
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data")
                .join(AUTHENTICODE_ROOT_NAME)
        );
        assert!(bundled.is_file(), "{bundled:?}");

        // `AuthenticodeRootUnavailable` is therefore **unreachable in this
        // tree** — every candidate list ends at a file that exists — so it has
        // no control arm here. It is kept because it is the reference's
        // behaviour for an installed tree with no `data/` beside it, and
        // because the alternative (returning `Ok` with no root) would silently
        // drop the pin for the one recipe that asked for it. Named as an
        // uncovered arm rather than left to look covered.
    }
}
