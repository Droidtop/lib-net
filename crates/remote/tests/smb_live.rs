//! The SMB backend against a real Samba server. CI starts one (`.github/workflows/ci.yml`) and names it in
//! `NET_REMOTE_SMB_SERVER` (`host:port`), `_SHARE`, `_USER` and `_PASSWORD`. Without them the test says so and
//! passes; CI fails if the server is not named (`NET_REMOTE_SMB_REQUIRED`).
#![cfg(all(feature = "smb", unix))]

use std::env;
use std::time::Duration;

use net_remote::{Error, Remote, RemotePath, Smb, SmbConfig};

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

#[test]
fn a_real_smb_server() {
    let Some(server) = env::var("NET_REMOTE_SMB_SERVER").ok().filter(|s| !s.is_empty()) else {
        assert!(env::var("NET_REMOTE_SMB_REQUIRED").is_err(), "NET_REMOTE_SMB_REQUIRED is set but no server is named");
        eprintln!("no SMB server named in NET_REMOTE_SMB_SERVER; skipped");
        return;
    };
    let var = |k: &str| env::var(k).unwrap_or_default();
    let config = |password: &str| {
        SmbConfig::new(&server, var("NET_REMOTE_SMB_SHARE"), var("NET_REMOTE_SMB_USER"), password).timeout(Duration::from_secs(15))
    };

    // A wrong password is a refused sign-in; an unknown share is refused, not "not found".
    assert!(matches!(Smb::connect(config("not the password")), Err(Error::SignInRefused)));
    let no_share = SmbConfig::new(&server, "no-such-share", var("NET_REMOTE_SMB_USER"), var("NET_REMOTE_SMB_PASSWORD"));
    assert!(matches!(Smb::connect(no_share), Err(Error::Refused(_))));

    let smb = Smb::connect(config(&var("NET_REMOTE_SMB_PASSWORD"))).unwrap();
    let run = format!("run-{}", std::process::id());
    let dir = p(&format!("{run}/saves/Game One"));
    smb.make_dirs(&dir).unwrap();
    smb.make_dirs(&dir).unwrap();

    let file = dir.join("slot 1.sav").unwrap();
    smb.write(&file, b"first").unwrap();
    smb.write(&file, b"second save").unwrap();
    assert_eq!(smb.read(&file).unwrap(), b"second save");
    assert_eq!(smb.read_range(&file, 7, 4).unwrap(), b"save");
    assert_eq!(smb.read_range(&file, 7, 100).unwrap(), b"save");
    let listed = smb.list(&dir).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "slot 1.sav");
    assert_eq!(listed[0].size, 11);
    assert!(!listed[0].is_dir);
    assert!(listed[0].mtime_ms > 0);

    // A large file, over several reads and writes.
    let big: Vec<u8> = (0..(3u32 << 20)).map(|i| (i % 251) as u8).collect();
    let big_file = dir.join("big.bin").unwrap();
    smb.write(&big_file, &big).unwrap();
    assert_eq!(smb.read(&big_file).unwrap(), big);
    assert_eq!(smb.read_range(&big_file, 2_000_000, 3).unwrap(), big[2_000_000..2_000_003].to_vec());
    smb.remove(&big_file).unwrap();

    let other = dir.join("slot 2.sav").unwrap();
    smb.write(&other, b"two").unwrap();
    smb.rename(&other, &file).unwrap();
    assert_eq!(smb.read(&file).unwrap(), b"two");
    assert!(smb.stat(&other).unwrap().is_none());
    assert!(smb.stat(&dir).unwrap().unwrap().is_dir);

    assert!(matches!(smb.read(&dir.join("none").unwrap()), Err(Error::NotFound(_))));
    assert!(matches!(smb.list(&p(&format!("{run}/none"))), Err(Error::NotFound(_))));
    assert!(matches!(smb.write(&p(&format!("{run}/no/such/x")), b"x"), Err(Error::NotFound(_))));
    smb.remove(&dir.join("none").unwrap()).unwrap();
    assert!(matches!(smb.remove(&dir), Err(Error::Refused(_))), "a folder is never removed");
    smb.remove(&file).unwrap();
    assert!(smb.list(&dir).unwrap().is_empty());
}

#[test]
fn a_server_that_does_not_answer_is_unreachable() {
    // Port 9 on this computer: nothing listens there.
    let result = Smb::connect(SmbConfig::new("127.0.0.1:9", "s", "u", "p").timeout(Duration::from_secs(5)));
    assert!(matches!(result, Err(Error::Unreachable(_))), "{:?}", result.err());
}
