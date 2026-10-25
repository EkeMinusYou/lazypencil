use lazypencil_xtask::evidence::linkage;
use serde_json::json;

fn elf() -> String {
    // Synthesized readelf fields, not an upstream executable or fixture dump.
    "ELF Header:\n  Class: ELF64\n  [Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\nDynamic section:\n 0x1 (NEEDED) Shared library: [libc.so.6]\n 0x1 (NEEDED) Shared library: [libgcc_s.so.1]\n 0x1d (RUNPATH) Library runpath: [$ORIGIN/lib]\nVersion needs section '.gnu.version_r' contains 2 entries:\n  0x0: Version: 1 File: libc.so.6 Cnt: 4\n  0x0: Name: GLIBC_2.9 Flags: none Version: 2\n  0x0: Name: GLIBC_2.34 Flags: none Version: 3\n  0x0: Name: GLIBC_2.38 Flags: none Version: 4\n  0x0: Name: GLIBC_ABI_DT_RELR Flags: none Version: 5\n  0x0: Version: 1 File: libgcc_s.so.1 Cnt: 1\n  0x0: Name: GCC_3.0 Flags: none Version: 6\n".into()
}

fn macho() -> String {
    "/original/executable:\nLoad command 0\n cmd LC_LOAD_DYLINKER\n cmdsize 32\n name /usr/lib/dyld (offset 12)\nLoad command 1\n cmd LC_BUILD_VERSION\n cmdsize 32\n platform 1\n minos 11.0\n sdk 15.2\n ntools 1\nLoad command 2\n cmd LC_LOAD_DYLIB\n cmdsize 56\n name /usr/lib/libSystem.B.dylib (offset 24)\n current version 1351.0.0\n compatibility version 1.0.0\nLoad command 3\n cmd LC_LOAD_WEAK_DYLIB\n cmdsize 48\n name @rpath/optional.dylib (offset 24)\n current version 2.1.0\n compatibility version 2.0.0\nLoad command 4\n cmd LC_RPATH\n cmdsize 40\n path @executable_path/lib (offset 12)\n".into()
}

#[test]
fn load_requirements_keep_numeric_versions_and_non_numeric_abi_requirements_distinct() {
    let report = linkage::elf(&elf()).unwrap();
    assert_eq!(report["maximum_numeric_glibc_requirement"], "2.38");
    assert_eq!(
        report["non_numeric_glibc_requirements"],
        json!(["ABI_DT_RELR"])
    );
    assert_eq!(report["needed"], json!(["libc.so.6", "libgcc_s.so.1"]));
    assert_eq!(report["search_paths"][0]["value"], "$ORIGIN/lib");
    assert_eq!(
        report["required_symbol_versions"]["libgcc_s.so.1"],
        json!(["GCC_3.0"])
    );
    for broken in [
        elf().replace("Cnt: 4", "Cnt: 5"),
        elf().replace("File: libc.so.6", "File: undeclared.so"),
        elf().replace("Version needs section", "Version definition section"),
        elf().replace("/lib64/ld-linux-x86-64.so.2", "relative-loader"),
        format!("{}[Requesting program interpreter: /other/loader]\n", elf()),
    ] {
        assert!(linkage::elf(&broken).is_err(), "{broken}");
    }
}

#[test]
fn mach_o_keeps_weak_loads_rpaths_and_binary_minimum_without_claiming_runtime_support() {
    let report = linkage::mach_o(&macho()).unwrap();
    assert_eq!(
        report["build_version"],
        json!({"platform":"macos","minimum_os":"11.0","sdk":"15.2"})
    );
    assert_eq!(report["needed"][1]["kind"], "LC_LOAD_WEAK_DYLIB");
    assert_eq!(report["search_paths"], json!(["@executable_path/lib"]));
    for broken in [
        macho().replace("platform 1", "platform 2"),
        macho().replace("minos 11.0", "minos unknown"),
        macho().replace("cmd LC_BUILD_VERSION", "cmd LC_VERSION_MIN_MACOSX"),
        macho().replace("current version 2.1.0", "current version unknown"),
        macho().replace("name /usr/lib/dyld (offset 12)", "name /usr/lib/dyld"),
        format!(
            "{}Load command 5\n cmd LC_BUILD_VERSION\n platform 1\n minos 11.0\n sdk 15.2\n",
            macho()
        ),
    ] {
        assert!(linkage::mach_o(&broken).is_err(), "{broken}");
    }
}

#[test]
fn binary_headers_bind_the_observed_architecture_and_reject_deferred_or_fat_targets() {
    let mut header = [0u8; 64];
    header[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    header[16..18].copy_from_slice(&3u16.to_le_bytes());
    header[18..20].copy_from_slice(&62u16.to_le_bytes());
    assert_eq!(
        linkage::format(&header, "x86_64-unknown-linux-gnu").unwrap(),
        "elf"
    );
    assert!(linkage::format(&header, "aarch64-unknown-linux-gnu").is_err());
    header[18..20].copy_from_slice(&183u16.to_le_bytes());
    assert_eq!(
        linkage::format(&header, "aarch64-unknown-linux-gnu").unwrap(),
        "elf"
    );
    header[5] = 2;
    assert!(linkage::format(&header, "aarch64-unknown-linux-gnu").is_err());
    header = [0; 64];
    header[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    header[4..8].copy_from_slice(&0x0100_000cu32.to_le_bytes());
    header[12..16].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        linkage::format(&header, "aarch64-apple-darwin").unwrap(),
        "mach-o"
    );
    header[..4].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
    assert!(linkage::format(&header, "aarch64-apple-darwin").is_err());
    assert!(linkage::format(&header, "x86_64-apple-darwin").is_err());
    assert!(linkage::format(&header, "x86_64-pc-windows-msvc").is_err());
    assert!(linkage::format(&header[..8], "aarch64-apple-darwin").is_err());
}

#[test]
#[cfg(any(
    all(
        target_os = "linux",
        target_env = "gnu",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(target_os = "macos", target_arch = "aarch64")
))]
fn real_inspector_reads_this_lightweight_test_executable_without_running_the_loader() {
    let host = linkage::native_host().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let output = temp.path().join("linkage");
    let report = linkage::collect(&executable, host, &output).unwrap();
    assert_eq!(
        report["binary_sha256"],
        lazypencil_xtask::util::file_digest(&executable).unwrap()
    );
    assert_eq!(report["host"], host);
    assert!(
        !report["declarations"]["needed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(output.join("linkage.json").is_file());
    assert!(linkage::collect(&executable, host, &output).is_err());
}
