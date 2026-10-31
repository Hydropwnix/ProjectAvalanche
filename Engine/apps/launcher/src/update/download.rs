use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use sha2::{Digest, Sha256};
use ureq::ResponseExt;

use super::{Progress, Release, github};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: u32 = 5;
const CHUNK: usize = 64 * 1024;

pub fn fetch(
    release: &Release,
    dir: &Path,
    progress: &Arc<Mutex<Progress>>,
    ctx: &egui::Context,
) -> Result<PathBuf, String> {
    let agent = github::agent(DOWNLOAD_TIMEOUT, CONNECT_TIMEOUT, MAX_REDIRECTS)?;
    let mut response = agent
        .get(&release.asset.url)
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(|e| format!("the download could not start: {e}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("the download answered HTTP {status}"));
    }
    github::check_final_url(response.get_uri())?;
    if let Some(len) =
        response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok())
        && len != release.asset.size
    {
        return Err(format!("the download is {len} bytes, the release says {}", release.asset.size));
    }
    let path = dir.join("download.tar.gz");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let mut reader = response.body_mut().as_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut last = Instant::now();
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or("the download is larger than any release")?;
        if total > release.asset.size || total > github::MAX_ARCHIVE {
            return Err(format!("the download is larger than the release says ({} bytes)", release.asset.size));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("cannot write the download: {e}"))?;
        if last.elapsed().as_millis() >= 100 {
            let (done, all) = (total as f64 / 1e6, release.asset.size as f64 / 1e6);
            set(progress, total as f32 / release.asset.size as f32, format!("{done:.1} of {all:.1} MB"));
            ctx.request_repaint();
            last = Instant::now();
        }
    }
    file.sync_all().map_err(|e| format!("cannot finish the download: {e}"))?;
    drop(file);
    if total != release.asset.size {
        return Err(format!("the download is {total} bytes, the release says {}", release.asset.size));
    }
    let got: [u8; 32] = hasher.finalize().into();
    if got != release.asset.sha256 {
        return Err("the download's SHA-256 does not match the release's; nothing was installed".into());
    }
    set(progress, 1.0, "Downloaded and verified.".into());
    ctx.request_repaint();
    Ok(path)
}

fn set(progress: &Arc<Mutex<Progress>>, fraction: f32, detail: String) {
    if let Ok(mut p) = progress.lock() {
        p.fraction = fraction.clamp(0.0, 1.0);
        p.detail = detail;
    }
}
