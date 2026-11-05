//! The SFTP backend against a real OpenSSH server. CI starts one (`.github/workflows/ci.yml`) and names it in
//! `NET_REMOTE_SFTP_HOST`, `_PORT`, `_USER`, `_PASSWORD`, `_HOST_KEY` (its SHA-256 fingerprint) and `_FOLDER`.
//! Without them the test says so and passes; CI fails if the server is not named (`NET_REMOTE_SFTP_REQUIRED`).

use std::env;
use std::time::Duration;

use net_remote::{is_temporary, Error, Remote, RemotePath, Sftp, SftpConfig};

struct Server {
    host: String,
    port: u16,
    user: String,
    password: String,
    host_key: String,
    folder: String,
}

fn server() -> Option<Server> {
    let host = env::var("NET_REMOTE_SFTP_HOST").ok().filter(|h| !h.is_empty());
    if host.is_none() && env::var("NET_REMOTE_SFTP_REQUIRED").is_ok() {
        panic!("NET_REMOTE_SFTP_REQUIRED is set but NET_REMOTE_SFTP_HOST is not");
    }
    let var = |k: &str| env::var(k).unwrap_or_default();
    Some(Server {
        host: host?,
        port: var("NET_REMOTE_SFTP_PORT").parse().unwrap_or(22),
        user: var("NET_REMOTE_SFTP_USER"),
        password: var("NET_REMOTE_SFTP_PASSWORD"),
        host_key: var("NET_REMOTE_SFTP_HOST_KEY"),
        folder: var("NET_REMOTE_SFTP_FOLDER"),
    })
}

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

fn config(s: &Server) -> SftpConfig {
    SftpConfig::new(&s.host, &s.user).port(s.port).folder(&s.folder).timeout(Duration::from_secs(15))
}

#[test]
fn a_real_sftp_server() {
    let Some(s) = server() else {
        eprintln!("no SFTP server named in NET_REMOTE_SFTP_HOST; skipped");
        return;
    };

    // Its key is never trusted silently: the first connect says what it is.
    match Sftp::connect(config(&s).password(&s.password)) {
        Err(Error::HostKeyUnknown(found)) => assert_eq!(found, s.host_key),
        Err(e) => panic!("expected HostKeyUnknown, got {e}"),
        Ok(_) => panic!("connected without a pinned key"),
    }
    // A different pinned key is a changed key.
    match Sftp::connect(config(&s).password(&s.password).host_key("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")) {
        Err(Error::HostKeyChanged { found, .. }) => assert_eq!(found, s.host_key),
        Err(e) => panic!("expected HostKeyChanged, got {e}"),
        Ok(_) => panic!("connected with the wrong key"),
    }
    // A wrong password is a refused sign-in.
    assert!(matches!(Sftp::connect(config(&s).password("not the password").host_key(&s.host_key)), Err(Error::SignInRefused)));

    let sftp = Sftp::connect(config(&s).password(&s.password).host_key(&s.host_key)).unwrap();
    let run = format!("run-{}", std::process::id());
    let dir = p(&format!("{run}/saves/Game One"));
    sftp.make_dirs(&dir).unwrap();
    sftp.make_dirs(&dir).unwrap();

    let file = dir.join("slot 1.sav").unwrap();
    sftp.write(&file, b"first").unwrap();
    sftp.write(&file, b"second save").unwrap();
    assert_eq!(sftp.read(&file).unwrap(), b"second save");
    assert_eq!(sftp.read_range(&file, 7, 4).unwrap(), b"save");
    assert_eq!(sftp.read_range(&file, 7, 100).unwrap(), b"save");
    let listed = sftp.list(&dir).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "slot 1.sav");
    assert_eq!(listed[0].size, 11);
    assert!(!listed[0].is_dir);
    assert!(listed[0].mtime_ms > 0);
    assert!(listed.iter().all(|e| !is_temporary(&e.name)));

    // A large file, several chunks.
    let big: Vec<u8> = (0..(3u32 << 20)).map(|i| (i % 251) as u8).collect();
    let big_file = dir.join("big.bin").unwrap();
    sftp.write(&big_file, &big).unwrap();
    assert_eq!(sftp.read(&big_file).unwrap(), big);
    assert_eq!(sftp.read_range(&big_file, 2_000_000, 3).unwrap(), big[2_000_000..2_000_003].to_vec());
    sftp.remove(&big_file).unwrap();

    let other = dir.join("slot 2.sav").unwrap();
    sftp.write(&other, b"two").unwrap();
    sftp.rename(&other, &file).unwrap();
    assert_eq!(sftp.read(&file).unwrap(), b"two");
    assert!(sftp.stat(&other).unwrap().is_none());
    assert!(sftp.stat(&dir).unwrap().unwrap().is_dir);

    assert!(matches!(sftp.read(&dir.join("none").unwrap()), Err(Error::NotFound(_))));
    assert!(matches!(sftp.list(&p(&format!("{run}/none"))), Err(Error::NotFound(_))));
    assert!(matches!(sftp.write(&p(&format!("{run}/no/such/x")), b"x"), Err(Error::NotFound(_))));
    sftp.remove(&dir.join("none").unwrap()).unwrap();
    assert!(matches!(sftp.remove(&dir), Err(Error::Refused(_))), "a folder is never removed");
    sftp.remove(&file).unwrap();
    assert!(sftp.list(&dir).unwrap().is_empty());
}

#[test]
fn a_server_that_does_not_answer_is_unreachable() {
    // Port 9 on this computer: nothing listens there.
    let result = Sftp::connect(SftpConfig::new("127.0.0.1", "me").port(9).password("x").timeout(Duration::from_secs(5)));
    assert!(matches!(result, Err(Error::Unreachable(_))));
}
