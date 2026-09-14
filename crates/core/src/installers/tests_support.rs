//! Test-only filesystem helpers shared by the installers halves.
//!
//! [`Scratch`], [`touch`], [`wine`] and [`install_steam`] were defined once in
//! the old `installers.rs` test module and used by every half's tests, so they
//! move here rather than being duplicated three times. `#[cfg(test)]`-gated
//! like the module that declares it, so no production build sees it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::runners::RunnerError;
use crate::runners::env::tests::FakeLaunchEnv;
use crate::runners::proton::ResponseHead;

/// A scratch directory that cleans itself up.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("gh-installers-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write a file, creating parents as needed.
pub fn touch(path: &Path) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, "stub").unwrap();
    path.to_path_buf()
}

pub fn wine() -> crate::runners::WineRunner {
    crate::runners::WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")))
}

/// Install `steam.exe` under `drive_c` and return it.
pub fn install_steam(drive_c: &Path) -> PathBuf {
    touch(&drive_c.join("Program Files (x86)/Steam/steam.exe"))
}

pub const PE_BODY: &[u8] = b"MZsafe-installer";

/// A client that answers with canned bytes and one declared length.
///
/// `delivered` counts what reached the sink, which is how a test shows the
/// origin check ran in the *head*: a refusal there must leave `delivered`
/// at zero. The reference's version of this test cannot see that — it mocks
/// `urlopen` and reads the whole body either way — so this is stronger than
/// a port of it.
pub struct FakeResponse {
    pub final_url: String,
    pub content_length: Option<String>,
    body: Vec<u8>,
    delivered: std::cell::Cell<usize>,
}

impl FakeResponse {
    /// A response from `url`'s own host, declaring its real length.
    pub fn at(url: &str, body: &[u8]) -> Self {
        Self {
            final_url: url.to_string(),
            content_length: Some(body.len().to_string()),
            body: body.to_vec(),
            delivered: std::cell::Cell::new(0),
        }
    }

    pub fn redirected_to(mut self, url: &str) -> Self {
        self.final_url = url.to_string();
        self
    }

    pub fn without_declared_length(mut self) -> Self {
        self.content_length = None;
        self
    }

    pub fn declaring(mut self, length: &str) -> Self {
        self.content_length = Some(length.to_string());
        self
    }

    pub fn delivered(&self) -> usize {
        self.delivered.get()
    }
}

impl crate::runners::proton::HttpClient for FakeResponse {
    fn get(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _timeout: Duration,
        on_head: &mut dyn FnMut(&ResponseHead) -> Result<(), RunnerError>,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), RunnerError>,
    ) -> Result<(), RunnerError> {
        on_head(&ResponseHead {
            content_length: self.content_length.clone(),
            final_url: self.final_url.clone(),
        })?;
        // Only reached when the head was accepted — which is what makes
        // `delivered` an observation about the head callback's answer.
        sink(&self.body)?;
        self.delivered.set(self.body.len());
        Ok(())
    }
}

/// A fake `osslsigncode` that records its argv and prints canned output.
///
/// A real script rather than a mock, and the values are baked into it
/// rather than read from the environment: `cargo test` runs cases in
/// threads of one process, so `std::env::set_var` in a test would be a race
/// against every other case. The record file is the evidence that the
/// verifier ran at all, which is what the reference asserts with
/// `verify.assert_not_called()`.
///
/// **Two subcommands**, because the publisher check reads the signature's
/// own PKCS#7 blob since `SEC-11`: `verify` prints `output` and exits
/// `exit`, and `extract-signature` writes a stand-in blob at its `-out`
/// argument and exits 0. The blob's *contents* are never parsed — the fake
/// `openssl` beside this answers for the certificates — but the file has to
/// exist, which is the one thing [`signed_subject_values`] checks about it.
/// A test that needs the extraction itself to fail uses
/// [`fake_osslsigncode_refusing_extraction`] instead.
pub fn fake_osslsigncode(directory: &Path, record: &Path, output: &str, exit: i32) -> PathBuf {
    fake_osslsigncode_with_extract(directory, record, output, exit, extract_signature_arm())
}

/// The `extract-signature` arm of a fake that succeeds: write a stand-in
/// blob wherever its `-out` points, then exit 0.
///
/// The `-out` value is the argument after `-out`, and the loop is POSIX `sh`
/// with no external commands, because the child has no `PATH`.
pub fn extract_signature_arm() -> &'static str {
    "  while [ $# -gt 0 ]; do\n\
     \x20   if [ \"$1\" = '-out' ]; then printf '%s\\n' 'signature-stub' > \"$2\"; fi\n\
     \x20   shift\n\
     \x20 done\n\
     \x20 exit 0\n"
}

