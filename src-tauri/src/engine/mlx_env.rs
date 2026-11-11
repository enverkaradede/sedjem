//! MLX environment management: locating local MLX Whisper models in the
//! Hugging Face cache and provisioning Sedjem's own Python venv with a
//! pinned `mlx-whisper`, so Sedjem never depends on another project's venv.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Pinned for reproducibility; same release Backtalk runs on this machine.
pub const MLX_WHISPER_VERSION: &str = "0.4.3";
const PYTHON_VERSION: &str = "3.11";
const VENV_DIR: &str = "mlx-env";
const PYTHON_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const PYTHON_CHECK_POLL: Duration = Duration::from_millis(100);

/// GUI apps launched from Finder do not inherit the shell PATH, so look in
/// the usual install locations explicitly.
const UV_CANDIDATES: &[&str] = &["/opt/homebrew/bin/uv", "/usr/local/bin/uv"];
const UV_HOME_CANDIDATES: &[&str] = &[".local/bin/uv", ".cargo/bin/uv"];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MlxModel {
    /// Hub id, e.g. `mlx-community/whisper-large-v3-turbo`.
    pub repo: String,
    /// Snapshot directory holding `config.json` and the weights.
    pub path: String,
    pub size_bytes: u64,
}

pub fn default_venv_python(app_data: &Path) -> PathBuf {
    app_data.join(VENV_DIR).join("bin").join("python")
}

/// Candidate Hugging Face hub cache directories, most specific first.
fn hub_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(cache) = std::env::var_os("HF_HUB_CACHE") {
        dirs.push(PathBuf::from(cache));
    }
    if let Some(hf_home) = std::env::var_os("HF_HOME") {
        dirs.push(PathBuf::from(hf_home).join("hub"));
    }
    dirs.push(home.join(".cache").join("huggingface").join("hub"));
    dirs.dedup();
    dirs
}

fn is_mlx_whisper_snapshot(dir: &Path) -> bool {
    dir.join("config.json").is_file()
        && (dir.join("weights.safetensors").exists() || dir.join("weights.npz").exists())
}

/// Size of the weights a snapshot points at (snapshots are symlinks to blobs).
fn snapshot_size(dir: &Path) -> u64 {
    ["weights.safetensors", "weights.npz"]
        .iter()
        .filter_map(|name| fs::metadata(dir.join(name)).ok())
        .map(|meta| meta.len())
        .sum()
}

/// Picks the snapshot `refs/main` points to, else the first complete one.
fn pick_snapshot(model_dir: &Path) -> Option<PathBuf> {
    let snapshots = model_dir.join("snapshots");
    if let Ok(main_ref) = fs::read_to_string(model_dir.join("refs").join("main")) {
        let candidate = snapshots.join(main_ref.trim());
        if is_mlx_whisper_snapshot(&candidate) {
            return Some(candidate);
        }
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(&snapshots)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    entries.into_iter().find(|p| is_mlx_whisper_snapshot(p))
}

/// Finds MLX Whisper models already present in the Hugging Face cache.
pub fn discover_models(home: &Path) -> Vec<MlxModel> {
    let mut found = Vec::new();
    for hub in hub_dirs(home) {
        let Ok(entries) = fs::read_dir(&hub) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(repo) = name
                .strip_prefix("models--")
                .map(|r| r.replacen("--", "/", 1))
            else {
                continue;
            };
            if !repo.to_lowercase().contains("whisper") {
                continue;
            }
            if let Some(snapshot) = pick_snapshot(&entry.path()) {
                let path = snapshot.to_string_lossy().into_owned();
                if found.iter().any(|m: &MlxModel| m.path == path) {
                    continue;
                }
                found.push(MlxModel {
                    repo,
                    size_bytes: snapshot_size(&snapshot),
                    path,
                });
            }
        }
    }
    found.sort_by(|a, b| a.repo.cmp(&b.repo));
    found
}

/// Picks the best default: large-v3-turbo if present, else the largest.
pub fn preferred_model(models: &[MlxModel]) -> Option<&MlxModel> {
    models
        .iter()
        .find(|m| m.repo.ends_with("whisper-large-v3-turbo"))
        .or_else(|| models.iter().max_by_key(|m| m.size_bytes))
}

