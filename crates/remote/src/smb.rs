//! SMB 2 and 3: Windows shares, Samba, and most NAS (Droidtop/tracker#573 design D2, item 3).
//!
//! libsmb2 (sahlberg/libsmb2, tag libsmb2-6.3, LGPL-2.1-or-later; vendor/libsmb2, built by build.rs) is a
//! userspace client: no root and no kernel module. VLC and Kodi use it to reach SMB shares on Android. This file
//! calls its synchronous API through a small binding, laid out from include/smb2/libsmb2.h at that tag.
//!
//! - **Sign-in:** NTLMSSP, the person's own user name, password and (optionally) domain. Signing is always
//!   asked for. With [`SmbConfig::require_encryption`] the connection must also be SMB 3 encryption, and
//!   connecting fails when the server can't do that.
//! - **Writes:** go to a temporary name and are renamed. SMB's rename doesn't replace an existing file (libsmb2
//!   sends `replace_if_exist = 0`), so the target is removed first: for that moment a reader can find the target
//!   missing, never half written.
//! - **Thread safety:** a libsmb2 context isn't thread-safe, so every call holds the share's lock.

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::raw::{c_char, c_int};
use std::sync::Mutex;
use std::time::Duration;

use crate::{temporary_name, Entry, Error, Remote, RemotePath, Result};

#[repr(C)]
struct Smb2Context {
    _private: [u8; 0],
}
#[repr(C)]
struct Smb2Dir {
    _private: [u8; 0],
}
#[repr(C)]
struct Smb2Fh {
    _private: [u8; 0],
}

/// `struct smb2_stat_64` (libsmb2.h). Every field is laid out as C has it; not all are read.
#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct Smb2Stat {
    smb2_type: u32,
    smb2_nlink: u32,
    smb2_ino: u64,
    smb2_size: u64,
    smb2_atime: u64,
    smb2_atime_nsec: u64,
    smb2_mtime: u64,
    smb2_mtime_nsec: u64,
    smb2_ctime: u64,
    smb2_ctime_nsec: u64,
    smb2_btime: u64,
    smb2_btime_nsec: u64,
}

/// `struct smb2dirent` (libsmb2.h).
#[repr(C)]
struct Smb2Dirent {
    name: *const c_char,
    st: Smb2Stat,
}

const SMB2_TYPE_DIRECTORY: u32 = 1;
const SMB2_NEGOTIATE_SIGNING_ENABLED: u16 = 0x0001;
const STATUS_ACCESS_DENIED: u32 = 0xC000_0022;
const STATUS_WRONG_PASSWORD: u32 = 0xC000_006A;
const STATUS_LOGON_FAILURE: u32 = 0xC000_006D;
const STATUS_ACCOUNT_DISABLED: u32 = 0xC000_0072;
const STATUS_BAD_NETWORK_NAME: u32 = 0xC000_00CC;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
const STATUS_NO_SUCH_FILE: u32 = 0xC000_000F;

