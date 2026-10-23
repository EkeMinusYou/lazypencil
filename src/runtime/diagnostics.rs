//! Read-only host facts; file integrity does not establish browser launchability.

use serde_json::{Value, json};

pub(crate) fn host_status() -> Value {
    let report = json!({
        "os":std::env::consts::OS,
        "architecture":std::env::consts::ARCH,
        "browser_sandbox":"required",
        "sandbox_profile":"not_checked",
        "shared_libraries":"not_checked",
    });
    #[cfg(target_os = "linux")]
    let report = {
        let mut report = report;
        report["linux"] = linux_policy(std::path::Path::new("/proc"));
        report["linux"]["effective_uid"] = json!(unsafe { libc::geteuid() });
        report
    };
    report
}

#[cfg(target_os = "linux")]
fn linux_policy(root: &std::path::Path) -> Value {
    json!({
        "unprivileged_userns_clone":read_integer(&root.join("sys/kernel/unprivileged_userns_clone")),
        "apparmor_restrict_unprivileged_userns":read_integer(&root.join("sys/kernel/apparmor_restrict_unprivileged_userns")),
        "max_user_namespaces":read_integer(&root.join("sys/user/max_user_namespaces")),
        "apparmor_documentation":"https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md",
    })
}

#[cfg(target_os = "linux")]
fn read_integer(path: &std::path::Path) -> Option<u64> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(129)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 128 {
        return None;
    }
    let value = std::str::from_utf8(&bytes).ok()?.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn host_policy_reports_facts_without_inferring_a_selected_browsers_profile() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("sys/kernel")).unwrap();
        fs::create_dir_all(root.join("sys/user")).unwrap();
        fs::write(root.join("sys/kernel/unprivileged_userns_clone"), "1\n").unwrap();
        fs::write(
            root.join("sys/kernel/apparmor_restrict_unprivileged_userns"),
            "1\n",
        )
        .unwrap();
        fs::write(root.join("sys/user/max_user_namespaces"), "63852\n").unwrap();
        let values = linux_policy(root);
        assert_eq!(values["unprivileged_userns_clone"], 1);
        assert_eq!(values["apparmor_restrict_unprivileged_userns"], 1);
        assert_eq!(values["max_user_namespaces"], 63852);
        fs::remove_file(root.join("sys/user/max_user_namespaces")).unwrap();
        assert!(linux_policy(root)["max_user_namespaces"].is_null());
        assert!(!root.join("data").exists());
    }

    #[test]
    fn unknown_malformed_and_oversized_proc_values_stay_unknown() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("value");
        assert_eq!(read_integer(&file), None);
        assert_eq!(read_integer(temp.path()), None);
        for value in [
            "",
            "-1",
            "+1",
            "1\n2",
            "18446744073709551616",
            "private-value",
        ] {
            fs::write(&file, value).unwrap();
            assert_eq!(read_integer(&file), None, "Accepted {value:?}");
        }
        fs::write(&file, vec![b'0'; 129]).unwrap();
        assert_eq!(read_integer(&file), None);
        fs::write(&file, [0xff, 0xfe]).unwrap();
        assert_eq!(read_integer(&file), None);
        fs::write(&file, "0\n").unwrap();
        assert_eq!(read_integer(&file), Some(0));
    }
}
