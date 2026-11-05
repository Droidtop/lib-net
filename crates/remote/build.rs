//! Builds libsmb2 (vendor/libsmb2, sahlberg/libsmb2 at libsmb2-6.3, LGPL-2.1-or-later) into net-remote when the
//! `smb` feature is on and the target is a unix (Linux, Android, macOS). On Windows a `\\server\share` path is
//! already a folder [`LocalFolder`] reads, so the C library is not built there.
//!
//! The source list and the configuration follow the tag's own lib/CMakeLists.txt and cmake/config.h.cmake:
//! every source in lib/ except krb5-wrapper.c (no GSSAPI or Kerberos: the person signs in with NTLMSSP), and a
//! config.h naming the POSIX headers every unix target has. compat.c supplies getlogin_r itself on Android below
//! API 28.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "smb")]
    smb::build();
}

#[cfg(feature = "smb")]
mod smb {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    /// lib/'s sources at libsmb2-6.3, as its lib/CMakeLists.txt lists them, without krb5-wrapper.c.
    const SOURCES: &[&str] = &[
        "aes.c",
        "aes128ccm.c",
        "alloc.c",
        "asn1-ber.c",
        "compat.c",
        "dcerpc.c",
        "dcerpc-lsa.c",
        "dcerpc-srvsvc.c",
        "errors.c",
        "hmac.c",
        "hmac-md5.c",
        "init.c",
        "libsmb2.c",
        "md4c.c",
        "md5.c",
        "ntlmssp.c",
        "pdu.c",
        "sha1.c",
        "sha224-256.c",
        "sha384-512.c",
        "smb2-cmd-close.c",
        "smb2-cmd-create.c",
        "smb2-cmd-echo.c",
        "smb2-cmd-error.c",
        "smb2-cmd-flush.c",
        "smb2-cmd-ioctl.c",
        "smb2-cmd-lock.c",
        "smb2-cmd-logoff.c",
        "smb2-cmd-negotiate.c",
        "smb2-cmd-notify-change.c",
        "smb2-cmd-oplock-break.c",
        "smb2-cmd-query-directory.c",
        "smb2-cmd-query-info.c",
        "smb2-cmd-read.c",
        "smb2-cmd-session-setup.c",
        "smb2-cmd-set-info.c",
        "smb2-cmd-tree-connect.c",
        "smb2-cmd-tree-disconnect.c",
        "smb2-cmd-write.c",
        "smb2-data-file-info.c",
        "smb2-data-filesystem-info.c",
        "smb2-data-reparse-point.c",
        "smb2-data-security-descriptor.c",
        "smb2-share-enum.c",
        "smb2-signing.c",
        "smb3-seal.c",
        "socket.c",
        "spnego-wrapper.c",
        "sync.c",
        "timestamps.c",
        "unicode.c",
        "usha.c",
    ];

    /// The headers cmake/ConfigureChecks.cmake finds on every unix target.
    const HAVE: &[&str] = &[
        "HAVE_ARPA_INET_H",
        "HAVE_ERRNO_H",
        "HAVE_FCNTL_H",
        "HAVE_INTTYPES_H",
        "HAVE_LINGER",
        "HAVE_NETDB_H",
        "HAVE_NETINET_IN_H",
        "HAVE_NETINET_TCP_H",
        "HAVE_POLL_H",
        "HAVE_SOCKADDR_STORAGE",
        "HAVE_STDINT_H",
        "HAVE_STDIO_H",
        "HAVE_STDLIB_H",
        "HAVE_STRINGS_H",
        "HAVE_STRING_H",
        "HAVE_SYS_IOCTL_H",
        "HAVE_SYS_POLL_H",
        "HAVE_SYS_SOCKET_H",
        "HAVE_SYS_STAT_H",
        "HAVE_SYS_TIME_H",
        "HAVE_SYS_TYPES_H",
        "HAVE_SYS_UIO_H",
        "HAVE_TIME_H",
        "HAVE_UNISTD_H",
        "STDC_HEADERS",
    ];

    pub fn build() {
        if env::var("CARGO_CFG_TARGET_FAMILY").map(|f| !f.split(',').any(|x| x == "unix")).unwrap_or(true) {
            return;
        }
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../vendor/libsmb2");
        if !root.join("lib/libsmb2.c").is_file() {
            panic!("vendor/libsmb2 is missing: run `git submodule update --init vendor/libsmb2` in lib-net");
        }
        let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
        let mut config = String::from("/* net-remote's build.rs: the unix answers to libsmb2's cmake/config.h.cmake */\n");
        for name in HAVE {
            config.push_str(&format!("#define {name} 1\n"));
        }
        if env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
            config.push_str("#define HAVE_SOCKADDR_LEN 1\n");
        }
        fs::write(out.join("config.h"), config).expect("config.h is written");

        let mut build = cc::Build::new();
        build
            .include(&out)
            .include(root.join("include"))
            .include(root.join("include/smb2"))
            .include(root.join("lib"))
            .define("HAVE_CONFIG_H", None)
            .define("_U_", Some("__attribute__((unused))"))
            .define("_GNU_SOURCE", None)
            .warnings(false);
        for source in SOURCES {
            build.file(root.join("lib").join(source));
        }
        build.compile("smb2");
        println!("cargo:rerun-if-changed={}", root.join("lib").display());
        println!("cargo:rerun-if-changed={}", root.join("include").display());
    }
}
