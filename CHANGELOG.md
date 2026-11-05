# Changelog

All notable changes to lib-net (droidtop-shared until 0.2.0). Versions follow [Semantic Versioning](https://semver.org/)
(in 0.x, only a new minor version may break the API; CI enforces it).

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

