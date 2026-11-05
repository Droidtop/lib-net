//! The S3 backend against a real S3-compatible server. CI starts one (versitygw, `.github/workflows/ci.yml`) and
//! names it in `NET_REMOTE_S3_ENDPOINT`, `_BUCKET`, `_ACCESS_KEY` and `_SECRET_KEY`. Without them the test says so and
//! passes; CI fails if the server is not named (`NET_REMOTE_S3_REQUIRED`).

use std::env;
use std::time::Duration;

use net_remote::{Error, Remote, RemotePath, S3Config, S3};

fn p(s: &str) -> RemotePath {
    RemotePath::new(s).unwrap()
}

#[test]
fn a_real_s3_server() {
    let endpoint = env::var("NET_REMOTE_S3_ENDPOINT").ok().filter(|e| !e.is_empty());
    let Some(endpoint) = endpoint else {
        assert!(env::var("NET_REMOTE_S3_REQUIRED").is_err(), "NET_REMOTE_S3_REQUIRED is set but no server is named");
        eprintln!("no S3 server named in NET_REMOTE_S3_ENDPOINT; skipped");
        return;
    };
    let var = |k: &str| env::var(k).unwrap_or_default();
    let config = |secret: &str| {
        S3Config::new(&endpoint, var("NET_REMOTE_S3_BUCKET"), var("NET_REMOTE_S3_ACCESS_KEY"), secret)
            .folder(format!("run-{}", std::process::id()))
            .allow_plain_http(endpoint.starts_with("http://127.0.0.1"))
            .timeout(Duration::from_secs(15))
    };

    // A wrong secret is a refused sign-in.
    let wrong = S3::new(config("not the secret")).unwrap();
    assert!(matches!(wrong.list(&RemotePath::root()), Err(Error::SignInRefused)), "{:?}", wrong.list(&RemotePath::root()).err());

    let s3 = S3::new(config(&var("NET_REMOTE_S3_SECRET_KEY"))).unwrap();
    let dir = p("saves/Game One");
    s3.make_dirs(&dir).unwrap();
    s3.make_dirs(&dir).unwrap();
    assert!(s3.list(&dir).unwrap().is_empty(), "a new folder is empty");

    let file = dir.join("slot 1.sav").unwrap();
    s3.write(&file, b"first").unwrap();
    s3.write(&file, b"second save").unwrap();
    assert_eq!(s3.read(&file).unwrap(), b"second save");
    assert_eq!(s3.read_range(&file, 7, 4).unwrap(), b"save");
    assert_eq!(s3.read_range(&file, 7, 100).unwrap(), b"save");
    let listed = s3.list(&dir).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "slot 1.sav");
    assert_eq!(listed[0].size, 11);
    assert!(!listed[0].is_dir);
    assert!(listed[0].mtime_ms > 0);
    let parent = s3.list(&p("saves")).unwrap();
    assert_eq!(parent.len(), 1, "{parent:?}");
    assert!(parent[0].is_dir);
    assert_eq!(parent[0].name, "Game One");

    let other = dir.join("slot 2.sav").unwrap();
    s3.write(&other, b"two").unwrap();
    s3.rename(&other, &file).unwrap();
    assert_eq!(s3.read(&file).unwrap(), b"two");
    assert!(s3.stat(&other).unwrap().is_none());
    assert_eq!(s3.stat(&file).unwrap().unwrap().size, 3);
    assert!(s3.stat(&dir).unwrap().unwrap().is_dir);

    assert!(matches!(s3.read(&dir.join("none").unwrap()), Err(Error::NotFound(_))));
    assert!(matches!(s3.list(&p("none")), Err(Error::NotFound(_))));
    assert!(matches!(s3.write(&p("no/such/x"), b"x"), Err(Error::NotFound(_))));
    s3.remove(&dir.join("none").unwrap()).unwrap();
    assert!(matches!(s3.remove(&dir), Err(Error::Refused(_))), "a folder is never removed");
    s3.remove(&file).unwrap();
    assert!(s3.list(&dir).unwrap().is_empty());
}
