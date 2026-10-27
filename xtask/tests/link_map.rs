use lazypencil_xtask::{evidence::link_map, util};
use std::{fs, path::Path};

#[test]
fn archive_inventories_distinguish_linker_records_from_symbols_and_discarded_tables() {
    let header = "             VMA              LMA     Size Align Out     In      Symbol";
    let prefix = "          c50b40           c50b40       20    32         ";
    let archive = "/build with spaces/libnative.a";
    let lld = format!(
        "{header}\n{prefix}{archive}(one.o):(.text)\n{prefix}{archive}(one.o):(.rodata)\n{prefix}{archive}(two.o):(.text)\n{prefix}        /not/an/input.a(symbol.o)\n"
    );
    let mac = format!(
        "# Path: /build/app\n# Arch: arm64\n# Object files:\n[  0] linker synthesized\n[  1] {archive}(two.o)\n[  2] {archive}(one.o)\n# Sections:\n# Symbols:\n[  3] /not/an/input.a(symbol.o)\n"
    );
    let gnu = format!(
        "Archive member included to satisfy reference by file (symbol)\n\n{archive}(two.o)    /build/referrer.a(ignored.o) (symbol)\n{archive}(one.o)\n                              /build/entry.o (symbol)\n\nAs-needed library included to satisfy reference by file (symbol)\n/not/an/input.a(symbol.o)\nDiscarded input sections\n/not/an/input.a(discarded.o)\nLinker script and memory map\n"
    );
    for (format, text) in [("elf_lld", lld), ("macos_ld64", mac), ("elf_gnu", gnu)] {
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let report = link_map::archive_references(text.as_bytes()).unwrap();
            assert_eq!(report["format"], format);
            assert_eq!(report["map_sha256"], util::digest(text.as_bytes()));
            assert_eq!(report["map_bytes"], text.len());
            assert_eq!(report["archive_member_pairs"], 2);
            assert_eq!(
                report["archives"],
                serde_json::json!([{"archive":archive,"members":["one.o","two.o"]}])
            );
            assert!(report["scope"].as_str().unwrap().contains("discarded"));
        }
    }
}

#[test]
fn malformed_linker_records_never_produce_an_incomplete_success_inventory() {
    for text in [
        "",
        "Unknown map",
        "# Object files:\n[  0] linker synthesized\n# Sections:\n",
        "# Object files:\n[  1] /build/lib.a(member.o)\n",
        "# Object files:\n[  1] /build/lib.a(one.o)\n[  1] /build/lib.a(two.o)\n# Sections:\n",
        "# Object files:\n[  1] /build/lib.a()\n# Sections:\n",
        "# Object files:\n[  1] /build/lib.a(one.o\n# Sections:\n",
        "# Object files:\n[  1] relative/lib.a(one.o)\n# Sections:\n",
    ] {
        assert!(
            link_map::archive_references(text.as_bytes()).is_err(),
            "{text}"
        );
    }
    assert!(link_map::archive_references(b"\xff").is_err());
}

#[test]
fn opaque_symbol_bytes_do_not_change_or_corrupt_selected_archive_paths() {
    let maps: &[&[u8]] = &[
        b"# Object files:\n[  0] linker synthesized\n[  1] /build/lib.a(one.o)\n# Sections:\n# Symbols:\n0x100E98B63\t0x14\t[  1] literal string: \x0fmissing field `\xc0\x01`\n",
        b"             VMA              LMA     Size Align Out     In      Symbol\n          c50b40           c50b40       20    32         /build/lib.a(one.o):(.rodata.\xff)\n          c50b40           c50b40       20    32                 \xffsymbol\n",
        b"Archive member included to satisfy reference by file (symbol)\n\n/build/lib.a(one.o)    /build/referrer.a(\xff.o) (\xffsymbol)\nDiscarded input sections\n/ignored.a(\xff.o)\n",
    ];
    for raw in maps {
        let mut crlf = Vec::new();
        for byte in *raw {
            if *byte == b'\n' {
                crlf.push(b'\r');
            }
            crlf.push(*byte);
        }
        for raw in [raw.to_vec(), crlf] {
            let report = link_map::archive_references(&raw).unwrap();
            assert_eq!(report["map_sha256"], util::digest(&raw));
            assert_eq!(report["map_bytes"], raw.len());
            assert_eq!(report["archive_member_pairs"], 1);
            assert_eq!(
                report["archives"],
                serde_json::json!([{"archive":"/build/lib.a","members":["one.o"]}])
            );
        }
    }
    for raw in [
        b"# Object files:\n[  1] /build/lib\xff.a(one.o)\n# Sections:\n".as_slice(),
        b"# Object files:\n[  1] /build/lib.a(one\xff.o)\n# Sections:\n".as_slice(),
    ] {
        assert!(link_map::archive_references(raw).is_err());
    }
    let literal_replacement = "# Object files:\n[  1] /build/lib\u{fffd}.a(one.o)\n# Sections:\n";
    let report = link_map::archive_references(literal_replacement.as_bytes()).unwrap();
    assert_eq!(report["archives"][0]["archive"], "/build/lib\u{fffd}.a");
}