extern "C" {
    fn smb2_init_context() -> *mut Smb2Context;
    fn smb2_destroy_context(smb2: *mut Smb2Context);
    fn smb2_set_timeout(smb2: *mut Smb2Context, seconds: c_int);
    fn smb2_set_security_mode(smb2: *mut Smb2Context, security_mode: u16);
    fn smb2_set_seal(smb2: *mut Smb2Context, val: c_int);
    fn smb2_set_user(smb2: *mut Smb2Context, user: *const c_char);
    fn smb2_set_password(smb2: *mut Smb2Context, password: *const c_char);
    fn smb2_set_domain(smb2: *mut Smb2Context, domain: *const c_char);
    fn smb2_connect_share(smb2: *mut Smb2Context, server: *const c_char, share: *const c_char, user: *const c_char) -> c_int;
    fn smb2_disconnect_share(smb2: *mut Smb2Context) -> c_int;
    fn smb2_get_error(smb2: *mut Smb2Context) -> *const c_char;
    fn smb2_get_nterror(smb2: *mut Smb2Context) -> c_int;
    fn smb2_opendir(smb2: *mut Smb2Context, path: *const c_char) -> *mut Smb2Dir;
    fn smb2_readdir(smb2: *mut Smb2Context, dir: *mut Smb2Dir) -> *mut Smb2Dirent;
    fn smb2_closedir(smb2: *mut Smb2Context, dir: *mut Smb2Dir);
    fn smb2_open(smb2: *mut Smb2Context, path: *const c_char, flags: c_int) -> *mut Smb2Fh;
    fn smb2_close(smb2: *mut Smb2Context, fh: *mut Smb2Fh) -> c_int;
    fn smb2_pread(smb2: *mut Smb2Context, fh: *mut Smb2Fh, buf: *mut u8, count: u32, offset: u64) -> c_int;
    fn smb2_pwrite(smb2: *mut Smb2Context, fh: *mut Smb2Fh, buf: *const u8, count: u32, offset: u64) -> c_int;
    fn smb2_get_max_read_size(smb2: *mut Smb2Context) -> u32;
    fn smb2_get_max_write_size(smb2: *mut Smb2Context) -> u32;
    fn smb2_stat(smb2: *mut Smb2Context, path: *const c_char, st: *mut Smb2Stat) -> c_int;
    fn smb2_mkdir(smb2: *mut Smb2Context, path: *const c_char) -> c_int;
    fn smb2_unlink(smb2: *mut Smb2Context, path: *const c_char) -> c_int;
    fn smb2_rename(smb2: *mut Smb2Context, oldpath: *const c_char, newpath: *const c_char) -> c_int;
}

/// What an SMB share is: the server, the share, the person's own sign-in, and the folder the share starts at.
#[derive(Clone)]
#[non_exhaustive]
pub struct SmbConfig {
    /// The server's name or address, with `:port` when it isn't 445.
    pub server: String,
    /// The share's name on the server (`\\server\<share>`).
    pub share: String,
    pub user: String,
    pub password: String,
    pub domain: Option<String>,
    /// The folder inside the share the share starts at; empty for its top.
    pub folder: String,
    /// Insist on SMB 3 encryption: connecting fails when the server can't encrypt.
    pub require_encryption: bool,
    pub timeout: Duration,
}

impl SmbConfig {
    pub fn new(server: impl Into<String>, share: impl Into<String>, user: impl Into<String>, password: impl Into<String>) -> SmbConfig {
        SmbConfig {
            server: server.into(),
            share: share.into(),
            user: user.into(),
            password: password.into(),
            domain: None,
            folder: String::new(),
            require_encryption: false,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn domain(mut self, domain: impl Into<String>) -> SmbConfig {
        self.domain = Some(domain.into());
        self
    }

    pub fn folder(mut self, folder: impl Into<String>) -> SmbConfig {
        self.folder = folder.into();
        self
    }

    pub fn require_encryption(mut self, on: bool) -> SmbConfig {
        self.require_encryption = on;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> SmbConfig {
        self.timeout = timeout;
        self
    }
}

impl std::fmt::Debug for SmbConfig {
    // The password is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbConfig")
            .field("server", &self.server)
            .field("share", &self.share)
            .field("user", &self.user)
            .field("password", &"…")
            .field("domain", &self.domain)
            .field("folder", &self.folder)
            .field("require_encryption", &self.require_encryption)
            .finish()
    }
}

/// A connected libsmb2 context. Only ever used under [`Smb`]'s lock.
struct Context(*mut Smb2Context);

// The context is only reached through Smb's Mutex, one call at a time.
unsafe impl Send for Context {}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the pointer came from smb2_init_context and is dropped once.
        unsafe {
            smb2_disconnect_share(self.0);
            smb2_destroy_context(self.0);
        }
    }
}

/// An SMB share, connected.
pub struct Smb {
    context: Mutex<Context>,
    /// The share's folder, without slashes at either end.
    folder: String,
}

fn c_string(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::BadPath(s.to_string()))
}

