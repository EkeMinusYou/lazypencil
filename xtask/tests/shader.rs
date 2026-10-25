use lazypencil_xtask::{shader, util};
use std::{fs, path::PathBuf};
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let source = util::root();
    for directory in [
        "src/shader/preprocessor",
        "resources/shader-preprocessor",
        "resources/licenses/shader-preprocessor",
    ] {
        for path in util::walk_filtered(&source.join(directory), |p| {
            p.components()
                .any(|c| c.as_os_str() == "target" || c.as_os_str() == "__pycache__")
        })
        .unwrap()
        {
            let output = root.join(path.strip_prefix(&source).unwrap());
            fs::create_dir_all(output.parent().unwrap()).unwrap();
            fs::copy(path, output).unwrap();
        }
    }
    (temp, root)
}
#[test]
fn shipped_wasm_binds_exact_source_notices_imports_and_memory_limit() {
    shader::verify(&util::root(), None).unwrap();
}
#[test]
fn changed_component_module_build_metadata_or_rebuilt_bytes_are_rejected() {
    for mutation in ["source", "binary", "hash", "rebuild"] {
        let (_temp, root) = fixture();
        match mutation {
            "source" => fs::write(
                root.join("src/shader/preprocessor/src/lib.rs"),
                b"changed source",
            )
            .unwrap(),
            "binary" => fs::write(
                root.join("resources/shader-preprocessor/module.wasm"),
                b"changed module",
            )
            .unwrap(),
            "hash" => {
                let path = root.join("resources/shader-preprocessor/build.json");
                let mut report: serde_json::Value = util::json(&path).unwrap();
                report["wasm_sha256"] = "0".repeat(64).into();
                fs::write(path, util::encode(&report).unwrap()).unwrap();
            }
            _ => {
                let path = root.join("wrong.wasm");
                fs::write(&path, b"another module").unwrap();
                assert!(shader::verify(&root, Some(&path)).is_err());
                continue;
            }
        }
        assert!(shader::verify(&root, None).is_err(), "{mutation}");
    }
}
#[test]
fn wasm_import_policy_rejects_host_functions_and_unbounded_or_larger_memory() {
    let valid = b"\0asm\x01\0\0\0\x02\x12\x01\x03env\x06memory\x02\x01\x80\x02\x80\x08";
    shader::memory_import(valid).unwrap();
    for bytes in [
        b"\0asm\x01\0\0\0\x02\x0b\x01\x03env\x03foo\x00\x00".as_slice(),
        b"\0asm\x01\0\0\0\x02\x12\x01\x03env\x06memory\x02\x01\x80\x02\x80\x10".as_slice(),
        b"\0asm\x01\0\0\0\x02\x10\x01\x03env\x06memory\x02\x00\x80\x02".as_slice(),
    ] {
        assert!(shader::memory_import(bytes).is_err());
    }
}