pub fn find_uv(home: &Path) -> Option<PathBuf> {
    UV_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .chain(UV_HOME_CANDIDATES.iter().map(|rel| home.join(rel)))
        .find(|p| p.is_file())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonCheck {
    pub ok: bool,
    pub mlx_whisper_version: Option<String>,
    pub message: String,
}

/// Verifies an interpreter can import `mlx_whisper`. Bounded by a timeout.
pub fn check_python(python: &Path) -> PythonCheck {
    if !python.is_file() {
        return PythonCheck {
            ok: false,
            mlx_whisper_version: None,
            message: "Python bulunamadı.".into(),
        };
    }
    let child = Command::new(python)
        .args([
            "-c",
            "import importlib.metadata as m, mlx_whisper; print(m.version('mlx-whisper'))",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            return PythonCheck {
                ok: false,
                mlx_whisper_version: None,
                message: err.to_string(),
            }
        }
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > PYTHON_CHECK_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return PythonCheck {
                    ok: false,
                    mlx_whisper_version: None,
                    message: "Python zaman aşımı.".into(),
                };
            }
            // A blocking sleep between polls, not a spin: the thread is parked.
            Ok(None) => thread::sleep(PYTHON_CHECK_POLL),
            Err(err) => {
                return PythonCheck {
                    ok: false,
                    mlx_whisper_version: None,
                    message: err.to_string(),
                }
            }
        }
    }
    match child.wait_with_output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            PythonCheck {
                ok: true,
                message: format!("mlx-whisper {version} hazır."),
                mlx_whisper_version: Some(version),
            }
        }
        Ok(_) => PythonCheck {
            ok: false,
            mlx_whisper_version: None,
            message: "Bu Python'da mlx-whisper kurulu değil.".into(),
        },
        Err(err) => PythonCheck {
            ok: false,
            mlx_whisper_version: None,
            message: err.to_string(),
        },
    }
}

pub type ProgressFn<'a> = &'a (dyn Fn(&str) + Sync);

/// Runs one setup command, streaming stdout and stderr lines to `progress`
/// as they arrive (uv reports its progress on stderr).
fn run_logged(mut command: Command, progress: ProgressFn<'_>) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let last_error = thread::scope(|scope| {
        scope.spawn(|| {
            if let Some(out) = stdout {
                BufReader::new(out)
                    .lines()
                    .map_while(Result::ok)
                    .for_each(|line| progress(&line));
            }
        });
        let errors = scope.spawn(|| {
            let mut last = None;
            if let Some(err) = stderr {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    progress(&line);
                    last = Some(line);
                }
            }
            last
        });
        errors.join().unwrap_or(None)
    });
    let status = child.wait().map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| format!("komut başarısız: {status}")))
    }
}

/// Creates (or repairs) Sedjem's venv and installs the pinned mlx-whisper.
pub fn provision_env(
    app_data: &Path,
    home: &Path,
    progress: ProgressFn<'_>,
) -> Result<PathBuf, String> {
    let uv = find_uv(home)
        .ok_or_else(|| "uv bulunamadı. `brew install uv` ile kurabilirsiniz.".to_owned())?;
    let venv = app_data.join(VENV_DIR);
    let python = default_venv_python(app_data);
    fs::create_dir_all(app_data).map_err(|err| err.to_string())?;

    if !python.is_file() {
        progress("Python ortamı oluşturuluyor…");
        let mut create = Command::new(&uv);
        create
            .arg("venv")
            .arg("--python")
            .arg(PYTHON_VERSION)
            .arg(&venv);
        run_logged(create, progress)?;
    }
    progress(&format!("mlx-whisper {MLX_WHISPER_VERSION} kuruluyor…"));
    let mut install = Command::new(&uv);
    install
        .arg("pip")
        .arg("install")
        .arg("--python")
        .arg(&python)
        .arg(format!("mlx-whisper=={MLX_WHISPER_VERSION}"));
    run_logged(install, progress)?;

    let check = check_python(&python);
    if check.ok {
        progress(&check.message);
        Ok(python)
    } else {
        Err(check.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(hub: &Path, repo_dir: &str, rev: &str, weights: &str) -> PathBuf {
        let snapshot = hub.join(repo_dir).join("snapshots").join(rev);
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("config.json"), "{}").unwrap();
        fs::write(snapshot.join(weights), vec![0u8; 16]).unwrap();
        fs::create_dir_all(hub.join(repo_dir).join("refs")).unwrap();
        fs::write(hub.join(repo_dir).join("refs").join("main"), rev).unwrap();
        snapshot
    }

    #[test]
    fn discovers_mlx_whisper_snapshots_and_skips_others() {
        let home = tempfile::tempdir().unwrap();
        let hub = home.path().join(".cache/huggingface/hub");
        make_snapshot(
            &hub,
            "models--mlx-community--whisper-large-v3-turbo",
            "abc",
            "weights.safetensors",
        );
        make_snapshot(
            &hub,
            "models--mlx-community--whisper-large-v3-mlx",
            "def",
            "weights.npz",
        );
        // CTranslate2 model: no MLX weights, must be skipped.
        let ct2 = hub.join("models--Systran--faster-whisper-small/snapshots/x");
        fs::create_dir_all(&ct2).unwrap();
        fs::write(ct2.join("model.bin"), "x").unwrap();
        // Unrelated model.
        make_snapshot(&hub, "models--org--llama", "zzz", "weights.safetensors");

        let models = discover_models(home.path());
        let repos: Vec<&str> = models.iter().map(|m| m.repo.as_str()).collect();
        assert_eq!(
            repos,
            vec![
                "mlx-community/whisper-large-v3-mlx",
                "mlx-community/whisper-large-v3-turbo"
            ]
        );
        assert_eq!(
            preferred_model(&models).unwrap().repo,
            "mlx-community/whisper-large-v3-turbo"
        );
        assert_eq!(models[0].size_bytes, 16);
    }

    #[test]
    fn missing_python_fails_check() {
        let check = check_python(Path::new("/nonexistent/python"));
        assert!(!check.ok);
    }
}
