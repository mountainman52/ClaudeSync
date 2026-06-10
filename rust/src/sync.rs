use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};

use crate::compression::{compress_content, decompress_content};
use crate::config::FileConfig;
use crate::error::{CsError, Result};
use crate::provider::{ClaudeProvider, RemoteFile};
use crate::utils::compute_md5_hash;

/// Retries an operation up to 3 times when it fails with a 403 (port of the
/// `retry_on_403` decorator).
pub fn retry_on_403<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    const MAX_RETRIES: usize = 3;
    const DELAY: Duration = Duration::from_secs(1);
    for attempt in 0..MAX_RETRIES {
        match f() {
            Err(CsError::Provider(msg))
                if msg.contains("403 Forbidden") && attempt < MAX_RETRIES - 1 =>
            {
                log::warn!(
                    "Received 403 error. Retrying in {} seconds... (Attempt {}/{})",
                    DELAY.as_secs(),
                    attempt + 1,
                    MAX_RETRIES
                );
                std::thread::sleep(DELAY);
            }
            other => return other,
        }
    }
    unreachable!()
}

fn progress_bar(len: u64, desc: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::with_template("{msg} [{bar:30}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    pb.set_message(desc.to_string());
    pb
}

/// Port of `SyncManager`.
pub struct SyncManager {
    pub active_organization_id: String,
    pub active_project_id: String,
    pub local_path: PathBuf,
    pub upload_delay: f64,
    pub two_way_sync: bool,
    pub prune_remote_files: bool,
    pub compression_algorithm: String,
}

impl SyncManager {
    pub fn new(config: &FileConfig, local_path: &Path) -> Self {
        SyncManager {
            active_organization_id: config.get_str("active_organization_id").unwrap_or_default(),
            active_project_id: config.get_str("active_project_id").unwrap_or_default(),
            local_path: local_path.to_path_buf(),
            upload_delay: config.get_f64("upload_delay", 0.5),
            two_way_sync: config.get_bool("two_way_sync", false),
            prune_remote_files: config.get_bool("prune_remote_files", false),
            compression_algorithm: config
                .get_str("compression_algorithm")
                .unwrap_or_else(|| "none".to_string()),
        }
    }

    /// Override the target project (used when syncing submodules).
    pub fn with_project(mut self, project_id: &str) -> Self {
        self.active_project_id = project_id.to_string();
        self
    }

    fn sleep_upload_delay(&self) {
        std::thread::sleep(Duration::from_secs_f64(self.upload_delay));
    }

    pub fn sync(
        &self,
        provider: &ClaudeProvider,
        local_files: &BTreeMap<String, String>,
        remote_files: &[RemoteFile],
    ) -> Result<()> {
        if self.compression_algorithm == "none" {
            self.sync_without_compression(provider, local_files, remote_files)
        } else {
            self.sync_with_compression(provider, local_files, remote_files)
        }
    }

    fn sync_without_compression(
        &self,
        provider: &ClaudeProvider,
        local_files: &BTreeMap<String, String>,
        remote_files: &[RemoteFile],
    ) -> Result<()> {
        let mut remote_files_to_delete: HashSet<String> =
            remote_files.iter().map(|rf| rf.file_name.clone()).collect();
        let mut synced_files: HashSet<String> = HashSet::new();

        let pb = progress_bar(local_files.len() as u64, "Local → Remote");
        for (local_file, local_checksum) in local_files {
            let remote_file = remote_files.iter().find(|rf| &rf.file_name == local_file);
            match remote_file {
                Some(rf) => self.update_existing_file(
                    provider,
                    local_file,
                    local_checksum,
                    rf,
                    &mut remote_files_to_delete,
                    &mut synced_files,
                )?,
                None => self.upload_new_file(provider, local_file, &mut synced_files)?,
            }
            pb.inc(1);
        }
        pb.finish();

        self.update_local_timestamps(remote_files, &synced_files)?;

        if self.two_way_sync {
            let pb = progress_bar(remote_files.len() as u64, "Local ← Remote");
            for remote_file in remote_files {
                self.sync_remote_to_local(
                    remote_file,
                    &mut remote_files_to_delete,
                    &mut synced_files,
                )?;
                pb.inc(1);
            }
            pb.finish();
        }

        self.prune_remote(provider, remote_files, &remote_files_to_delete)?;
        Ok(())
    }

    fn sync_with_compression(
        &self,
        provider: &ClaudeProvider,
        local_files: &BTreeMap<String, String>,
        remote_files: &[RemoteFile],
    ) -> Result<()> {
        let packed_content = self.pack_files(local_files)?;
        let compressed_content =
            compress_content(&packed_content, &self.compression_algorithm)?;

        let remote_file_name = format!(
            "claudesync_packed_{}.dat",
            chrono::Local::now().format("%Y%m%d%H%M%S")
        );
        retry_on_403(|| {
            log::debug!("Uploading compressed file {remote_file_name} to remote...");
            provider.upload_file(
                &self.active_organization_id,
                &self.active_project_id,
                &remote_file_name,
                &compressed_content,
            )?;
            self.sleep_upload_delay();
            Ok(())
        })?;

        if self.two_way_sync {
            let remote_compressed = retry_on_403(|| {
                log::debug!("Downloading latest compressed file from remote...");
                let remote_files = provider
                    .list_files(&self.active_organization_id, &self.active_project_id)?;
                Ok(remote_files
                    .into_iter()
                    .filter(|rf| rf.file_name.starts_with("claudesync_packed_"))
                    .max_by(|a, b| a.file_name.cmp(&b.file_name))
                    .map(|rf| rf.content))
            })?;
            if let Some(content) = remote_compressed {
                let unpacked = decompress_content(&content, &self.compression_algorithm)?;
                self.unpack_files(&unpacked)?;
            }
        }

        // Clean up previously uploaded packed files
        for remote_file in remote_files {
            if remote_file.file_name.starts_with("claudesync_packed_") {
                provider.delete_file(
                    &self.active_organization_id,
                    &self.active_project_id,
                    &remote_file.uuid,
                )?;
            }
        }
        Ok(())
    }

    pub fn pack_files(&self, local_files: &BTreeMap<String, String>) -> Result<String> {
        let mut packed = String::new();
        for file_path in local_files.keys() {
            let full_path = self.local_path.join(file_path);
            let content = fs::read_to_string(&full_path)?;
            packed.push_str(&format!("--- BEGIN FILE: {file_path} ---\n"));
            packed.push_str(&content);
            packed.push_str(&format!("\n--- END FILE: {file_path} ---\n"));
        }
        Ok(packed)
    }

    fn unpack_files(&self, packed_content: &str) -> Result<()> {
        let mut current_file: Option<String> = None;
        let mut current_content = String::new();

        for line in packed_content.lines() {
            if let Some(rest) = line.strip_prefix("--- BEGIN FILE:") {
                if let Some(file) = current_file.take() {
                    self.write_file(&file, &current_content)?;
                    current_content.clear();
                }
                current_file = Some(rest.trim().trim_end_matches(" ---").trim().to_string());
            } else if line.starts_with("--- END FILE:") {
                if let Some(file) = current_file.take() {
                    self.write_file(&file, &current_content)?;
                    current_content.clear();
                }
            } else {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }
        if let Some(file) = current_file {
            self.write_file(&file, &current_content)?;
        }
        Ok(())
    }

    fn write_file(&self, file_path: &str, content: &str) -> Result<()> {
        let full_path = self.local_path.join(file_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full_path, content)?;
        Ok(())
    }

    fn update_existing_file(
        &self,
        provider: &ClaudeProvider,
        local_file: &str,
        local_checksum: &str,
        remote_file: &RemoteFile,
        remote_files_to_delete: &mut HashSet<String>,
        synced_files: &mut HashSet<String>,
    ) -> Result<()> {
        let remote_checksum = compute_md5_hash(&remote_file.content);
        if local_checksum != remote_checksum {
            log::debug!("Updating {local_file} on remote...");
            retry_on_403(|| {
                provider.delete_file(
                    &self.active_organization_id,
                    &self.active_project_id,
                    &remote_file.uuid,
                )?;
                let content = fs::read_to_string(self.local_path.join(local_file))?;
                provider.upload_file(
                    &self.active_organization_id,
                    &self.active_project_id,
                    local_file,
                    &content,
                )?;
                Ok(())
            })?;
            self.sleep_upload_delay();
            synced_files.insert(local_file.to_string());
        }
        remote_files_to_delete.remove(local_file);
        Ok(())
    }

    fn upload_new_file(
        &self,
        provider: &ClaudeProvider,
        local_file: &str,
        synced_files: &mut HashSet<String>,
    ) -> Result<()> {
        log::debug!("Uploading new file {local_file} to remote...");
        let content = fs::read_to_string(self.local_path.join(local_file))?;
        retry_on_403(|| {
            provider.upload_file(
                &self.active_organization_id,
                &self.active_project_id,
                local_file,
                &content,
            )?;
            Ok(())
        })?;
        self.sleep_upload_delay();
        synced_files.insert(local_file.to_string());
        Ok(())
    }

    fn update_local_timestamps(
        &self,
        remote_files: &[RemoteFile],
        synced_files: &HashSet<String>,
    ) -> Result<()> {
        for remote_file in remote_files {
            if synced_files.contains(&remote_file.file_name) {
                let local_file_path = self.local_path.join(&remote_file.file_name);
                if local_file_path.exists() {
                    if let Some(ts) = parse_remote_timestamp(&remote_file.created_at) {
                        let ft = filetime::FileTime::from_unix_time(ts.timestamp(), 0);
                        filetime::set_file_times(&local_file_path, ft, ft)?;
                        log::debug!(
                            "Updated timestamp on local file {}",
                            local_file_path.display()
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn sync_remote_to_local(
        &self,
        remote_file: &RemoteFile,
        remote_files_to_delete: &mut HashSet<String>,
        synced_files: &mut HashSet<String>,
    ) -> Result<()> {
        let local_file_path = self.local_path.join(&remote_file.file_name);
        if local_file_path.exists() {
            // Update only if remote is newer than the local mtime
            let local_mtime: DateTime<Utc> = fs::metadata(&local_file_path)?.modified()?.into();
            let remote_mtime = parse_remote_timestamp(&remote_file.created_at)
                .ok_or_else(|| CsError::Other("Invalid remote timestamp".into()))?;
            if remote_mtime > local_mtime {
                log::debug!("Updating local file {} from remote...", remote_file.file_name);
                fs::write(&local_file_path, &remote_file.content)?;
                synced_files.insert(remote_file.file_name.clone());
                remote_files_to_delete.remove(&remote_file.file_name);
            }
        } else {
            log::debug!("Creating new local file {} from remote...", remote_file.file_name);
            if let Some(parent) = local_file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&local_file_path, &remote_file.content)?;
            synced_files.insert(remote_file.file_name.clone());
            remote_files_to_delete.remove(&remote_file.file_name);
        }
        Ok(())
    }

    fn prune_remote(
        &self,
        provider: &ClaudeProvider,
        remote_files: &[RemoteFile],
        remote_files_to_delete: &HashSet<String>,
    ) -> Result<()> {
        if !self.prune_remote_files {
            log::info!("Remote pruning is not enabled.");
            return Ok(());
        }
        for file_to_delete in remote_files_to_delete {
            log::debug!("Deleting {file_to_delete} from remote...");
            if let Some(remote_file) = remote_files
                .iter()
                .find(|rf| &rf.file_name == file_to_delete)
            {
                retry_on_403(|| {
                    provider.delete_file(
                        &self.active_organization_id,
                        &self.active_project_id,
                        &remote_file.uuid,
                    )?;
                    Ok(())
                })?;
                self.sleep_upload_delay();
            }
        }
        Ok(())
    }

    /// Packs and compresses local files without uploading (embedding output).
    pub fn embedding(&self, local_files: &BTreeMap<String, String>) -> Result<String> {
        let packed = self.pack_files(local_files)?;
        compress_content(&packed, &self.compression_algorithm)
    }
}

/// Parses remote ISO timestamps like `2024-01-01T00:00:00Z`.
fn parse_remote_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let normalized = s.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
