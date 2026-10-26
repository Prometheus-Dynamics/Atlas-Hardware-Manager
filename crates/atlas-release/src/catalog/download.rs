//! Downloading a remote release into the cache: resumed and retried, and
//! checked against its SHA-256 before it is kept.

use super::*;

impl ReleaseCatalog {
    /// Downloads a remote release into the cache, checking its SHA-256
    /// before it is kept.
    pub async fn download(
        &self,
        id: &str,
        progress: impl Fn(DownloadProgress) + Send,
    ) -> Result<ReleaseEntry, ReleaseError> {
        let mut entry = self
            .entry(id)
            .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
        let ReleaseOrigin::Remote { url, .. } = entry.origin.clone() else {
            return Ok(entry);
        };
        if entry.path.as_ref().is_some_and(|path| path.exists()) {
            return Ok(entry);
        }

        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| ReleaseError::io(&self.cache_dir, error))?;
        let target = self.cache_dir.join(cache_name(&entry));
        let partial = target.with_extension("partial");
        let download_error = |message: String| ReleaseError::Download {
            url: url.clone(),
            message,
        };

        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|error| ReleaseError::io(&partial, error))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let mut last = String::new();
        let mut done = false;
        // A busy release host answers 5xx now and then, and a big image's
        // connection can drop: try again, carrying on from what arrived.
        'attempts: for attempt in 1..=DOWNLOAD_ATTEMPTS {
            if attempt > 1 {
                tokio::time::sleep(Duration::from_secs(1 << (attempt - 2))).await;
            }
            let mut request = self.http.get(&url);
            if downloaded > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={downloaded}-"));
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    last = error.to_string();
                    continue;
                }
            };
            let status = response.status();
            if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                last = format!("the server answered {status}");
                continue;
            }
            if !status.is_success() {
                return Err(download_error(format!("the server answered {status}")));
            }
            // The server sent the whole file again: start over.
            if downloaded > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
                file.set_len(0)
                    .await
                    .map_err(|error| ReleaseError::io(&partial, error))?;
                file.seek(std::io::SeekFrom::Start(0))
                    .await
                    .map_err(|error| ReleaseError::io(&partial, error))?;
                hasher = Sha256::new();
                downloaded = 0;
            }
            let total = response.content_length().map(|len| len + downloaded);
            let mut stream = response.bytes_stream();
            loop {
                let chunk = match tokio::time::timeout(DOWNLOAD_STALL, stream.next()).await {
                    Err(_) => {
                        last = format!("nothing arrived for {} s", DOWNLOAD_STALL.as_secs());
                        continue 'attempts;
                    }
                    Ok(None) => break,
                    Ok(Some(Err(error))) => {
                        last = error.to_string();
                        continue 'attempts;
                    }
                    Ok(Some(Ok(chunk))) => chunk,
                };
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|error| ReleaseError::io(&partial, error))?;
                downloaded += chunk.len() as u64;
                progress(DownloadProgress { downloaded, total });
            }
            done = true;
            break;
        }
        if !done {
            drop(file);
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(download_error(format!(
                "{last} (tried {DOWNLOAD_ATTEMPTS} times)"
            )));
        }
        file.sync_all()
            .await
            .map_err(|error| ReleaseError::io(&partial, error))?;
        drop(file);

        let actual = to_hex(&hasher.finalize());
        if !entry.sha256.is_empty() && actual != entry.sha256 {
            // Kept aside so the user can still choose to use it.
            let _ = tokio::fs::rename(&partial, unverified_path(&target)).await;
            return Err(ReleaseError::HashMismatch {
                path: target,
                expected: entry.sha256,
                actual,
            });
        }
        tokio::fs::rename(&partial, &target)
            .await
            .map_err(|error| ReleaseError::io(&target, error))?;
        entry.path = Some(target);
        self.upsert(entry.clone())?;
        Ok(entry)
    }
}
