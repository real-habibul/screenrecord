use crate::{cancelled, check_cancel, local_assets::*, CANCELLED};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    pub ready: bool,
    pub runtime_ready: bool,
    pub model_ready: bool,
    pub runtime_version: String,
    pub model_name: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Active {
    version: u32,
    directory: String,
}

pub(crate) struct Lease {
    pub directory: PathBuf,
    _files: Vec<fs::File>,
}

fn base_status() -> LocalStatus {
    LocalStatus {
        ready: false,
        runtime_ready: false,
        model_ready: false,
        runtime_version: "1.9.5 (b5454)".into(),
        model_name: "Whisper base multilingual".into(),
        download_bytes: ARCHIVE.size + MODEL.size,
        installed_bytes: 0,
        error: None,
    }
}

fn ordinary(path: &Path, directory: bool) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        "Local subtitle files are missing. Install local subtitles again.".to_owned()
    })?;
    use std::os::windows::fs::MetadataExt;
    if metadata.file_type().is_symlink()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err("Local subtitle storage contains an unsupported link or file type.".into());
    }
    Ok(metadata)
}

fn installation(root: &Path) -> Result<Option<PathBuf>, String> {
    if !root.is_absolute() {
        return Err("Local subtitle storage must be an absolute application directory.".into());
    }
    if !root
        .try_exists()
        .map_err(|_| "Cannot inspect local subtitle storage.".to_owned())?
    {
        return Ok(None);
    }
    ordinary(root, true)?;
    let pointer = root.join("active.json");
    if !pointer
        .try_exists()
        .map_err(|_| "Cannot inspect local subtitle setup.".to_owned())?
    {
        return Ok(None);
    }
    if ordinary(&pointer, false)?.len() > 4096 {
        return Err("Local subtitle setup metadata is invalid. Install again.".into());
    }
    let mut bytes = Vec::new();
    open_locked(&pointer)?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read local subtitle setup.".to_owned())?;
    if bytes.len() > 4096 {
        return Err("Local subtitle setup metadata is too large.".into());
    }
    let active: Active = serde_json::from_slice(&bytes)
        .map_err(|_| "Local subtitle setup metadata is damaged. Install again.".to_owned())?;
    let id = active
        .directory
        .strip_prefix("installation-")
        .and_then(|id| uuid::Uuid::parse_str(id).ok());
    if active.version != 1
        || id.is_none()
        || active.directory != format!("installation-{}", id.unwrap())
    {
        return Err("Local subtitle setup metadata is invalid. Install again.".into());
    }
    let directory = root.join(active.directory);
    ordinary(&directory, true)?;
    Ok(Some(directory))
}

fn open_locked(path: &Path) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| "Cannot lock local subtitle files for verification.".into())
}

fn verify_file(path: &Path, asset: &Asset, cancel: &AtomicBool) -> Result<fs::File, String> {
    if ordinary(path, false)?.len() != asset.size {
        return Err("Local subtitle files failed integrity verification. Install again.".into());
    }
    let mut file = open_locked(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0;
    loop {
        check_cancel(cancel)?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Cannot verify local subtitle files.".to_owned())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > asset.size {
            return Err("Local subtitle file size changed during verification.".into());
        }
        hash.update(&buffer[..count]);
    }
    if total != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err("Local subtitle files failed SHA-256 verification. Install again.".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "Cannot reopen verified local subtitle files.".to_owned())?;
    Ok(file)
}

fn verify_runtime(directory: &Path, cancel: &AtomicBool) -> Result<Vec<fs::File>, String> {
    // GGML discovers backends beside the executable. Reject additional executable files.
    for entry in fs::read_dir(directory)
        .map_err(|_| "Cannot inspect the local subtitle runtime.".to_owned())?
    {
        let entry = entry.map_err(|_| "Cannot inspect the local subtitle runtime.".to_owned())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !RUNTIME.iter().any(|asset| asset.name == name)
            && name != MODEL.name
            && name != "LICENSE.txt"
        {
            return Err(
                "Unexpected files were found in the local subtitle runtime. Install again.".into(),
            );
        }
    }
    RUNTIME
        .iter()
        .map(|asset| verify_file(&directory.join(asset.name), asset, cancel))
        .collect()
}

