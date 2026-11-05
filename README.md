# lib-net

Networking, trust and reporting for droidtop and Enginehost, and later Pythia and the agent: one of the shared
library families of Droidtop/tracker#867. The libraries belong to no program; each program has its own scaffold
and pins a tagged release of each library it uses (#867 D2).

This is Droidtop/droidtop-shared, moved here with its history (#867 step 1: "reuse and contract first, rewrites
second"). Its modules are unchanged and keep their package, `dev.droidtop.shared.*`, so nothing that uses them is
edited. `:version` and `:launch-contract` are not networking; they stay here until the Kotlin side of lib-games
and lib-engines exists. Every module is plain JVM: the Kotlin standard library and `java.security` only, no
Android API, so its tests run on a build machine with no device and no SDK.

| Module | Owns |
|---|---|
| [`:version`](version/README.md) | Version parsing and ordering: strict dotted versions with a build counter, and loose ordering of free-form strings |
| [`:launch-contract`](launch-contract/README.md) | The names droidtop and Enginehost agree on to start each other: intent actions and extras, the `enginehost.json` keys, and a shape check |
| [`:net`](net/README.md) | One GitHub client for both apps: which requests may carry a token and redirects decided per hop, conditional release listings with pagination and rate-limit errors, bounded downloads |
| [`:report`](report/README.md) | A bug reporter that needs no permission: one report model the person previews and edits, a redactor, and senders (Bugzilla REST, GitHub/Gitea issue form, email). No key ships in an app |
| [`:trust`](trust/README.md) | Checking signatures: the certificate-chain JWS both apps use, and SHA-256 helpers. Pins no key |
| `:testpki` | Test-only: throwaway certificate chains and signatures for the tests |

**Rust crates** (Cargo workspace at the root; Droidtop/tracker#867 D3):

| Crate | Owns |
|---|---|
| [`net-remote`](crates/remote) | The person's own shares and cloud storage behind one interface, `Remote`: list, stat, read, ranged read, a write that lands in one step, rename, remove one file, make folders. Backends so far: a local folder, WebDAV (Nextcloud, ownCloud, most NAS) SFTP (any SSH server, its key pinned) and S3-compatible storage. A share is a network resource for many uses; lib-sync uses it as one sync backend (Droidtop/tracker#573) |

Licence: GPL-3.0 (see `LICENSE`).

## Build and test

CI (`.github/workflows/ci.yml`) runs `./gradlew test` on JDK 17 and, for the Rust crates, `cargo fmt`, `clippy`
and `cargo test` (WebDAV against a real WebDAV server it starts) plus a build for both Android ABIs, then builds and tests each program that uses
this library against the commit, through that program's own `library-check.yml` (#867 D2): droidtop and
Enginehost. Work lands on `main` from `ci/*` branches once CI is green.

## Versions

**The public API stays compatible within a version** (Droidtop/tracker#867, owner 2026-11-04: "I should be able to
bump the version number and immediately get the benefit"). A program takes a fix by moving its pin, with no change to
its own code. Each module's public API is checked in as `<module>/api/<module>.api` (the Kotlin binary-compatibility
validator): `./gradlew apiCheck` fails when the API is not its dump, and `./gradlew apiDump` rewrites it.
`scripts/check-api-compat.sh` fails when a line was removed from a dump since the last release, unless the version is
a new minor (in 0.x) and the CHANGELOG says what broke and what a program changes. Adding to the API only needs the
new dump.

The library's version is `lib.version` in `gradle.properties`; the Rust workspace's version in `Cargo.toml` is the
same number, and CI fails when they differ. For the Rust crates, `cargo-semver-checks` holds the interface to the
last release, as the API dumps do for the Kotlin modules. A release is a tag `vX.Y.Z`: CI creates it on a
green `main` whose version is not yet tagged, so to release, raise the version (and the CHANGELOG) in the change
that needs it. Each program pins a tag.