/// A fake `osslsigncode` whose `extract-signature` fails, as the real one
/// does on a file with no signature at all (`No signature found`,
/// `Unable to extract existing signature`, exit 255).
///
/// Its own constructor rather than a flag on [`fake_osslsigncode`], so the
/// two behaviours cannot be confused at a call site: every case that gets
/// as far as the publisher check needs the successful one, and only the
/// failure-mode cases want this.
pub fn fake_osslsigncode_refusing_extraction(
    directory: &Path,
    record: &Path,
    output: &str,
    exit: i32,
) -> PathBuf {
    let arm = "  printf '%s\\n' 'No signature found' 'Unable to extract existing signature'\n\
               \x20 exit 255\n";
    fake_osslsigncode_with_extract(directory, record, output, exit, arm)
}

/// A fake `osslsigncode` whose `extract-signature` arm is `extract_arm`.
///
/// One script with two arms rather than two scripts, because the recording
/// and the `verify` answer have to be identical in both: `SEC-11` moved
/// part of the gate into a second subcommand of the same tool, so a fake
/// that answered only one of them would make every case that reaches the
/// publisher test look like a missing tool.
pub fn fake_osslsigncode_with_extract(
    directory: &Path,
    record: &Path,
    output: &str,
    exit: i32,
    extract_arm: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("osslsigncode");
    std::fs::create_dir_all(directory).unwrap();
    // `#!/bin/sh` plus the absolute path of the real shell, because
    // `run_capturing` now spawns with a cleared environment (`SEC-09`) and a
    // bare shebang would leave the kernel to resolve `sh` through a `PATH`
    // the child no longer has.
    let body = format!(
        "#!{shell}\n\
         printf '%s\\n' \"$@\" >> '{record}'\n\
         if [ \"$1\" = 'extract-signature' ]; then\n\
         {extract_arm}\
         fi\n\
         printf '%s\\n' '{output}'\n\
         exit {exit}\n",
        shell = fake_shell(),
        record = record.display(),
    );
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

/// `/bin/sh` as an absolute path, for the fakes' shebangs.
///
/// Hardcoded rather than looked up through `PATH`, because the whole point
/// is that the child has no `PATH` — and because a lookup here would make
/// the fakes depend on the developer's shell, which is the sort of hidden
/// input that turns a hermetic suite into one that passes on one machine.
pub fn fake_shell() -> &'static str {
    "/bin/sh"
}

/// A launch environment whose `osslsigncode` is `script`.
///
/// **No `openssl`**, which is what the refusals need: every case whose
/// chain gate fails — a bad signature, a missing verifier, an origin or
/// size refusal that never reaches the verifier — is decided before the
/// certificates are read, so it must not be able to depend on a reader
/// being present. A case that expects the *publisher* test to be reached
/// needs [`verifier_env_with_certificates`], and this one reaching
/// `CertificateReaderMissing` is itself asserted by a test.
pub fn verifier_env(script: &Path) -> FakeLaunchEnv {
    FakeLaunchEnv::new().with_which("osslsigncode", &script.to_string_lossy())
}

/// A launch environment with a fake `osslsigncode` **and** a fake `openssl`,
/// which is what every case that reaches the publisher test needs since
/// `SEC-11`: the publisher is read from the signature's certificates, so a
/// case that means "this signature is approved" has to say what the
/// certificates say.
///
/// `subjects` are those certificates, in the RFC2253 rendering the real
/// `openssl x509 -nameopt RFC2253` prints — which is also the rendering the
/// catalog strings are written in, so `CN=Valve Corp.,O=Valve Corp.,…` is
/// the form a Steam case wants. The verifier records its argv to `record`
/// and the reader records beside it with an `.openssl` suffix.
pub fn verifier_env_with_certificates(
    directory: &Path,
    record: &Path,
    output: &str,
    exit: i32,
    subjects: &[&str],
) -> FakeLaunchEnv {
    let verifier = fake_osslsigncode(directory, record, output, exit);
    let reader_record = PathBuf::from(format!("{}.openssl", record.display()));
    let reader = fake_openssl(directory, &reader_record, subjects, 0);
    verifier_env_with_reader(&verifier, &reader)
}

/// A launch environment whose verifier is `verifier` and whose reader is
/// `reader`, for the cases that need each of the two tools configured
/// differently — the failure modes of the extraction, where the whole point
/// is that one of them fails while the other would have answered.
pub fn verifier_env_with_reader(verifier: &Path, reader: &Path) -> FakeLaunchEnv {
    FakeLaunchEnv::new()
        .with_which("osslsigncode", &verifier.to_string_lossy())
        .with_which("openssl", &reader.to_string_lossy())
}