pub(crate) fn verified(root: &Path, cancel: &AtomicBool) -> Result<Lease, String> {
    check_cancel(cancel)?;
    let directory = installation(root)?.ok_or("Install local subtitles before generating them.")?;
    let mut files = verify_runtime(&directory, cancel)?;
    files.push(verify_file(&directory.join(MODEL.name), &MODEL, cancel)?);
    Ok(Lease {
        directory,
        _files: files,
    })
}

pub fn status(root: &Path) -> Result<LocalStatus, String> {
    status_with_cancel(root, &AtomicBool::new(false))
}

fn status_with_cancel(root: &Path, cancel: &AtomicBool) -> Result<LocalStatus, String> {
    let mut status = base_status();
    let directory = match installation(root) {
        Ok(Some(path)) => path,
        Ok(None) => return Ok(status),
        Err(error) => {
            status.error = Some(error);
            return Ok(status);
        }
    };
    let runtime = verify_runtime(&directory, cancel);
    let model = verify_file(&directory.join(MODEL.name), &MODEL, cancel);
    status.runtime_ready = runtime.is_ok();
    status.model_ready = model.is_ok();
    status.installed_bytes = u64::from(status.runtime_ready)
        * RUNTIME.iter().map(|asset| asset.size).sum::<u64>()
        + u64::from(status.model_ready) * MODEL.size;
    status.error = runtime.err().or_else(|| model.err());
    status.ready = status.runtime_ready && status.model_ready;
    Ok(status)
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(1200))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let host = url.host_str().unwrap_or("");
            let trusted = [
                "github.com",
                "release-assets.githubusercontent.com",
                "huggingface.co",
                "cdn-lfs.huggingface.co",
                "cdn-lfs-us-1.hf.co",
                "cas-bridge.xethub.hf.co",
                "us.aws.cdn.hf.co",
            ]
            .contains(&host);
            if attempt.previous().len() > 5
                || url.scheme() != "https"
                || !trusted
                || !url.username().is_empty()
                || url.password().is_some()
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Cannot initialize secure local subtitle downloads.".into())
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    asset: &Asset,
    output: &Path,
    cancel: &AtomicBool,
    progress: &mut impl Fn(f64),
    offset: u64,
) -> Result<(), String> {
    let operation = async move {
        let mut response = client.get(url).send().await.map_err(|_| {
            "Could not download local subtitles. Check the connection and retry.".to_owned()
        })?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size != asset.size)
        {
            return Err("Local subtitle download returned an unexpected response or size.".into());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .await
            .map_err(|_| "Cannot create the local subtitle download.".to_owned())?;
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut last_progress = std::time::Instant::now();
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|_| "Local subtitle download was interrupted. Retry setup.".to_owned())?
        {
            count += bytes.len() as u64;
            if count > asset.size {
                return Err("Local subtitle download exceeded its verified size.".into());
            }
            hash.update(&bytes);
            file.write_all(&bytes).await.map_err(|_| {
                "Cannot save local subtitles. Check available disk space.".to_owned()
            })?;
            if last_progress.elapsed() >= Duration::from_millis(150) || count == asset.size {
                progress(0.95 * (offset + count) as f64 / (ARCHIVE.size + MODEL.size) as f64);
                last_progress = std::time::Instant::now();
            }
        }
        if count != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
            return Err("Local subtitle download failed SHA-256 verification. Retry setup.".into());
        }
        file.sync_all()
            .await
            .map_err(|_| "Cannot flush the verified local subtitle download.".to_owned())?;
        Ok(())
    };
    tokio::select! { biased; _ = cancelled(cancel) => Err(CANCELLED.into()), result = operation => result }
}

fn unpack(archive: &Path, output: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let file = verify_file(archive, &ARCHIVE, cancel)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "The verified local subtitle archive could not be opened.".to_owned())?;
    for asset in RUNTIME {
        check_cancel(cancel)?;
        let mut source = archive
            .by_name(&format!("Release/{}", asset.name))
            .map_err(|_| "The local subtitle archive is missing a required file.".to_owned())?;
        if source.size() != asset.size || source.is_symlink() {
            return Err("The local subtitle archive contains an invalid entry.".into());
        }
        let mut target = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output.join(asset.name))
            .map_err(|_| "Cannot extract local subtitle runtime files.".to_owned())?;
        let mut buffer = [0; 64 * 1024];
        let mut count = 0;
        loop {
            check_cancel(cancel)?;
            let read = source
                .read(&mut buffer)
                .map_err(|_| "Local subtitle archive extraction failed.".to_owned())?;
            if read == 0 {
                break;
            }
            count += read as u64;
            if count > asset.size {
                return Err("Local subtitle archive extraction exceeded its limit.".into());
            }
            target
                .write_all(&buffer[..read])
                .map_err(|_| "Cannot save local subtitle runtime files.".to_owned())?;
        }
        target
            .sync_all()
            .map_err(|_| "Cannot flush local subtitle runtime files.".to_owned())?;
        drop(target);
        verify_file(&output.join(asset.name), asset, cancel)?;
    }
    fs::write(output.join("LICENSE.txt"), LICENSE)
        .map_err(|_| "Cannot save local subtitle license notices.".to_owned())?;
    Ok(())
}

