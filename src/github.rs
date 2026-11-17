use crate::sources::USER_AGENT;
use crate::ui;
use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub enum Download {
    Installed(PathBuf),
    AlreadyPresent(PathBuf),
}

pub fn download_symbol(url: &str, dest_dir: &Path, symbol_path: &str) -> Result<Download> {
    let filename = Path::new(symbol_path)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine a file name from {}", symbol_path))?;
    let dest_file = dest_dir.join(filename);

    if dest_file.exists() {
        return Ok(Download::AlreadyPresent(dest_file));
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()?;
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("Download failed for {}", url))?;

    if !resp.status().is_success() {
        return Err(ui::user_error(
            format!("Download failed with HTTP {}", resp.status()),
            Some(url),
        ));
    }

    let pb = ui::download_bar(resp.content_length());

    // Write to a .part file and rename on success so a failed download never
    // leaves a truncated symbol behind.
    let tmp = dest_dir.join(format!("{}.part", filename.to_string_lossy()));
    let written = (|| -> Result<()> {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("Cannot create {}", tmp.display()))?;
        let mut buf = [0u8; 16384];
        loop {
            let n = resp
                .read(&mut buf)
                .context("Connection lost during download")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .with_context(|| format!("Cannot write {}", tmp.display()))?;
            pb.inc(n as u64);
        }
        Ok(())
    })();
    pb.finish_and_clear();

    if let Err(err) = written {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    fs::rename(&tmp, &dest_file)
        .with_context(|| format!("Cannot move {} to {}", tmp.display(), dest_file.display()))?;

    Ok(Download::Installed(dest_file))
}
