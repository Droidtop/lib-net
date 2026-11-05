//! S3-compatible storage: AWS S3, Backblaze B2, Wasabi, Cloudflare R2, MinIO, Garage and the like
//! (Droidtop/tracker#573 design D2, item 4).
//!
//! rusty-s3 signs each request (SigV4, as a presigned address valid for a few minutes); the request
//! itself goes through ureq with rustls, as WebDAV's do. The person's own access key and secret are
//! used; nothing authenticating ships in a program.
//!
//! S3 has objects, not folders, so a folder here is a key prefix ending in `/`: [`Remote::make_dirs`]
//! leaves an empty marker object for each level (the way S3 consoles make folders), and a listing
//! asks for one level with the `/` delimiter. A write is one PUT, which S3 makes visible in one step.
//! A rename is a copy (read, then write) and a delete, since rusty-s3 signs no server-side copy: it
//! moves the file's bytes through this device. Nothing is ever deleted under a prefix wholesale.

use std::io::{self, Read, Write};
use std::time::{Duration, UNIX_EPOCH};

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use url::Url;

use crate::{Entry, Error, Remote, RemotePath, Result};

/// What an S3 share is: the service's address, the bucket, and the person's own key.
#[derive(Clone)]
#[non_exhaustive]
pub struct S3Config {
    /// The service's address, for example `https://s3.eu-central-003.backblazeb2.com`. No key in it.
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// The folder inside the bucket the share starts at; empty for the bucket's top.
    pub folder: String,
    /// Address the bucket as `<endpoint>/<bucket>` (most S3-compatible services, and IP addresses)
    /// rather than `<bucket>.<endpoint host>`.
    pub path_style: bool,
    /// Plain http is refused unless this is set: the person's choice, for a service on their own network.
    pub allow_plain_http: bool,
    pub timeout: Duration,
}