/// libsmb2's error for the last call on [`ctx`], as one of [`Error`], for [`path`].
fn failure(ctx: *mut Smb2Context, ret: c_int, path: &str) -> Error {
    // SAFETY: ctx is a live context; smb2_get_error returns a string owned by it, valid until the next call.
    let (message, nt) = unsafe {
        let p = smb2_get_error(ctx);
        let message = if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
        (message, smb2_get_nterror(ctx) as u32)
    };
    let errno = -ret;
    match nt {
        STATUS_LOGON_FAILURE | STATUS_WRONG_PASSWORD | STATUS_ACCOUNT_DISABLED => Error::SignInRefused,
        STATUS_ACCESS_DENIED => Error::Refused("not allowed".into()),
        STATUS_BAD_NETWORK_NAME => Error::Refused("the server has no share by that name".into()),
        STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND | STATUS_NO_SUCH_FILE => Error::NotFound(path.to_string()),
        _ if errno == libc_enoent() => Error::NotFound(path.to_string()),
        _ if [libc_etimedout(), libc_econnrefused(), libc_ehostunreach(), libc_enetunreach()].contains(&errno)
            || unreachable_words(&message) =>
        {
            Error::Unreachable(if message.is_empty() { format!("errno {errno}") } else { message.trim().to_string() })
        }
        _ => Error::Refused(if message.is_empty() { format!("it answered {errno}") } else { message }),
    }
}

/// Whether libsmb2's words say the server wasn't reached. It reports a failed connect in its message ("Socket connect
/// failed with 111"), not in the value it returns.
fn unreachable_words(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    ["socket connect failed", "connect failed", "timed out", "timeout", "getaddrinfo", "no route to host", "network is unreachable"]
        .iter()
        .any(|w| m.contains(w))
}

// The errno values libsmb2 returns negated: Linux and Android share them; macOS differs for the network ones.
fn libc_enoent() -> c_int {
    2
}
fn libc_etimedout() -> c_int {
    if cfg!(target_vendor = "apple") {
        60
    } else {
        110
    }
}
fn libc_econnrefused() -> c_int {
    if cfg!(target_vendor = "apple") {
        61
    } else {
        111
    }
}
fn libc_ehostunreach() -> c_int {
    if cfg!(target_vendor = "apple") {
        65
    } else {
        113
    }
}
fn libc_enetunreach() -> c_int {
    if cfg!(target_vendor = "apple") {
        51
    } else {
        101
    }
}

// The open flags libsmb2 takes are the POSIX ones: Linux and Android share these; macOS differs.
const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;
const O_CREAT: c_int = if cfg!(target_vendor = "apple") { 0x200 } else { 0o100 };
const O_TRUNC: c_int = if cfg!(target_vendor = "apple") { 0x400 } else { 0o1000 };

impl Smb {
    /// Connects and signs in. [`Error::SignInRefused`] when the server refused the sign-in, [`Error::Unreachable`]
    /// when it didn't answer.
    pub fn connect(config: SmbConfig) -> Result<Smb> {
        let server = config.server.trim();
        if server.is_empty() || server.contains(['/', '\\', '@', ' ']) {
            return Err(Error::BadPath(config.server.clone()));
        }
        let share = config.share.trim().trim_matches(['/', '\\']);
        if share.is_empty() || share.contains(['/', '\\']) {
            return Err(Error::BadPath(config.share.clone()));
        }
        let (c_server, c_share, c_user, c_password) =
            (c_string(server)?, c_string(share)?, c_string(&config.user)?, c_string(&config.password)?);
        let c_domain = config.domain.as_deref().filter(|d| !d.is_empty()).map(c_string).transpose()?;
        // SAFETY: smb2_init_context returns a new context or null; every pointer passed below is a live CString.
        let context = unsafe {
            let ctx = smb2_init_context();
            if ctx.is_null() {
                return Err(Error::Protocol("libsmb2 could not start".into()));
            }
            let context = Context(ctx);
            smb2_set_timeout(ctx, config.timeout.as_secs().clamp(1, i32::MAX as u64) as c_int);
            smb2_set_security_mode(ctx, SMB2_NEGOTIATE_SIGNING_ENABLED);
            if config.require_encryption {
                smb2_set_seal(ctx, 1);
            }
            smb2_set_user(ctx, c_user.as_ptr());
            smb2_set_password(ctx, c_password.as_ptr());
            if let Some(domain) = &c_domain {
                smb2_set_domain(ctx, domain.as_ptr());
            }
            let ret = smb2_connect_share(ctx, c_server.as_ptr(), c_share.as_ptr(), c_user.as_ptr());
            if ret < 0 {
                return Err(failure(ctx, ret, share));
            }
            context
        };
        Ok(Smb { context: Mutex::new(context), folder: config.folder.trim_matches(['/', '\\']).to_string() })
    }

