# Changelog

All notable changes to lib-net (droidtop-shared until 0.2.0). Versions follow [Semantic Versioning](https://semver.org/)
(in 0.x, only a new minor version may break the API; CI enforces it).

## [0.2.3]

### Added
- `net-remote`: S3-compatible storage (`S3`, `S3Config`; Droidtop/tracker#573): AWS S3, Backblaze B2, Wasabi,
  Cloudflare R2, MinIO, Garage. The person's own access key and secret; each request signed with rusty-s3 (SigV4,
  valid five minutes) and sent with the same ureq client as WebDAV, https only unless the person allows plain http.
  A folder is a key prefix with an empty marker object per level; a write is one PUT; a rename is a copy through
  this device and a delete (rusty-s3 signs no server-side copy). CI tests it against versitygw, a real S3 server
  that checks signatures.

## [0.2.2]

### Added
- `net-remote`: SFTP (`Sftp`, `SftpConfig`; Droidtop/tracker#573). Any SSH server, signed in with the person's
  password or an OpenSSH private key. The server's key is pinned by its SHA-256 fingerprint: without one, connecting
  fails with `Error::HostKeyUnknown` carrying the server's fingerprint for the person to check and accept, and with a
  different one, `Error::HostKeyChanged`. russh 0.65 and russh-sftp 3, on ring, run on a runtime of the backend's
  own, so `Remote` stays blocking. CI tests it against a real OpenSSH server.
- `Error::HostKeyUnknown` and `Error::HostKeyChanged` (the enum is `#[non_exhaustive]`).

## [0.2.1]

### Added
- `net-remote`, the first Rust crate here (Droidtop/tracker#573; owner, 2026-11-05: "Share client? That's be in
  lib-net. lib-sync controls synchronization. A share can be a backend for a sync, but it's really just a network
  resource we can use for lots of stuff"). The `Remote` interface (list, stat, read, ranged read, a write that lands
  in one step through a temporary `.<name>.<tag>.part`, rename, remove one file and never a folder, make folders),
  checked share paths (`RemotePath`: no `..`, `.`, empty segment or backslash), and two backends: `LocalFolder` and
  `WebDav` (PROPFIND, GET with Range, PUT, MOVE, DELETE, MKCOL; Basic sign-in over https only unless the person
  allows plain http for that share; an address with a user or password in it refused; no redirects followed; a
  timeout on every request; uploads over 8 MiB streamed).
- A Cargo workspace at the root, on the same version as `lib.version`, with CI for it: fmt, clippy, tests (WebDAV
  against wsgidav), a build for arm64-v8a and x86_64 Android, and `cargo-semver-checks` against the last release.

## [0.2.0]

### Changed
- The library moved from Droidtop/droidtop-shared to Droidtop/lib-net, with its history, as one of the shared
  library families of Droidtop/tracker#867. Its modules and their package (`dev.droidtop.shared.*`) are
  unchanged; the version property is `lib.version`.
- Each module's public API is checked in (`<module>/api/<module>.api`) and CI holds the code to it: `apiCheck` fails
  on any difference, and `scripts/check-api-compat.sh` fails when an API line is removed since the last release
  without a breaking version (Droidtop/tracker#867).

Also in 0.2.0, from droidtop-shared after its 0.1.0:

### Added
- `:sitecheck`: how to tell a browser check (a Cloudflare challenge, DDoS-Guard) from the page (`SiteCheck.isChallenge`),
  which cookies are the check's clearance, when a web view has passed it, the `Clearance` model, and the `SiteSolver`
  (the browser's own try, nobody asked) and `SiteAsker` (the person) interfaces an app plugs its web view and screens
  into. Lifted from droidtop's `SiteChecks` (Droidtop/tracker#600) with its tests. Droidtop/tracker#751.
- `:net` `GitHubClient(onResponse = ...)`: told of each answer's address, status and `X-RateLimit-Remaining`, for an
  app's own log (Enginehost's one line per request, Droidtop/tracker#550, #195).
- `:launch-contract`: `EnginehostIntents` (actions, extras, reasons, the capabilities provider) and
  `EnginehostConfig` (the `enginehost.json` keys and `problems`, a shape check), every string as released builds
  send it. Droidtop/tracker#204.
- `:report`: `ReportDraft`/`Section`, `ReportComposer`, `Redactor` (from Enginehost's `scrub`; a folder
  name with spaces no longer leaks past the first space), and `ReportSender` with `BugzillaSender`,
  `IssueFormSender` and `EmailSender`. Droidtop/tracker#655.
- `:net`: `GitHubAuth` (token hosts matched exactly, redirects followed by hand with the header decided
  per hop), `GitHubClient` (`get`, `listReleases`, `download`), `GitHubHttpException` and
  `GitHubReleases`, lifted from droidtop's `GitHubAuth`/`Http` and Enginehost's `PluginCatalog`.
  Where the two disagreed on the token hosts, droidtop's three exact hosts win: Enginehost also
  sent the token to `objects.githubusercontent.com` and to subdomains of the three.
  Droidtop/tracker#194.
- `:version`: `Version`, `VersionSeries` and `VersionConstraint` (from Enginehost, with their
  tests), and `Versions.compareLoose` (from droidtop, with its tests). Droidtop/tracker#191.
- `:trust`: `ChainSignature` (the certificate-chain JWS verifier, from droidtop) and `Sha256`,
  with droidtop's tests ported onto a test-only `:testpki`. Droidtop/tracker#191.
- `:trust` `BundleTar`: streaming reader for tar.xz bundles with path, link, duplicate, size,
  count and memory refusals. Droidtop/tracker#191.
- `:trust` `PublishedKeys` (key-file addresses, reading a published key, anchor keys, third-party
  origin ids, and `select` over fetched bytes), `KeyRotation` and `Origins.normalizeGithub`, from
  droidtop's `PluginSourceKeys` and `UserOriginKeys` and Enginehost's `OfficialKeyRotation`.
  Droidtop/tracker#191.

