//! Private, immutable ACP error records, including failures before a turn exists.
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn write(
    directory: &Path,
    phase: &str,
    error: &super::acp::Error,
) -> io::Result<PathBuf> {
    let path = directory.join(format!("adapter-error-{}.json", uuid::Uuid::new_v4()));
    let record = serde_json::json!({
        "version": 1,
        "recorded_at_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "phase": phase,
        "error": error,
    });
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut file, &record)?;
        file.write_all(b"\n")?;
        file.sync_all()
    })();
    if let Err(error) = result {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnostics_preserve_error_data_in_unique_private_files() {
        let directory = tempfile::tempdir().unwrap();
        let error = super::super::acp::Error::new(-32603, "load failed\nwith detail")
            .data(json!({"details": "Session not found", "code": "CANCELLED"}));
        let first = write(directory.path(), "session/load", &error).unwrap();
        let second = write(directory.path(), "session/load", &error).unwrap();
        assert_ne!(first, second);
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(first.clone()).unwrap()).unwrap();
        assert_eq!(record["phase"], "session/load");
        assert_eq!(record["error"], serde_json::to_value(error).unwrap());
        assert!(record["recorded_at_unix_ms"].as_u64().unwrap() > 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(first).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn diagnostics_report_write_failure_without_replacing_the_acp_error() {
        let directory = tempfile::tempdir().unwrap();
        let error = super::super::protocol_error(
            "session/load",
            super::super::acp::Error::new(-32603, "private detail"),
            Some(&directory.path().join("missing")),
        );
        assert_eq!(error.code, "AGENT_ERROR");
        let details = error.details.as_ref().unwrap();
        assert_eq!(details["acp_error_code"], -32603);
        assert!(details["adapter_diagnostic_write_error"].is_string());
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private detail")
        );
    }
}