    /// [`path`] inside the share, as libsmb2 takes it.
    fn full(&self, path: &RemotePath) -> String {
        match (self.folder.is_empty(), path.is_root()) {
            (true, _) => path.as_str().to_string(),
            (false, true) => self.folder.clone(),
            (false, false) => format!("{}/{}", self.folder, path.as_str()),
        }
    }

    fn with<T>(&self, work: impl FnOnce(*mut Smb2Context) -> Result<T>) -> Result<T> {
        let guard = self.context.lock().unwrap_or_else(|e| e.into_inner());
        work(guard.0)
    }

    fn stat_raw(ctx: *mut Smb2Context, full: &str) -> Result<Option<Smb2Stat>> {
        let c = c_string(full)?;
        let mut st = Smb2Stat::default();
        // SAFETY: ctx is live under the lock; c and st outlive the call.
        let ret = unsafe { smb2_stat(ctx, c.as_ptr(), &mut st) };
        if ret < 0 {
            return match failure(ctx, ret, full) {
                Error::NotFound(_) => Ok(None),
                other => Err(other),
            };
        }
        Ok(Some(st))
    }

    /// Opens [`full`] with [`flags`], runs [`work`] on the handle, and closes it.
    fn with_file<T>(ctx: *mut Smb2Context, full: &str, flags: c_int, work: impl FnOnce(*mut Smb2Fh) -> Result<T>) -> Result<T> {
        let c = c_string(full)?;
        // SAFETY: ctx is live under the lock; c outlives the call.
        let fh = unsafe { smb2_open(ctx, c.as_ptr(), flags) };
        if fh.is_null() {
            return Err(failure(ctx, -libc_enoent(), full));
        }
        let result = work(fh);
        // SAFETY: fh came from smb2_open on ctx and is closed once.
        unsafe { smb2_close(ctx, fh) };
        result
    }

    fn unlink(ctx: *mut Smb2Context, full: &str) -> Result<()> {
        let c = c_string(full)?;
        // SAFETY: ctx is live under the lock; c outlives the call.
        let ret = unsafe { smb2_unlink(ctx, c.as_ptr()) };
        if ret < 0 {
            return match failure(ctx, ret, full) {
                Error::NotFound(_) => Ok(()),
                other => Err(other),
            };
        }
        Ok(())
    }

    fn rename_raw(ctx: *mut Smb2Context, from: &str, to: &str) -> Result<()> {
        let (c_from, c_to) = (c_string(from)?, c_string(to)?);
        // SAFETY: ctx is live under the lock; both strings outlive the call.
        let ret = unsafe { smb2_rename(ctx, c_from.as_ptr(), c_to.as_ptr()) };
        if ret < 0 {
            return Err(failure(ctx, ret, from));
        }
        Ok(())
    }
}

fn entry(name: String, st: &Smb2Stat) -> Entry {
    let is_dir = st.smb2_type == SMB2_TYPE_DIRECTORY;
    Entry::new(name, is_dir, if is_dir { 0 } else { st.smb2_size }, (st.smb2_mtime as i64) * 1000 + (st.smb2_mtime_nsec / 1_000_000) as i64)
}

