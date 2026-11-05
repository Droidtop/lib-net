//! SFTP over SSH: any SSH server, the person's own computer, a NAS, a Steam Deck (Droidtop/tracker#573
//! design D2, item 2).
//!
//! russh and russh-sftp, with ring for the cryptography (the same as the rest of the programs here; not
//! russh's default aws-lc-rs). They are async on tokio; [`Sftp`] runs them on a single-threaded runtime
//! of its own, so this backend is blocking like the others.
//!
//! **The server's key is pinned.** [`SftpConfig::host_key`] is the key's SHA-256 fingerprint as OpenSSH
//! prints it (`SHA256:...`). Without one, connecting fails with [`Error::HostKeyUnknown`] carrying the
//! server's fingerprint, for the person to check against the server and accept; with a different one, it
//! fails with [`Error::HostKeyChanged`]. A key is never trusted silently.
//!
//! **Writes in one step where the server can.** A file is written to a temporary name and renamed onto
//! its target. SFTP version 3 refuses a rename onto a file that exists, so the target is removed first:
//! a reader can find the target missing for that moment, never half written.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::StatusCode;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::Runtime;

use crate::{temporary_name, Entry, Error, Remote, RemotePath, Result};

/// What an SFTP share is: the server, the person's own sign-in, and the folder the share starts at.
#[derive(Clone)]
#[non_exhaustive]
pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    /// A private key in OpenSSH's format, used instead of or before the password.
    pub private_key: Option<String>,
    /// The private key's passphrase, when it has one.
    pub key_passphrase: Option<String>,
    /// The folder on the server the share starts at: absolute, or relative to the user's home. Empty is home.
    pub folder: String,
    /// The server key's SHA-256 fingerprint, as accepted by the person (`SHA256:...`).
    pub host_key: Option<String>,
    /// How long to wait for the server before it counts as unreachable.
    pub timeout: Duration,
}

