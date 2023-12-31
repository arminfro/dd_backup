use std::{fs, path::Path};

use crate::run::{config::BackupConfig, utils::convert_to_byte_size};

use super::{
    command_output::command_output,
    lsblk::{BlockDevice, Lsblk},
};

/// Represents a filesystem associated with a block device.
#[derive(Debug)]
pub struct Filesystem {
    /// The underlying block device information.
    pub blockdevice: BlockDevice,
    /// The path to the device.
    pub device_path: String,
    /// The mount path for the filesystem.
    pub mountpath: String,
    // The available size of the block device
    pub fsavail: Option<u64>,
    pub fsck_command: String,
    pub skip_fsck: bool,
}

impl Filesystem {
    /// Creates a new `Filesystem` instance for the specified UUID, using the provided `Lsblk` instance.
    ///
    /// It returns `Ok(Some(Filesystem))` if the UUID is unique and associated with a block device,
    /// `Ok(None)` if the UUID is not found in the available filesystems,
    /// or an error message if the UUID is not unique.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the filesystem.
    /// * `available_filesystems` - The list of available block devices to search for a matching UUID.
    /// * `mountpath` - The optional mount path of the filesystem.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(Filesystem))`: If a unique match is found based on the UUID.
    /// - `Ok(None)`: If no match is found based on the UUID.
    /// - `Err(String)`: If the UUID is not unique among the available filesystems.
    pub fn new(
        backup_config: &BackupConfig,
        available_filesystems: &[BlockDevice],
        mountpath: Option<String>,
    ) -> Result<Option<Filesystem>, String> {
        let uuid_filtered_lsblk =
            Self::validate_uuid_uniq(&backup_config.uuid, available_filesystems)?;

        match Self::validate_present_uuid(uuid_filtered_lsblk) {
            Some(blockdevice) => {
                let filesystem = Filesystem {
                    blockdevice: blockdevice.clone(),
                    device_path: format!("/dev/{}", &blockdevice.name),
                    mountpath: mountpath.unwrap_or("/mnt".to_string()),
                    fsavail: blockdevice
                        .fsavail
                        .clone()
                        .map(|fsavail| convert_to_byte_size(&fsavail).unwrap_or(None))
                        .unwrap_or(None),
                    fsck_command: backup_config
                        .fsck_command
                        .clone()
                        .unwrap_or("fsck -n".to_string()),
                    skip_fsck: backup_config.skip_fsck.unwrap_or(false),
                };
                debug!("{:?}", filesystem);
                Ok(Some(filesystem))
            }
            None => {
                info!(
                    "Filesystem with uuid {}, not found, skipping it",
                    &backup_config.uuid
                );
                Ok(None)
            }
        }
    }

    /// Validates if the UUID is associated with a unique block device.
    /// Returns `Some(&BlockDevice)` if the UUID is unique and associated with a block device,
    /// or `None` if it's not unique.
    fn validate_present_uuid(uuid_filtered_lsblk: Vec<&BlockDevice>) -> Option<&BlockDevice> {
        if uuid_filtered_lsblk.len() == 1 {
            Some(uuid_filtered_lsblk[0])
        } else {
            None
        }
    }