impl Remote for Smb {
    fn list(&self, dir: &RemotePath) -> Result<Vec<Entry>> {
        let full = self.full(dir);
        self.with(|ctx| {
            let c = c_string(&full)?;
            // SAFETY: ctx is live under the lock; c outlives the call.
            let handle = unsafe { smb2_opendir(ctx, c.as_ptr()) };
            if handle.is_null() {
                return Err(failure(ctx, -libc_enoent(), &full));
            }
            let mut out = Vec::new();
            loop {
                // SAFETY: handle came from smb2_opendir on ctx; the entry is valid until the next readdir.
                let ent = unsafe { smb2_readdir(ctx, handle) };
                if ent.is_null() {
                    break;
                }
                // SAFETY: a non-null entry from smb2_readdir has a NUL-terminated name.
                let (name, st) = unsafe { (CStr::from_ptr((*ent).name).to_string_lossy().into_owned(), &(*ent).st) };
                if name == "." || name == ".." || RemotePath::root().join(&name).is_err() {
                    continue;
                }
                out.push(entry(name, st));
            }
            // SAFETY: handle came from smb2_opendir on ctx and is closed once.
            unsafe { smb2_closedir(ctx, handle) };
            Ok(out)
        })
    }

    fn stat(&self, path: &RemotePath) -> Result<Option<Entry>> {
        let full = self.full(path);
        self.with(|ctx| Ok(Smb::stat_raw(ctx, &full)?.map(|st| entry(path.name().to_string(), &st))))
    }

    fn read_into(&self, path: &RemotePath, sink: &mut dyn Write) -> Result<u64> {
        let full = self.full(path);
        self.with(|ctx| {
            Smb::with_file(ctx, &full, O_RDONLY, |fh| {
                // SAFETY: ctx is live under the lock.
                let chunk = unsafe { smb2_get_max_read_size(ctx) }.clamp(4096, 1 << 20);
                let mut buf = vec![0u8; chunk as usize];
                let mut offset = 0u64;
                loop {
                    // SAFETY: fh is open on ctx; buf holds chunk bytes.
                    let n = unsafe { smb2_pread(ctx, fh, buf.as_mut_ptr(), chunk, offset) };
                    if n < 0 {
                        return Err(failure(ctx, n, &full));
                    }
                    if n == 0 {
                        break;
                    }
                    sink.write_all(&buf[..n as usize])?;
                    offset += n as u64;
                }
                Ok(offset)
            })
        })
    }

    fn read_range(&self, path: &RemotePath, offset: u64, len: u64) -> Result<Vec<u8>> {
        let full = self.full(path);
        self.with(|ctx| {
            Smb::with_file(ctx, &full, O_RDONLY, |fh| {
                // SAFETY: ctx is live under the lock.
                let chunk = unsafe { smb2_get_max_read_size(ctx) }.clamp(4096, 1 << 20) as u64;
                let mut out = Vec::new();
                while (out.len() as u64) < len {
                    let want = (len - out.len() as u64).min(chunk) as u32;
                    let mut buf = vec![0u8; want as usize];
                    // SAFETY: fh is open on ctx; buf holds want bytes.
                    let n = unsafe { smb2_pread(ctx, fh, buf.as_mut_ptr(), want, offset + out.len() as u64) };
                    if n < 0 {
                        return Err(failure(ctx, n, &full));
                    }
                    if n == 0 {
                        break;
                    }
                    out.extend_from_slice(&buf[..n as usize]);
                }
                Ok(out)
            })
        })
    }