static INSTALLING: AtomicBool = AtomicBool::new(false);
struct Installing;
impl Drop for Installing {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Release);
    }
}

pub async fn install(
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    mut progress: impl Fn(f64) + Send + 'static,
) -> Result<LocalStatus, String> {
    check_cancel(&cancel)?;
    INSTALLING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local subtitle setup is already running.".to_owned())?;
    let _installing = Installing;
    if !root.is_absolute() {
        return Err("Local subtitle storage must be an absolute application directory.".into());
    }
    let check_root = root.clone();
    let token = Arc::clone(&cancel);
    let existing = tokio::task::spawn_blocking(move || status_with_cancel(&check_root, &token))
        .await
        .map_err(|_| "Cannot verify local subtitle setup.".to_owned())??;
    check_cancel(&cancel)?;
    if existing.ready {
        progress(1.0);
        return Ok(existing);
    }
    fs::create_dir_all(&root).map_err(|_| "Cannot create local subtitle storage.".to_owned())?;
    ordinary(&root, true)?;
    let stage = tempfile::Builder::new()
        .prefix(".setup-")
        .tempdir_in(&root)
        .map_err(|_| "Cannot prepare local subtitle setup.".to_owned())?;
    let runtime = stage.path().join("runtime");
    fs::create_dir(&runtime).map_err(|_| "Cannot prepare local subtitle runtime.".to_owned())?;
    let client = download_client()?;
    progress(0.0);
    let archive = stage.path().join(ARCHIVE.name);
    download(
        &client,
        RUNTIME_URL,
        &ARCHIVE,
        &archive,
        &cancel,
        &mut progress,
        0,
    )
    .await?;
    let output = runtime.clone();
    let token = Arc::clone(&cancel);
    tokio::task::spawn_blocking(move || unpack(&archive, &output, &token))
        .await
        .map_err(|_| "Local subtitle extraction stopped unexpectedly.".to_owned())??;
    download(
        &client,
        MODEL_URL,
        &MODEL,
        &runtime.join(MODEL.name),
        &cancel,
        &mut progress,
        ARCHIVE.size,
    )
    .await?;
    check_cancel(&cancel)?;
    let mut probe = tokio::process::Command::new(runtime.join("whisper-cli.exe"));
    let system =
        std::env::var_os("SystemRoot").ok_or("Windows system directory is unavailable.")?;
    probe
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .current_dir(&runtime)
        .env_clear()
        .env("SystemRoot", &system)
        .env("WINDIR", &system)
        .env("PATH", Path::new(&system).join("System32"))
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::extract::run(&mut probe, &cancel, Duration::from_secs(10)).await.map_err(|_| {
        if cancel.load(Ordering::Acquire) { CANCELLED.to_owned() } else { "The verified local subtitle runtime could not start. Check Microsoft Visual C++ x64 runtime availability.".to_owned() }
    })?;
    activate(&root, &runtime, &cancel)?;
    stage.close().map_err(|_| {
        "Local subtitles installed, but temporary download cleanup failed.".to_owned()
    })?;
    let status = tokio::task::spawn_blocking(move || status(&root))
        .await
        .map_err(|_| "Cannot verify the installed local subtitles.".to_owned())??;
    if !status.ready {
        return Err(status
            .error
            .unwrap_or_else(|| "Local subtitle setup verification failed.".into()));
    }
    progress(1.0);
    Ok(status)
}

