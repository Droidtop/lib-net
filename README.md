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

Licence: GPL-3.0 (see `LICENSE`).

## Build and test

CI (`.github/workflows/ci.yml`) runs `./gradlew test` on JDK 17, then builds and tests each program that uses
this library against the commit, through that program's own `library-check.yml` (#867 D2): droidtop and
Enginehost. Work lands on `main` from `ci/*` branches once CI is green.

## Versions

The library's version is `lib.version` in `gradle.properties`. A release is a tag `vX.Y.Z`: CI creates it on a
green `main` whose version is not yet tagged, so to release, raise the version (and the CHANGELOG) in the change
that needs it. Each program pins a tag.