    /// Validates if the UUID is unique among the available filesystems.
    /// Returns a filtered list of block devices with the specified UUID, or an error if the UUID is not unique.
    fn validate_uuid_uniq<'b>(
        uuid: &str,
        available_filesystems: &'b [BlockDevice],
    ) -> Result<Vec<&'b BlockDevice>, String> {
        let uuid_filtered_lsblk: Vec<&BlockDevice> = available_filesystems
            .iter()
            .filter(|filesystem| filesystem.uuid.as_deref() == Some(uuid))
            .collect::<Vec<&BlockDevice>>();

        if uuid_filtered_lsblk.len() <= 1 {
            Ok(uuid_filtered_lsblk)
        } else {
            Err(format!("Not a unique UUID: {}", uuid))
        }
    }

    /// Checks if the device is mounted.
    /// Returns `true` if the device is mounted, otherwise `false`.
    pub fn is_mounted(&self) -> bool {
        self.blockdevice.mountpoint.is_some()
    }

    /// Mounts the device.
    /// Returns `Ok(())` if the device is mounted successfully, otherwise returns an error message.
    pub fn mount(&mut self) -> Result<(), String> {
        let output = command_output(
            vec!["mount", &self.device_path, &self.mountpath],
            &format!(
                "mount filesystem {} at {}",
                self.device_path, self.mountpath
            ),
            Some(true),
        )?;

        if output.status.success() {
            self.blockdevice.mountpoint = Some(self.mountpath.clone());
            info!(
                "Filesystem {} mounted successfully on {}",
                self.device_path, self.mountpath
            );
            Ok(())
        } else {
            Err(format!(
                "Error mounting filesystem {} on {}",
                self.device_path, self.mountpath
            ))
        }
    }

    /// Unmounts the device.
    /// Returns `Ok(())` if the device is unmounted successfully, otherwise returns an error message.
    pub fn unmount(&mut self) -> Result<(), String> {
        let mountpoint = self
            .blockdevice
            .mountpoint
            .clone()
            .ok_or(self.mountpath.clone())?;

        command_output(vec!["sync"], "execute sync", Some(false))?;

        let output = command_output(
            vec!["umount", &mountpoint],
            &format!("unmount filesystem {} at {}", self.device_path, &mountpoint),
            Some(true),
        )?;

        if output.status.success() {
            self.blockdevice.mountpoint = None;
            info!("Filesystem {} unmounted successfully", self.device_path);
            Ok(())
        } else {
            Err(format!(
                "Error unmounting filesystem {} at {}: {}",
                self.device_path,
                &mountpoint,
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Checks if the number of existing backups exceeds the specified number of copies.
    pub fn present_number_of_copies(
        &self,
        suffix_file_name_pattern: &str,
        backup_dst_dir: &str,
    ) -> usize {
        let backup_files = match fs::read_dir(backup_dst_dir) {
            Ok(files) => files
                .filter_map(|entry| {
                    entry.ok().and_then(|e| {
                        e.file_name()
                            .to_str()
                            .map(|s| s.to_string())
                            .filter(|s| s.contains(suffix_file_name_pattern))
                    })
                })
                .collect::<Vec<String>>(),
            Err(_) => Vec::new(),
        };

        backup_files.len() // >= self.backup_device.copies as usize
    }

    /// Deletes the oldest backup file.
    pub fn delete_oldest_backup(
        &self,
        suffix_file_name_pattern: &str,
        backup_dst_path: &str,
    ) -> Result<(), String> {
        let present_backup_files =
            self.present_backup_files(suffix_file_name_pattern, backup_dst_path)?;
        if let Some(oldest_file) = present_backup_files.iter().min_by_key(|&file_name| {
            let file_path = Path::new(backup_dst_path).join(file_name);
            if let Ok(metadata) = fs::metadata(file_path) {
                if let Ok(created) = metadata.created() {
                    return created;
                }
            }
            // fallback value to ensure consistent ordering
            std::time::UNIX_EPOCH
        }) {
            let file_path = format!("{}/{}", backup_dst_path, oldest_file);
            info!("Delete old back up file: {}", file_path);
            fs::remove_file(&file_path)
                .map_err(|e| format!("Failed to delete oldest backup file '{}': {}", file_path, e))
        } else {
            Ok(())
        }
    }

    /// Returns the available space of the block device, converted to bytes, or None if the size is unavailable / readable.
    pub fn available_space(&self) -> Result<Option<u64>, String> {
        let device_uuid = self.blockdevice.uuid.clone();
        // needs a new lsblk instance, since the filesystem size is only accessible if mounted
        let lsblk = Lsblk::new()?;
        let filesystem = lsblk
            .available_filesystems
            .iter()
            .find(|fs| fs.uuid == device_uuid)
            .unwrap();

        Ok(filesystem
            .fsavail
            .clone()
            .map(|fsavail| convert_to_byte_size(&fsavail).unwrap_or(None))
            .unwrap_or(None))
    }

    fn present_backup_files(
        &self,
        suffix_file_name_pattern: &str,
        backup_dst_path: &str,
    ) -> Result<Vec<String>, String> {
        let present_backup_files = fs::read_dir(backup_dst_path)
            .map_err(|e| format!("Failed to read backup directory: {}", e))?
            .filter_map(|entry| {
                entry.ok().and_then(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| s.to_string())
                        .filter(|s| s.contains(suffix_file_name_pattern))
                })
            })
            .collect::<Vec<String>>();
        Ok(present_backup_files)
    }

    /// Validates the filesystem check configuration.
    ///
    /// If the `skip_fsck` field is set to `true`, this function returns `Ok(())` without performing any checks.
    /// If the `skip_fsck` field is set to `false` or not specified, this function executes the `fsck` command
    /// specified in the `fsck_command` (otherwise `fsck -n /dev/path1`) field and checks if the command succeeded.
    /// If the command succeeds, it returns `Ok(())`. Otherwise, it returns an `Err` with an error message.
    pub fn validate_fsck_or_skip(&self) -> Result<(), String> {
        match self.skip_fsck {
            true => Ok(()),
            false => {
                let fsck_command = &self.fsck_command.clone();
                let mut command_parts: Vec<&str> = fsck_command.split(' ').collect();
                command_parts.push(self.device_path.as_str());

                let output = command_output(command_parts, "check fs", Some(true))?;

                if output.status.success() {
                    Ok(())
                } else {
                    Err("ATTENTION: fsck was not successfull".to_string())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_filesystems() -> Vec<BlockDevice> {
        vec![
            BlockDevice {
                name: "sda1".to_string(),
                model: Some("model1".to_string()),
                serial: Some("serial1".to_string()),
                uuid: Some("uuid1".to_string()),
                mountpoint: Some("/mnt/sda1".to_string()),
                size: "100GB".to_string(),
                fsavail: Some("50GB".to_string()),
            },
            BlockDevice {
                name: "sdb1".to_string(),
                model: Some("model2".to_string()),
                serial: Some("serial2".to_string()),
                uuid: Some("uuid2".to_string()),
                mountpoint: Some("/mnt/sdb1".to_string()),
                size: "200GB".to_string(),
                fsavail: Some("100GB".to_string()),
            },
            BlockDevice {
                name: "sdc1".to_string(),
                model: Some("model3".to_string()),
                serial: Some("serial3".to_string()),
                uuid: Some("uuid2".to_string()), // Duplicate UUID
                mountpoint: Some("/mnt/sdc1".to_string()),
                size: "300GB".to_string(),
                fsavail: Some("150GB".to_string()),
            },
        ]
    }

    #[test]
    fn test_validate_present_uuid() {
        let filesystems = generate_test_filesystems();

        let uuid_filtered_lsblk = filesystems
            .iter()
            .filter(|fs| fs.uuid.as_deref() == Some("uuid1"))
            .collect::<Vec<&BlockDevice>>();
        assert!(Filesystem::validate_present_uuid(uuid_filtered_lsblk).is_some());

        let uuid_filtered_lsblk = filesystems
            .iter()
            .filter(|fs| fs.uuid.as_deref() == Some("uuid2"))
            .collect::<Vec<&BlockDevice>>();
        assert!(Filesystem::validate_present_uuid(uuid_filtered_lsblk).is_none());
    }

    #[test]
    fn test_validate_uuid_uniq() {
        let filesystems = generate_test_filesystems();

        assert!(Filesystem::validate_uuid_uniq("uuid1", &filesystems).is_ok());
        assert!(Filesystem::validate_uuid_uniq("uuid2", &filesystems).is_err());
        assert!(Filesystem::validate_uuid_uniq("uuid3", &filesystems).is_ok()); // UUID not present
    }
}