fn activate(root: &Path, runtime: &Path, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    let directory = format!("installation-{}", uuid::Uuid::new_v4());
    let destination = root.join(&directory);
    fs::rename(runtime, &destination)
        .map_err(|_| "Cannot publish verified local subtitle files.".to_owned())?;
    let publish = (|| {
        check_cancel(cancel)?;
        let mut temporary = tempfile::NamedTempFile::new_in(root)
            .map_err(|_| "Cannot prepare local subtitle setup metadata.".to_owned())?;
        serde_json::to_writer(
            &mut temporary,
            &Active {
                version: 1,
                directory,
            },
        )
        .map_err(|_| "Cannot write local subtitle setup metadata.".to_owned())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| "Cannot flush local subtitle setup metadata.".to_owned())?;
        temporary.persist(root.join("active.json")).map_err(|_| {
            "Cannot activate local subtitles. Existing setup is unchanged.".to_owned()
        })?;
        Ok::<(), String>(())
    })();
    if publish.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    publish
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    const SMALL: Asset = Asset {
        name: "test.bin",
        size: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };

    async fn server(
        response: &'static [u8],
        delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/asset", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() < 4096);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            assert!(
                request.starts_with("get /asset ")
                    && !request.contains("authorization")
                    && !request.contains("content-length")
            );
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(response).await;
        });
        (address, task)
    }

    #[tokio::test]
    async fn downloads_require_exact_length_and_hash_without_sending_credentials_or_audio() {
        for (response, success) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".as_slice(),
                true,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabd".as_slice(),
                false,
            ),
            (
                b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcd".as_slice(),
                false,
            ),
            (
                b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
                false,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (url, task) = server(response, Duration::ZERO).await;
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let result = download(
                &client,
                &url,
                &SMALL,
                &root.path().join(SMALL.name),
                &AtomicBool::new(false),
                &mut |_| {},
                0,
            )
            .await;
            assert_eq!(result.is_ok(), success);
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_drops_download_and_leaves_existing_setup_unchanged() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("active.json"), b"original setup").unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let stage_path = stage.path().to_owned();
        let (url, task) = server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc",
            Duration::from_millis(200),
        )
        .await;
        let cancel = AtomicBool::new(false);
        let client = reqwest::Client::new();
        let target = stage.path().join(SMALL.name);
        let mut progress = |_| {};
        let operation = download(&client, &url, &SMALL, &target, &cancel, &mut progress, 0);
        let trigger = async {
            tokio::time::sleep(Duration::from_millis(40)).await;
            cancel.store(true, Ordering::Release);
        };
        let (result, _) = tokio::join!(operation, trigger);
        assert_eq!(result.unwrap_err(), CANCELLED);
        stage.close().unwrap();
        assert!(!stage_path.exists());
        assert_eq!(
            std::fs::read(root.path().join("active.json")).unwrap(),
            b"original setup"
        );
        task.await.unwrap();
    }

    #[test]
    fn verified_file_lease_prevents_replacement_and_detects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(SMALL.name);
        std::fs::write(&path, b"abc").unwrap();
        let lease = verify_file(&path, &SMALL, &AtomicBool::new(false)).unwrap();
        assert!(std::fs::write(&path, b"abd").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(lease);
        std::fs::write(&path, b"abd").unwrap();
        assert!(verify_file(&path, &SMALL, &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn unexpected_backend_is_rejected_before_any_executable_can_run() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ggml-foreign.dll"), b"untrusted").unwrap();
        assert!(verify_runtime(root.path(), &AtomicBool::new(false))
            .unwrap_err()
            .contains("Unexpected files"));
    }

    #[test]
    fn activation_is_atomic_and_failed_publication_removes_only_its_new_directory() {
        let root = tempfile::tempdir().unwrap();
        let pointer = root.path().join("active.json");
        fs::write(&pointer, b"previous installation").unwrap();
        let stage = root.path().join("prepared");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("complete.bin"), b"complete").unwrap();
        assert_eq!(
            activate(root.path(), &stage, &AtomicBool::new(true)).unwrap_err(),
            CANCELLED
        );
        assert_eq!(fs::read(&pointer).unwrap(), b"previous installation");
        assert!(stage.exists());
        let lock = open_locked(&pointer).unwrap();
        assert!(activate(root.path(), &stage, &AtomicBool::new(false))
            .unwrap_err()
            .contains("Existing setup is unchanged"));
        assert_eq!(fs::read(&pointer).unwrap(), b"previous installation");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        drop(lock);
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("complete.bin"), b"complete").unwrap();
        activate(root.path(), &stage, &AtomicBool::new(false)).unwrap();
        let active: Active = serde_json::from_slice(&fs::read(&pointer).unwrap()).unwrap();
        assert_eq!(
            fs::read(root.path().join(active.directory).join("complete.bin")).unwrap(),
            b"complete"
        );
        assert!(!stage.exists());
    }
}