#[test]
fn persisted_linker_inventories_are_bound_to_original_bytes_and_never_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let map = temp.path().join("raw.map");
    let output = temp.path().join("inputs.json");
    let raw = b"# Object files:\n[  1] /build/libnative.a(native.o)\n# Sections:\n# Symbols:\nopaque \xff\xfe\n";
    fs::write(&map, raw).unwrap();
    let report = link_map::collect_inputs(&map, &output).unwrap();
    assert_eq!(util::json::<serde_json::Value>(&output).unwrap(), report);
    assert_eq!(report["map_sha256"], util::digest(raw));
    assert!(link_map::collect_inputs(&map, &output).is_err());
    assert_eq!(fs::read(&map).unwrap(), raw);
}

#[test]
fn normal_and_evidence_builds_share_every_compiler_and_linker_argument() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().canonicalize().unwrap();
    for host in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
    ] {
        let normal = link_map::cargo_arguments(host, &target, false).unwrap();
        let mut evidence = link_map::cargo_arguments(host, &target, true).unwrap();
        evidence.retain(|arg| arg != "--message-format=json");
        assert_eq!(normal, evidence, "{host}");
        assert_eq!(
            &normal[..6],
            [
                "cargo",
                "rustc",
                "--locked",
                "--release",
                "--bin",
                "lazypencil"
            ]
        );
        assert_eq!(normal.iter().filter(|arg| *arg == "--").count(), 1);
        assert!(normal.last().unwrap().ends_with(link_map::CACHE_PATH));
    }
}

#[test]
fn map_arguments_stay_on_the_root_rustc_invocation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("release map.txt");
    for host in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        let args = link_map::rustc_arguments(host, &path).unwrap().unwrap();
        assert_eq!(
            args,
            [
                "-C".to_owned(),
                format!("link-arg=-Wl,-Map={}", path.display())
            ]
        );
    }
    assert_eq!(
        link_map::rustc_arguments("aarch64-apple-darwin", &path)
            .unwrap()
            .unwrap(),
        [
            "-C".to_owned(),
            format!("link-arg=-Wl,-map,{}", path.display())
        ]
    );
    for host in ["x86_64-apple-darwin", "x86_64-pc-windows-msvc"] {
        assert!(link_map::rustc_arguments(host, &path).unwrap().is_none());
    }
    for path in [
        Path::new("relative.map").to_owned(),
        path.with_file_name("comma,map"),
        path.with_file_name("percent%map"),
        path.with_file_name("newline\nmap"),
    ] {
        assert!(link_map::rustc_arguments("x86_64-unknown-linux-gnu", &path).is_err());
    }
}

#[test]
fn raw_maps_keep_every_byte_and_reject_empty_or_reused_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input.map");
    let output = temp.path().join("link-map.map");
    let raw = b"# Object files:\r\n[  1] /build/lib.a(member.o)\r\n# Symbols:\r\n";
    fs::write(&source, raw).unwrap();
    let report = link_map::collect(&source, &output).unwrap();
    assert_eq!(fs::read(&output).unwrap(), raw);
    assert_eq!(report["sha256"], util::digest(raw));
    assert_eq!(report["bytes"], raw.len());
    assert!(link_map::collect(&source, &output).is_err());
    fs::write(&source, []).unwrap();
    let empty_output = temp.path().join("empty.map");
    assert!(link_map::collect(&source, &empty_output).is_err());
    assert!(!empty_output.exists());
}

#[cfg(unix)]
#[test]
fn map_collection_rejects_symlinked_inputs_and_dangling_outputs() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input.map");
    fs::write(&source, "map").unwrap();
    let alias = temp.path().join("alias.map");
    symlink(&source, &alias).unwrap();
    let output = temp.path().join("output.map");
    assert!(link_map::collect(&alias, &output).is_err());
    symlink(temp.path().join("absent"), &output).unwrap();
    assert!(link_map::collect(&source, &output).is_err());
}
