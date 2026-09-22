//! Synchronous Profile transforms in a disposable, resource-bounded Boa process.
use crate::domain::{AppError, ErrorCode, ProfileId};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

mod worker;
pub use worker::worker_main;

pub const CAPABILITY: &str = "profile_scripts_v1";
pub const TEMPLATE: &str = "function main(config, profileName, context) {\n  // Modify the configuration, then return it.\n  return config;\n}\n";
pub const SOURCE_LIMIT: usize = 256 * 1024;
const WIRE_LIMIT: usize = 12 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileScript {
    pub draft: String,
    /// Editing or saving a draft never changes the enabled version.
    pub active: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScriptPreview {
    pub yaml: String,
    pub logs: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct WorkerRequest {
    scripts: Vec<String>,
    config: serde_json::Value,
    name: String,
    id: ProfileId,
}

#[derive(Serialize, Deserialize)]
struct WorkerResponse {
    result: Result<serde_json::Value, String>,
    logs: Vec<String>,
}

pub fn validate_source(source: &str) -> Result<(), AppError> {
    if source.len() > SOURCE_LIMIT {
        return Err(error("Script exceeds 256 KiB"));
    }
    Ok(())
}

fn error(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::ValidationFailed, message)
}

/// Reject YAML values that JavaScript cannot round-trip without data loss.
fn json_value(value: &serde_yaml::Value, depth: usize) -> Result<serde_json::Value, AppError> {
    use serde_yaml::Value;
    if depth > 128 {
        return Err(error("Configuration nesting exceeds 128 levels"));
    }
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(v) => (*v).into(),
        Value::String(v) => v.clone().into(),
        Value::Number(v) => {
            let n = v.as_f64().ok_or_else(|| error("Unsupported YAML number"))?;
            if !n.is_finite() || (n.fract() == 0.0 && n.abs() > 9_007_199_254_740_991.0) {
                return Err(error(
                    "Scripts require finite numbers and safe JavaScript integers",
                ));
            }
            serde_json::to_value(v).map_err(|_| error("Unsupported YAML number"))?
        }
        Value::Sequence(v) => serde_json::Value::Array(
            v.iter()
                .map(|v| json_value(v, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Value::Mapping(v) => {
            let mut object = serde_json::Map::new();
            for (key, value) in v {
                let key = key
                    .as_str()
                    .ok_or_else(|| error("Scripts require string mapping keys"))?;
                object.insert(key.into(), json_value(value, depth + 1)?);
            }
            serde_json::Value::Object(object)
        }
        Value::Tagged(_) => return Err(error("Tagged YAML values are unsupported by scripts")),
    })
}

pub fn transform(
    id: &ProfileId,
    name: &str,
    value: &serde_yaml::Value,
    scripts: Vec<String>,
) -> Result<ScriptPreview, AppError> {
    for source in &scripts {
        validate_source(source)?;
    }
    let request = WorkerRequest {
        scripts,
        config: json_value(value, 0)?,
        name: name.into(),
        id: id.clone(),
    };
    let input = serde_json::to_vec(&request).map_err(|e| error(e.to_string()))?;
    if input.len() > WIRE_LIMIT {
        return Err(error("Script input exceeds 12 MiB"));
    }
    #[cfg(not(test))]
    let response = {
        let executable = std::env::current_exe().map_err(|e| error(e.to_string()))?;
        run_process(&executable, input, Duration::from_secs(5))?
    };
    #[cfg(test)]
    let response = worker::evaluate(request);
    let value = response.result.map_err(error)?;
    let yaml = serde_yaml::to_string(&value).map_err(|e| error(e.to_string()))?;
    let preview = ScriptPreview {
        yaml,
        logs: response.logs,
    };
    if serde_json::to_vec(&preview)
        .map_err(|e| error(e.to_string()))?
        .len()
        > OUTPUT_LIMIT
    {
        return Err(error("Script preview exceeds 8 MiB"));
    }
    Ok(preview)
}

fn run_process(
    executable: &std::path::Path,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<WorkerResponse, AppError> {
    let mut child = Command::new(executable)
        .arg("--script-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| error(format!("Cannot start script worker: {e}")))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(error(e.to_string())),
            _ => {}
        }
        if started.elapsed() >= timeout || memory_exceeded(child.id()) {
            break Err(error("Script exceeded its time or memory limit"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if status.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let written = writer
        .join()
        .map_err(|_| error("Script input writer failed"))?;
    let bytes = reader
        .join()
        .map_err(|_| error("Script output reader failed"))?
        .map_err(|e| error(e.to_string()))?;
    let status = status?;
    if !status.success() {
        return Err(error("Script worker exited without a result"));
    }
    written.map_err(|e| error(e.to_string()))?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err(error("Script output exceeds 8 MiB"));
    }
    serde_json::from_slice(&bytes).map_err(|_| error("Invalid script worker response"))
}

#[cfg(target_os = "macos")]
fn memory_exceeded(pid: u32) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // proc_pid_rusage writes the versioned struct into the caller-owned buffer.
    let result = unsafe {
        libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast())
    };
    result == 0 && unsafe { info.assume_init() }.ri_resident_size > 256 * 1024 * 1024
}

#[cfg(not(target_os = "macos"))]
fn memory_exceeded(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversion_rejects_unsafe_yaml() {
        for yaml in [
            "n: 9007199254740993",
            "n: .nan",
            "1: value",
            "n: !tag value",
        ] {
            assert!(
                json_value(&serde_yaml::from_str(yaml).unwrap(), 0).is_err(),
                "{yaml}"
            );
        }
    }
    #[test]
    #[cfg(unix)]
    fn parent_kills_and_reaps_a_stalled_worker() {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("verge-script-timeout-{}", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        let result = run_process(&path, b"{}".to_vec(), Duration::from_millis(80));
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        std::fs::remove_file(path).unwrap();
    }
}