    fn write_from(&self, path: &RemotePath, source: &mut dyn Read) -> Result<u64> {
        let tmp = self.full(&temporary_name(path)?);
        let target = self.full(path);
        self.with(|ctx| {
            if let Some(parent) = path.parent().filter(|p| !p.is_root()) {
                if Smb::stat_raw(ctx, &self.full(&parent))?.is_none() {
                    return Err(Error::NotFound(parent.to_string()));
                }
            }
            let written = Smb::with_file(ctx, &tmp, O_WRONLY | O_CREAT | O_TRUNC, |fh| {
                // SAFETY: ctx is live under the lock.
                let chunk = unsafe { smb2_get_max_write_size(ctx) }.clamp(4096, 1 << 20) as usize;
                let mut buf = vec![0u8; chunk];
                let mut offset = 0u64;
                loop {
                    let n = source.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    let mut done = 0;
                    while done < n {
                        // SAFETY: fh is open on ctx; the slice holds n - done bytes.
                        let w = unsafe { smb2_pwrite(ctx, fh, buf[done..n].as_ptr(), (n - done) as u32, offset) };
                        if w <= 0 {
                            return Err(failure(ctx, w.min(-1), &tmp));
                        }
                        done += w as usize;
                        offset += w as u64;
                    }
                }
                Ok(offset)
            });
            let result = written.and_then(|n| {
                // SMB's rename doesn't replace: the target goes first, never a folder.
                match Smb::stat_raw(ctx, &target)? {
                    Some(st) if st.smb2_type == SMB2_TYPE_DIRECTORY => return Err(Error::Refused(format!("{path} is a folder"))),
                    Some(_) => Smb::unlink(ctx, &target)?,
                    None => {}
                }
                Smb::rename_raw(ctx, &tmp, &target).map(|_| n)
            });
            if result.is_err() {
                let _ = Smb::unlink(ctx, &tmp);
            }
            result
        })
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        let (from_full, to_full) = (self.full(from), self.full(to));
        self.with(|ctx| {
            match Smb::stat_raw(ctx, &to_full)? {
                Some(st) if st.smb2_type == SMB2_TYPE_DIRECTORY => return Err(Error::Refused(format!("{to} is a folder"))),
                Some(_) => Smb::unlink(ctx, &to_full)?,
                None => {}
            }
            Smb::rename_raw(ctx, &from_full, &to_full)
        })
    }

    fn remove(&self, path: &RemotePath) -> Result<()> {
        let full = self.full(path);
        self.with(|ctx| match Smb::stat_raw(ctx, &full)? {
            None => Ok(()),
            Some(st) if st.smb2_type == SMB2_TYPE_DIRECTORY => Err(Error::Refused(format!("{path} is a folder"))),
            Some(_) => Smb::unlink(ctx, &full),
        })
    }

    fn make_dirs(&self, dir: &RemotePath) -> Result<()> {
        let mut at = RemotePath::root();
        for segment in dir.segments().map(str::to_string).collect::<Vec<_>>() {
            at = at.join(&segment)?;
            let full = self.full(&at);
            self.with(|ctx| match Smb::stat_raw(ctx, &full)? {
                Some(st) if st.smb2_type == SMB2_TYPE_DIRECTORY => Ok(()),
                Some(_) => Err(Error::Refused(format!("{at} is a file"))),
                None => {
                    let c = c_string(&full)?;
                    // SAFETY: ctx is live under the lock; c outlives the call.
                    let ret = unsafe { smb2_mkdir(ctx, c.as_ptr()) };
                    if ret < 0 {
                        Err(failure(ctx, ret, &full))
                    } else {
                        Ok(())
                    }
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
    fn the_password_is_never_printed() {
        assert!(!format!("{:?}", SmbConfig::new("nas.local", "games", "me", "hunter2")).contains("hunter2"));
    }

    #[test]
    fn a_server_or_share_with_a_path_or_sign_in_in_it_is_refused() {
        for (server, share) in [("", "s"), ("me@nas.local", "s"), ("nas.local/s", "s"), ("nas.local", ""), ("nas.local", "a/b")] {
            assert!(matches!(Smb::connect(SmbConfig::new(server, share, "u", "p")), Err(Error::BadPath(_))), "{server:?} {share:?}");
        }
    }

    #[test]
    fn libsmb2s_connect_failures_read_as_unreachable() {
        assert!(unreachable_words(
            "smb2_service failed with : Socket connect failed with 111
"
        ));
        assert!(unreachable_words("Timeout expired and no connection exists"));
        assert!(!unreachable_words("Session setup failed with (0xc000006d) STATUS_LOGON_FAILURE"));
    }

    #[test]
    fn the_binding_matches_libsmb2s_struct_layout() {
        // struct smb2_stat_64: two u32 then ten u64, no padding.
        assert_eq!(std::mem::size_of::<Smb2Stat>(), 2 * 4 + 10 * 8);
    }
}