impl SftpConfig {
    pub fn new(host: impl Into<String>, user: impl Into<String>) -> SftpConfig {
        SftpConfig {
            host: host.into(),
            port: 22,
            user: user.into(),
            password: None,
            private_key: None,
            key_passphrase: None,
            folder: String::new(),
            host_key: None,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn port(mut self, port: u16) -> SftpConfig {
        self.port = port;
        self
    }

    pub fn password(mut self, password: impl Into<String>) -> SftpConfig {
        self.password = Some(password.into());
        self
    }

    pub fn private_key(mut self, key: impl Into<String>, passphrase: Option<String>) -> SftpConfig {
        self.private_key = Some(key.into());
        self.key_passphrase = passphrase;
        self
    }

    pub fn folder(mut self, folder: impl Into<String>) -> SftpConfig {
        self.folder = folder.into();
        self
    }

    pub fn host_key(mut self, fingerprint: impl Into<String>) -> SftpConfig {
        self.host_key = Some(fingerprint.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> SftpConfig {
        self.timeout = timeout;
        self
    }
}

impl std::fmt::Debug for SftpConfig {
    // Neither the password nor the key is ever printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("private_key", &self.private_key.as_ref().map(|_| "…"))
            .field("folder", &self.folder)
            .field("host_key", &self.host_key)
            .finish()
    }
}

/// Checks the server's key against the pinned one, and keeps what the server showed.
struct KeyCheck {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

/// A key's SHA-256 fingerprint as OpenSSH prints it.
fn fingerprint(key: &PublicKeyOrCertificate) -> String {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => key.key_data().fingerprint(HashAlg::Sha256).to_string(),
        PublicKeyOrCertificate::Certificate(cert) => cert.public_key().fingerprint(HashAlg::Sha256).to_string(),
    }
}

impl client::Handler for KeyCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_public_key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let found = fingerprint(server_public_key);
        let matches = self.expected.as_deref() == Some(found.as_str());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
        Ok(matches)
    }
}

/// An SFTP share, connected.
pub struct Sftp {
    runtime: Runtime,
    sftp: SftpSession,
    // Kept so the SSH connection lives as long as the share.
    _session: Handle<KeyCheck>,
    /// The share's folder on the server, without a trailing `/`; empty for the user's home.
    folder: String,
    timeout: Duration,
}

/// Bytes read or written at a time.
const CHUNK: usize = 64 * 1024;

impl Sftp {
    /// Connects and signs in. [`Error::HostKeyUnknown`] or [`Error::HostKeyChanged`] when the server's key
    /// is not the pinned one; [`Error::SignInRefused`] when the server refused the sign-in.
    pub fn connect(config: SftpConfig) -> Result<Sftp> {
        if config.host.trim().is_empty() || config.host.contains(['/', '@', ' ']) {
            return Err(Error::BadPath(config.host.clone()));
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().map_err(Error::Io)?;
        let seen = Arc::new(Mutex::new(None));
        let check = KeyCheck { expected: config.host_key.clone(), seen: seen.clone() };
        let ssh_config = Arc::new(client::Config {
            inactivity_timeout: Some(config.timeout.max(Duration::from_secs(60))),
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        });
        let timeout = config.timeout;
        let (session, sftp) = runtime.block_on(async {
            let connecting = client::connect(ssh_config, (config.host.as_str(), config.port), check);
            let mut session = match tokio::time::timeout(timeout, connecting).await {
                Err(_) => return Err(Error::Unreachable(format!("{} did not answer in time", config.host))),
                Ok(Err(e)) => {
                    let found = seen.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    return Err(match (found, &config.host_key) {
                        (Some(found), None) => Error::HostKeyUnknown(found),
                        (Some(found), Some(expected)) if *expected != found => Error::HostKeyChanged { expected: expected.clone(), found },
                        _ => Error::Unreachable(e.to_string()),
                    });
                }
                Ok(Ok(session)) => session,
            };
            let mut signed_in = false;
            if let Some(text) = &config.private_key {
                let key = keys::decode_secret_key(text, config.key_passphrase.as_deref())
                    .map_err(|e| Error::Refused(format!("the private key could not be read ({e})")))?;
                let hash = session.best_supported_rsa_hash().await.ok().flatten().flatten();
                let result = session
                    .authenticate_publickey(&config.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await
                    .map_err(|e| Error::Unreachable(e.to_string()))?;
                signed_in = result.success();
            }
            if !signed_in {
                if let Some(password) = &config.password {
                    let result =
                        session.authenticate_password(&config.user, password).await.map_err(|e| Error::Unreachable(e.to_string()))?;
                    signed_in = result.success();
                }
            }
            if !signed_in {
                return Err(Error::SignInRefused);
            }
            let channel = session.channel_open_session().await.map_err(|e| Error::Unreachable(e.to_string()))?;
            channel.request_subsystem(true, "sftp").await.map_err(|e| Error::Refused(format!("the server has no SFTP ({e})")))?;
            let sftp = SftpSession::new(channel.into_stream()).await.map_err(sftp_failure_for("/"))?;
            sftp.set_timeout(timeout.as_secs().max(1));
            Ok((session, sftp))
        })?;
        Ok(Sftp { runtime, sftp, _session: session, folder: config.folder.trim_end_matches('/').to_string(), timeout })
    }

    /// [`path`] on the server.
    fn full(&self, path: &RemotePath) -> String {
        match (self.folder.is_empty(), path.is_root()) {
            (true, true) => ".".to_string(),
            (true, false) => path.as_str().to_string(),
            (false, true) => self.folder.clone(),
            (false, false) => format!("{}/{}", self.folder, path.as_str()),
        }
    }

    /// Runs [`work`] on the share's runtime, as unreachable when it takes longer than the timeout.
    fn run<T>(&self, work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        let timeout = self.timeout;
        self.runtime.block_on(async move {
            match tokio::time::timeout(timeout, work).await {
                Ok(result) => result,
                Err(_) => Err(Error::Unreachable("the server stopped answering".into())),
            }
        })
    }
}

/// An SFTP error as one of [`Error`], for an operation on [`path`].
fn sftp_failure(e: SftpError, path: &str) -> Error {
    match e {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => Error::NotFound(path.to_string()),
            StatusCode::PermissionDenied => Error::Refused("not allowed".into()),
            _ => Error::Refused(if status.error_message.is_empty() { status.status_code.to_string() } else { status.error_message }),
        },
        SftpError::Timeout => Error::Unreachable("the server stopped answering".into()),
        SftpError::IO(message) => Error::Unreachable(message),
        other => Error::Protocol(other.to_string()),
    }
}

fn sftp_failure_for(path: &str) -> impl Fn(SftpError) -> Error + '_ {
    move |e| sftp_failure(e, path)
}

fn is_missing(e: &SftpError) -> bool {
    matches!(e, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}

impl Remote for Sftp {
    fn list(&self, dir: &RemotePath) -> Result<Vec<Entry>> {
        let full = self.full(dir);
        self.run(async {
            let read = self.sftp.read_dir(full.as_str()).await.map_err(sftp_failure_for(&full))?;
            Ok(read
                .into_iter()
                .filter_map(|item| {
                    let name = item.file_name();
                    if name == "." || name == ".." || RemotePath::root().join(&name).is_err() {
                        return None;
                    }
                    let meta = item.metadata();
                    let is_dir = meta.is_dir();
                    Some(Entry::new(
                        name,
                        is_dir,
                        if is_dir { 0 } else { meta.size.unwrap_or(0) },
                        meta.mtime.map(|t| t as i64 * 1000).unwrap_or(0),
                    ))
                })
                .collect())
        })
    }

    fn stat(&self, path: &RemotePath) -> Result<Option<Entry>> {
        let full = self.full(path);
        self.run(async {
            match self.sftp.metadata(full.as_str()).await {
                Ok(meta) => {
                    let is_dir = meta.is_dir();
                    Ok(Some(Entry::new(
                        path.name(),
                        is_dir,
                        if is_dir { 0 } else { meta.size.unwrap_or(0) },
                        meta.mtime.map(|t| t as i64 * 1000).unwrap_or(0),
                    )))
                }
                Err(e) if is_missing(&e) => Ok(None),
                Err(e) => Err(sftp_failure(e, &full)),
            }
        })
    }

    fn read_into(&self, path: &RemotePath, sink: &mut dyn Write) -> Result<u64> {
        let full = self.full(path);
        self.run(async {
            let mut file = self.sftp.open(full.as_str()).await.map_err(sftp_failure_for(&full))?;
            let mut buf = vec![0u8; CHUNK];
            let mut total = 0u64;
            loop {
                let n = file.read(&mut buf).await.map_err(|e| Error::Unreachable(e.to_string()))?;
                if n == 0 {
                    break;
                }
                sink.write_all(&buf[..n])?;
                total += n as u64;
            }
            Ok(total)
        })
    }

    fn read_range(&self, path: &RemotePath, offset: u64, len: u64) -> Result<Vec<u8>> {
        let full = self.full(path);
        self.run(async {
            let mut file = self.sftp.open(full.as_str()).await.map_err(sftp_failure_for(&full))?;
            file.seek(io::SeekFrom::Start(offset)).await.map_err(|e| Error::Unreachable(e.to_string()))?;
            let mut out = Vec::new();
            let mut buf = vec![0u8; CHUNK];
            while (out.len() as u64) < len {
                let want = ((len - out.len() as u64) as usize).min(CHUNK);
                let n = file.read(&mut buf[..want]).await.map_err(|e| Error::Unreachable(e.to_string()))?;
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
            Ok(out)
        })
    }

    fn write_from(&self, path: &RemotePath, source: &mut dyn Read) -> Result<u64> {
        let tmp = temporary_name(path)?;
        let tmp_full = self.full(&tmp);
        let written = self.run(async {
            let mut file = self.sftp.create(tmp_full.as_str()).await.map_err(|e| {
                if is_missing(&e) {
                    // The folder is not there.
                    Error::NotFound(path.parent().map(|p| p.to_string()).unwrap_or_default())
                } else {
                    sftp_failure(e, &tmp_full)
                }
            })?;
            let mut buf = vec![0u8; CHUNK];
            let mut total = 0u64;
            loop {
                let n = source.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).await.map_err(|e| Error::Unreachable(e.to_string()))?;
                total += n as u64;
            }
            file.shutdown().await.map_err(|e| Error::Unreachable(e.to_string()))?;
            Ok(total)
        });
        let result = written.and_then(|n| self.rename(&tmp, path).map(|_| n));
        if result.is_err() {
            let _ = self.run(async { self.sftp.remove_file(tmp_full.as_str()).await.map_err(sftp_failure_for(&tmp_full)) });
        }
        result
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        let (from_full, to_full) = (self.full(from), self.full(to));
        self.run(async {
            if self.sftp.rename(from_full.as_str(), to_full.as_str()).await.is_ok() {
                return Ok(());
            }
            // Version 3 refuses to rename onto a file that exists: the target goes first, never a folder.
            match self.sftp.metadata(to_full.as_str()).await {
                Ok(meta) if meta.is_dir() => return Err(Error::Refused(format!("{to} is a folder"))),
                Ok(_) => self.sftp.remove_file(to_full.as_str()).await.map_err(sftp_failure_for(&to_full))?,
                Err(e) if is_missing(&e) => {}
                Err(e) => return Err(sftp_failure(e, &to_full)),
            }
            self.sftp.rename(from_full.as_str(), to_full.as_str()).await.map_err(sftp_failure_for(&from_full))
        })
    }

    fn remove(&self, path: &RemotePath) -> Result<()> {
        let full = self.full(path);
        self.run(async {
            match self.sftp.metadata(full.as_str()).await {
                Ok(meta) if meta.is_dir() => Err(Error::Refused(format!("{path} is a folder"))),
                Ok(_) => match self.sftp.remove_file(full.as_str()).await {
                    Err(e) if !is_missing(&e) => Err(sftp_failure(e, &full)),
                    _ => Ok(()),
                },
                Err(e) if is_missing(&e) => Ok(()),
                Err(e) => Err(sftp_failure(e, &full)),
            }
        })
    }

    fn make_dirs(&self, dir: &RemotePath) -> Result<()> {
        let mut at = RemotePath::root();
        for segment in dir.segments().map(str::to_string).collect::<Vec<_>>() {
            at = at.join(&segment)?;
            let full = self.full(&at);
            self.run(async {
                match self.sftp.metadata(full.as_str()).await {
                    Ok(meta) if meta.is_dir() => Ok(()),
                    Ok(_) => Err(Error::Refused(format!("{at} is a file"))),
                    Err(e) if is_missing(&e) => self.sftp.create_dir(full.as_str()).await.map_err(sftp_failure_for(&full)),
                    Err(e) => Err(sftp_failure(e, &full)),
                }
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_password_nor_key_is_ever_printed() {
        let config = SftpConfig::new("nas.local", "me").password("hunter2").private_key("-----BEGIN OPENSSH PRIVATE KEY-----", None);
        let printed = format!("{config:?}");
        assert!(!printed.contains("hunter2"));
        assert!(!printed.contains("BEGIN OPENSSH"));
    }

    #[test]
    fn a_host_with_a_sign_in_or_path_in_it_is_refused() {
        for bad in ["", "me@nas.local", "nas.local/share", "nas local"] {
            assert!(matches!(Sftp::connect(SftpConfig::new(bad, "me")), Err(Error::BadPath(_))), "{bad:?}");
        }
    }
}