impl S3Config {
    pub fn new(
        endpoint: impl Into<String>,
        bucket: impl Into<String>,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> S3Config {
        S3Config {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            region: "us-east-1".into(),
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            folder: String::new(),
            path_style: true,
            allow_plain_http: false,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn region(mut self, region: impl Into<String>) -> S3Config {
        self.region = region.into();
        self
    }

    pub fn folder(mut self, folder: impl Into<String>) -> S3Config {
        self.folder = folder.into();
        self
    }

    pub fn path_style(mut self, path_style: bool) -> S3Config {
        self.path_style = path_style;
        self
    }

    pub fn allow_plain_http(mut self, allow: bool) -> S3Config {
        self.allow_plain_http = allow;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> S3Config {
        self.timeout = timeout;
        self
    }
}

impl std::fmt::Debug for S3Config {
    // The secret is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key", &self.access_key)
            .field("secret_key", &"…")
            .field("folder", &self.folder)
            .finish()
    }
}

/// One object in a listing.
struct Listed {
    key: String,
    size: u64,
    last_modified: Option<String>,
}

/// An S3 share.
pub struct S3 {
    agent: ureq::Agent,
    bucket: Bucket,
    credentials: Credentials,
    /// The folder inside the bucket, without slashes at either end.
    folder: String,
}

/// How long a signed address stays valid: long enough for one request to start.
const SIGNED_FOR: Duration = Duration::from_secs(300);

impl S3 {
    pub fn new(config: S3Config) -> Result<S3> {
        let endpoint = config.endpoint.trim();
        let url = Url::parse(endpoint).map_err(|_| Error::BadPath(endpoint.to_string()))?;
        match url.scheme() {
            "https" => {}
            "http" if config.allow_plain_http => {}
            "http" => return Err(Error::NotEncrypted),
            _ => return Err(Error::BadPath(endpoint.to_string())),
        }
        // A key in the address would end up in logs and history.
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err(Error::BadPath(endpoint.to_string()));
        }
        let style = if config.path_style { UrlStyle::Path } else { UrlStyle::VirtualHost };
        let bucket = Bucket::new(url, style, config.bucket.clone(), config.region.clone())
            .map_err(|e| Error::BadPath(format!("{} ({e})", config.bucket)))?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(config.timeout.min(Duration::from_secs(15)))
            .timeout_read(config.timeout)
            .timeout_write(config.timeout)
            .redirects(0)
            .user_agent(concat!("droidtop-net-remote/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(S3 {
            agent,
            bucket,
            credentials: Credentials::new(config.access_key, config.secret_key),
            folder: config.folder.trim_matches('/').to_string(),
        })
    }

    /// [`path`]'s key in the bucket.
    fn key(&self, path: &RemotePath) -> String {
        match (self.folder.is_empty(), path.is_root()) {
            (true, _) => path.as_str().to_string(),
            (false, true) => self.folder.clone(),
            (false, false) => format!("{}/{}", self.folder, path.as_str()),
        }
    }

    /// The prefix of what is inside the folder [`dir`]: its key and `/`, or nothing for the bucket's top.
    fn prefix(&self, dir: &RemotePath) -> String {
        let key = self.key(dir);
        if key.is_empty() {
            key
        } else {
            format!("{key}/")
        }
    }

    /// One level of what is under [`prefix`]: the objects and the folders' prefixes.
    fn list_level(&self, prefix: &str, limit: Option<usize>) -> Result<(Vec<Listed>, Vec<String>)> {
        let mut contents = Vec::new();
        let mut folders = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action = self.bucket.list_objects_v2(Some(&self.credentials));
            action.with_delimiter("/");
            if !prefix.is_empty() {
                action.with_prefix(prefix);
            }
            if let Some(max) = limit {
                action.with_max_keys(max);
            }
            if let Some(t) = &token {
                action.with_continuation_token(t.clone());
            }
            let body = self.call(self.agent.get(action.sign(SIGNED_FOR).as_str()), prefix)?;
            let page = rusty_s3::actions::ListObjectsV2::parse_response(&body).map_err(|e| Error::Protocol(e.to_string()))?;
            contents.extend(page.contents.into_iter().map(|c| Listed {
                key: c.key,
                size: c.size.unwrap_or(0),
                last_modified: c.last_modified,
            }));
            folders.extend(page.common_prefixes.into_iter().map(|p| p.prefix));
            match page.next_continuation_token {
                Some(next) if limit.is_none() => token = Some(next),
                _ => break,
            }
        }
        Ok((contents, folders))
    }

    /// Sends [`request`] and reads a small answer as text.
    fn call(&self, request: ureq::Request, what: &str) -> Result<String> {
        let response = request.call().map_err(|e| failure(e, what))?;
        let mut body = String::new();
        response.into_reader().take(64 << 20).read_to_string(&mut body)?;
        Ok(body)
    }

    fn object(&self, key: &str) -> Result<Option<Entry>> {
        let url = self.bucket.head_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        match self.agent.head(url.as_str()).call() {
            Ok(r) => {
                let size = r.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
                let mtime_ms = r
                    .header("Last-Modified")
                    .and_then(|v| httpdate::parse_http_date(v).ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let name = key.rsplit('/').next().unwrap_or(key);
                Ok(Some(Entry::new(name, false, size, mtime_ms)))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(failure(e, key)),
        }
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let url = self.bucket.put_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        self.agent.put(url.as_str()).send_bytes(bytes).map(|_| ()).map_err(|e| failure(e, key))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let url = self.bucket.delete_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        match self.agent.delete(url.as_str()).call() {
            Ok(_) | Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(e) => Err(failure(e, key)),
        }
    }

    fn is_folder(&self, path: &RemotePath) -> Result<bool> {
        if path.is_root() {
            return Ok(true);
        }
        let prefix = self.prefix(path);
        let (contents, folders) = self.list_level(&prefix, Some(1))?;
        Ok(!contents.is_empty() || !folders.is_empty())
    }
}

/// An error from ureq as one of [`Error`], for an operation on [`what`].
fn failure(e: ureq::Error, what: &str) -> Error {
    match e {
        ureq::Error::Status(code, response) => {
            let mut body = String::new();
            let _ = response.into_reader().take(16 << 10).read_to_string(&mut body);
            let code_word = body.split("<Code>").nth(1).and_then(|r| r.split("</Code>").next()).unwrap_or("").to_string();
            match (code, code_word.as_str()) {
                (403, "InvalidAccessKeyId" | "SignatureDoesNotMatch") => Error::SignInRefused,
                (401, _) => Error::SignInRefused,
                (403, _) => Error::Refused("not allowed".into()),
                (404, "NoSuchBucket") => Error::Refused("there is no such bucket".into()),
                (404, _) => Error::NotFound(what.to_string()),
                (301 | 307, _) => Error::Refused("the bucket is in another region or at another address".into()),
                (_, word) if !word.is_empty() => Error::Refused(word.to_string()),
                (other, _) => Error::Refused(format!("it answered {other}")),
            }
        }
        ureq::Error::Transport(t) => Error::Unreachable(t.to_string()),
    }
}

/// An S3 time (`2026-11-05T06:30:15.000Z`) in ms since the Unix epoch, or 0. Pure.
fn parse_time(s: &str) -> i64 {
    let digits = |r: std::ops::Range<usize>| s.get(r).and_then(|t| t.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(sec)) =
        (digits(0..4), digits(5..7), digits(8..10), digits(11..13), digits(14..16), digits(17..19))
    else {
        return 0;
    };
    // Howard Hinnant's days_from_civil.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days * 86_400 + h * 3_600 + mi * 60 + sec) * 1000
}

impl Remote for S3 {
    fn list(&self, dir: &RemotePath) -> Result<Vec<Entry>> {
        let prefix = self.prefix(dir);
        let (contents, folders) = self.list_level(&prefix, None)?;
        if contents.is_empty() && folders.is_empty() && !dir.is_root() {
            return Err(Error::NotFound(dir.to_string()));
        }
        let mut out = Vec::new();
        for object in contents {
            let name = object.key.strip_prefix(&prefix).unwrap_or(&object.key);
            // The folder's own marker, or anything a plain path can't name.
            if name.is_empty() || RemotePath::root().join(name).is_err() {
                continue;
            }
            out.push(Entry::new(name, false, object.size, object.last_modified.as_deref().map(parse_time).unwrap_or(0)));
        }
        for folder in folders {
            let name = folder.strip_prefix(&prefix).unwrap_or(&folder).trim_end_matches('/');
            if RemotePath::root().join(name).is_ok() {
                out.push(Entry::new(name, true, 0, 0));
            }
        }
        Ok(out)
    }

    fn stat(&self, path: &RemotePath) -> Result<Option<Entry>> {
        if !path.is_root() {
            if let Some(mut entry) = self.object(&self.key(path))? {
                entry.name = path.name().to_string();
                return Ok(Some(entry));
            }
        }
        Ok(self.is_folder(path)?.then(|| Entry::new(path.name(), true, 0, 0)))
    }

    fn read_into(&self, path: &RemotePath, sink: &mut dyn Write) -> Result<u64> {
        let key = self.key(path);
        let url = self.bucket.get_object(Some(&self.credentials), &key).sign(SIGNED_FOR);
        let response = self.agent.get(url.as_str()).call().map_err(|e| failure(e, &path.to_string()))?;
        Ok(io::copy(&mut response.into_reader(), sink)?)
    }

    fn read_range(&self, path: &RemotePath, offset: u64, len: u64) -> Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let key = self.key(path);
        let url = self.bucket.get_object(Some(&self.credentials), &key).sign(SIGNED_FOR);
        let range = format!("bytes={offset}-{}", offset.saturating_add(len - 1));
        let response = match self.agent.get(url.as_str()).set("Range", &range).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(416, _)) => return Ok(Vec::new()),
            Err(e) => return Err(failure(e, &path.to_string())),
        };
        let whole = response.status() == 200;
        let mut reader = response.into_reader();
        if whole {
            io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
        }
        let mut out = Vec::new();
        reader.take(len).read_to_end(&mut out)?;
        Ok(out)
    }

