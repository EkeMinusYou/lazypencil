use lazypencil_xtask::{evidence::link_map, util};
use std::{fs, path::Path};

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
