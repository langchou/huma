use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tracing::warn;

#[derive(Debug, Serialize)]
struct AuditRecord<'a> {
    v: u32,
    seq: u64,
    ts_ms: u64,
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<&'a str>,
    data: Value,
}

/// Append-only audit writer per `docs/standards/logging-audit.md`.
/// Write failures emit `WARN` and never crash active daemon sessions.
pub struct AuditLog {
    path: PathBuf,
    seq: AtomicU64,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            warn!(path = %parent.display(), error = %err, "failed to create audit directory");
        }
        Self {
            path,
            seq: AtomicU64::new(1),
        }
    }

    pub fn record(&self, event: &str, project_id: Option<&str>, data: Value) {
        let rec = AuditRecord {
            v: 1,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            ts_ms: now_ms(),
            event,
            project_id,
            data,
        };
        let Ok(line) = serde_json::to_string(&rec) else {
            warn!(event, "failed to serialize audit record");
            return;
        };

        if let Err(err) = append_secure_line(&self.path, &line) {
            warn!(path = %self.path.display(), event, error = %err, "audit write failed");
        }
    }
}

/// Redacts command arguments that may carry tokens, keys, or secret values.
pub fn redact_argv(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut redact_next = false;
    for arg in argv {
        if redact_next {
            out.push("[REDACTED]".to_string());
            redact_next = false;
            continue;
        }
        if let Some((flag, _val)) = arg.split_once('=') {
            if is_sensitive_key(flag) {
                out.push(format!("{flag}=[REDACTED]"));
                continue;
            }
        } else if is_sensitive_key(arg) {
            out.push(arg.clone());
            redact_next = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn is_sensitive_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("KEY")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("AUTH")
}

fn append_secure_line(path: &Path, line: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{line}")?;
    }
    #[cfg(not(unix))]
    {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_cli_arguments() {
        let argv = vec![
            "agy".into(),
            "acp".into(),
            "--api-key".into(),
            "super-secret-123".into(),
            "--auth_token=xyz987".into(),
            "--stdio".into(),
        ];
        assert_eq!(
            redact_argv(&argv),
            vec![
                "agy",
                "acp",
                "--api-key",
                "[REDACTED]",
                "--auth_token=[REDACTED]",
                "--stdio",
            ]
        );
    }

    #[test]
    fn appends_versioned_jsonl_records() {
        let dir = std::env::temp_dir().join(format!("huma-audit-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("audit.jsonl");
        let audit = AuditLog::new(path.clone());
        audit.record("project.opened", Some("p1"), json!({"driver": "agy"}));
        audit.record("project.closed", Some("p1"), json!({}));

        let raw = fs::read_to_string(&path).expect("read audit log");
        let lines: Vec<Value> = raw
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid json line"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["v"], 1);
        assert_eq!(lines[0]["seq"], 1);
        assert_eq!(lines[0]["event"], "project.opened");
        assert_eq!(lines[1]["seq"], 2);
        let _ = fs::remove_dir_all(dir);
    }
}