    fn write_from(&self, path: &RemotePath, source: &mut dyn Read) -> Result<u64> {
        // S3 needs a request's length before its body, and makes a whole PUT visible at once.
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes)?;
        if let Some(parent) = path.parent() {
            if !self.is_folder(&parent)? {
                return Err(Error::NotFound(parent.to_string()));
            }
        }
        self.put(&self.key(path), &bytes)?;
        Ok(bytes.len() as u64)
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()> {
        if self.is_folder(to)? && !to.is_root() && self.object(&self.key(to))?.is_none() {
            return Err(Error::Refused(format!("{to} is a folder")));
        }
        let bytes = self.read(from)?;
        self.put(&self.key(to), &bytes)?;
        self.delete(&self.key(from))
    }

    fn remove(&self, path: &RemotePath) -> Result<()> {
        let key = self.key(path);
        if self.object(&key)?.is_none() {
            if self.is_folder(path)? {
                return Err(Error::Refused(format!("{path} is a folder")));
            }
            return Ok(());
        }
        self.delete(&key)
    }

    fn make_dirs(&self, dir: &RemotePath) -> Result<()> {
        let mut at = RemotePath::root();
        for segment in dir.segments().map(str::to_string).collect::<Vec<_>>() {
            at = at.join(&segment)?;
            let marker = self.prefix(&at);
            if self.object(&marker)?.is_none() {
                self.put(&marker, b"")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_times_read() {
        assert_eq!(parse_time("2026-11-05T06:30:15.000Z"), 1_793_860_215_000);
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse_time("not a time"), 0);
    }

    #[test]
    fn keys_sit_under_the_shares_folder() {
        let s3 = S3::new(S3Config::new("https://s3.example.com", "games", "k", "s").folder("/droidtop/")).unwrap();
        assert_eq!(s3.key(&RemotePath::new("a/b.sav").unwrap()), "droidtop/a/b.sav");
        assert_eq!(s3.prefix(&RemotePath::root()), "droidtop/");
        let top = S3::new(S3Config::new("https://s3.example.com", "games", "k", "s")).unwrap();
        assert_eq!(top.prefix(&RemotePath::root()), "");
        assert_eq!(top.prefix(&RemotePath::new("a").unwrap()), "a/");
    }

    #[test]
    fn addresses_that_would_leak_a_key_are_refused() {
        assert!(matches!(S3::new(S3Config::new("http://nas.local:9000", "b", "k", "s")), Err(Error::NotEncrypted)));
        assert!(S3::new(S3Config::new("http://nas.local:9000", "b", "k", "s").allow_plain_http(true)).is_ok());
        assert!(S3::new(S3Config::new("https://key:secret@s3.example.com", "b", "k", "s")).is_err());
        assert!(S3::new(S3Config::new("https://s3.example.com/?X-Amz-Credential=x", "b", "k", "s")).is_err());
        assert!(!format!("{:?}", S3Config::new("https://s3.example.com", "b", "k", "hunter2")).contains("hunter2"));
    }
}