/// The real Steam signer's subject, as the real installer's certificate
/// prints it (`osslsigncode verify` and `openssl x509 -nameopt RFC2253`
/// agree byte for byte; measured on the downloaded `SteamSetup.exe`).
///
/// A named constant because several cases need a subject the Steam recipe
/// approves, and the point of `SEC-11` is that only the certificate's own
/// text can satisfy it.
pub const VALVE_CERTIFICATE: &str = "CN=Valve Corp.,O=Valve Corp.,L=Bellevue,ST=Washington,C=US";

/// A fake `openssl` that answers the two calls the publisher check makes.
///
/// `subjects` are the certificates the "signature" carries. The `pkcs7`
/// branch emits them as one PEM block per subject — the block body *is* the
/// subject text, because nothing here parses it as DER and a body a test can
/// read makes a failure legible — and the `x509` branch reads the single
/// block it was handed and prints that one `subject=` line. So the fake is
/// per-certificate the way the real pair is: three subjects mean three
/// blocks and three reads, and a case cannot match on a certificate that
/// was never in the blob.
///
/// `exit` is the status of the `pkcs7` call, so "the reader cannot read this
/// blob" is a case a test can set up; an empty `subjects` list is how a
/// reader that succeeds and finds nothing is set up.
///
/// A real script, for the reasons [`fake_osslsigncode`] gives, and with the
/// same constraint on its data: a subject containing a `'` would close the
/// quoting, so subjects here are distinguished names as `openssl` prints
/// them and never carry one.
///
/// Its `pkcs7` arm also dumps its own environment to `<record>.env`, the way
/// [`fake_osslsigncode_dumping_env`] does for the verifier: a second spawn
/// site is a second chance for the launcher's environment to leak (`SEC-09`),
/// and an observation of the child is worth more than "it goes through the
/// same helper". The dump goes to its own file so nothing this fake prints
/// to stdout changes for the cases that read the subjects.
pub fn fake_openssl(directory: &Path, record: &Path, subjects: &[&str], exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(directory).unwrap();
    let bundle: String = subjects
        .iter()
        .map(|subject| {
            format!(
                "printf '%s\\n' '-----BEGIN CERTIFICATE-----' '{subject}' '-----END CERTIFICATE-----'\n"
            )
        })
        .collect();
    let path = directory.join("openssl");
    let body = format!(
        "#!{shell}\n\
         printf '%s\\n' \"$@\" >> '{record}'\n\
         if [ \"$1\" = 'pkcs7' ]; then\n\
         {bundle}\
         export -p > '{record}.env'\n\
         exit {exit}\n\
         fi\n\
         if [ \"$1\" = 'x509' ]; then\n\
         shift\n\
         certificate=''\n\
         while [ $# -gt 0 ]; do\n\
         \x20 if [ \"$1\" = '-in' ]; then certificate=\"$2\"; fi\n\
         \x20 shift\n\
         done\n\
         while IFS= read -r line; do\n\
         \x20 case \"$line\" in\n\
         \x20   -----BEGIN*) ;;\n\
         \x20   -----END*) ;;\n\
         \x20   *) printf 'subject=%s\\n' \"$line\" ;;\n\
         \x20 esac\n\
         done < \"$certificate\"\n\
         exit 0\n\
         fi\n\
         exit 2\n",
        shell = fake_shell(),
        record = record.display(),
    );
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

/// A fake verifier that prints an approving line and then dumps its own
/// environment to `env_record` (`SEC-09`).
///
/// A separate helper rather than a flag on [`fake_osslsigncode`], because
/// the dump has to land in its own file: the other tests assert on the
/// merged stdout/stderr text and on the argv record, and adding lines to
/// either would change what they are reading.
pub fn fake_osslsigncode_dumping_env(
    directory: &Path,
    env_record: &Path,
    output: &str,
    exit: i32,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("osslsigncode");
    std::fs::create_dir_all(directory).unwrap();
    // `export -p` rather than `env`, because the shell's `export` is a
    // builtin: with `PATH` cleared there is nothing to resolve `env`, `cat`
    // or any other external command with, and the dump would be empty for
    // the wrong reason. `printf` is a builtin too.
    //
    // Note what this *cannot* show: `sh` sets `PWD`, `SHLVL` and `OLDPWD`
    // itself, so those three appear even in a completely empty environment.
    // The test excludes them by name rather than assuming an empty dump.
    //
    // The extraction arm is the same one every other fake answers with: the
    // publisher check drives it since `SEC-11`, so a fake that could only
    // dump its environment would leave this test refusing at the extraction
    // instead of reaching the environment it is about.
    let body = format!(
        "#!{shell}\nif [ \"$1\" = 'extract-signature' ]; then\n{arm}fi\nprintf '%s\\n' '{output}'\nexport -p > '{env_record}'\nexit {exit}\n",
        shell = fake_shell(),
        arm = extract_signature_arm(),
        output = output,
        env_record = env_record.display(),
    );
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}
