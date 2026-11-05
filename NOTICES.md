# Third-party code in lib-net

lib-net is GPL-3.0-only (see `LICENSE`). Code from other projects that it vendors or builds in is listed here, with
its licence. Each one's own licence files and copyright notices are kept as they came, in its folder.

## libsmb2

- **What:** a userspace SMB 2/3 client library, built into `net-remote`'s `smb` feature (`crates/remote/build.rs`,
  `crates/remote/src/smb.rs`) on unix targets.
- **Where:** `vendor/libsmb2`, a submodule of <https://github.com/sahlberg/libsmb2> pinned to tag `libsmb2-6.3`
  (tag object `6e7078b`, commit `2bcb014b23a8c8cccf534eea8bda2fe5df453cfd`).
- **Copyright:** Ronnie Sahlberg, with portions copyright Primary Data Inc. (`lib/libsmb2.c`).
- **Licence:** the library itself (`lib/` and `include/`) is under the GNU Lesser General Public License version 2.1
  or, at your option, any later version (its `COPYING`; the text is `LICENCE-LGPL-2.1.txt`). net-remote builds only
  `lib/` and `include/`. LGPL-2.1-or-later code may be combined into a GPL-3.0 work.
- **Inside `lib/`,** a few primitives carry their own notices, all compatible with the above:
  - `aes.c`: public domain;
  - `md5.c`: Colin Plumb, public domain;
  - `md4c.c`: RSA Data Security's RFC 1320 reference code;
  - `sha1.c`, `usha.c` and the other SHA files: the RFC 4634 reference code.
- **Not built:** `lib/krb5-wrapper.c` (Kerberos), and the `utils/`, `examples/` and `tests/` folders, which `COPYING`
  licenses separately.
- **Approved by the owner,** 2026-11-05 (Droidtop/tracker#573): "gotcha. Then yeah, go ahead"; "I've approved libsmb
  as a vendored smb implementation".
