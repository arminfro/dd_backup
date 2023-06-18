use std::path::Path;

use chrono::Local;
use chrono_humanize::Humanize;
use relative_path::RelativePath;

use crate::run::utils::current_date;

use super::{command_output::command_output, device::Device, filesystem::Filesystem, BackupArgs};

#[derive(Debug)]
pub struct Backup<'a> {
    /// The destination filesystem for the backup.
    pub dst_filesystem: &'a Filesystem,
    /// The backup device.
    pub backup_device: &'a Device,
    /// The command line arguments for the backup operation.
    pub backup_args: &'a BackupArgs,
}

impl<'a> Backup<'a> {
    /// Creates a new `BackUp` instance.
    ///
    /// # Arguments
    ///
    /// * `dst_filesystem` - The destination filesystem for the backup.
    /// * `backup_device` - The device to be backed up.
    pub fn new(
        dst_filesystem: &'a Filesystem,
        backup_device: &'a Device,
        backup_args: &'a BackupArgs,
    ) -> Backup<'a> {
        let backup = Backup {
            dst_filesystem,
            backup_device,
            backup_args,
        };
        debug!("{:?}", backup);
        backup
    }

    /// Runs the backup process using the `dd` command.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the backup process is successful.
    /// * `Err` with an error message if the backup process encounters an error.
    pub fn run(&self) -> Result<(), String> {
        self.validate_state()?;

        let input_file_arg = format!("if={}", self.backup_device.device_path.clone());
        let output_file_arg = format!("of={}", self.backup_file_path());
        let command_parts = vec!["dd", &input_file_arg, &output_file_arg, "status=progress"];
        let description = format!("run dd command: {:?}", &command_parts.join(" "));
        match self.backup_args.dry {
            true => {
                info!(
                    "[DRY RUN] backup would run with command: {}",
                    &command_parts.join(" "),
                );
                Ok(())
            }
            false => {
                let time_before_dd = Local::now();
                let output =
                    command_output(command_parts.clone(), description.as_str(), Some(true))?;

                if output.status.success() {
                    let time_after_dd = Local::now();
                    let diff = time_after_dd - time_before_dd;
                    info!(
                        "Success running backup with dd command {} for {}: {}",
                        &command_parts.join(" "),
                        diff.humanize(),
                        String::from_utf8_lossy(&output.stdout).to_string()
                    );

                    self.chown()
                } else {
                    Err(format!(
                        "Error running dd command {}: {}",
                        &command_parts.join(" "),
                        String::from_utf8_lossy(&output.stderr).to_string()
                    ))
                }
            }
        }
    }

    /// Sets the owner of the backup file to the current user ID and group ID.
    ///
    /// This function changes the owner of the backup file specified by `output_file_path`
    /// to the current user and group. It uses the `chown` command to perform the operation.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: If the operation is successful.
    /// - `Err(String)`: If an error occurs during the operation.
    fn chown(&self) -> Result<(), String> {
        let output_file_path = self.backup_file_path();

        // Retrieve the current user and group IDs
        let user_id = unsafe { libc::getuid() };
        let group_id = unsafe { libc::getgid() };

        let user_group_id_arg = format!("{}:{}", user_id, group_id);
        let command_parts = vec!["chown", &user_group_id_arg, &output_file_path];
        command_output(
            command_parts,
            "change owner of backup file to $UID",
            Some(true),
        )?;
        Ok(())
    }

    /// Returns the output dir path for the backup.
    fn backup_dir_path(&self) -> String {
        let relative_path =
            RelativePath::new(&self.dst_filesystem.blockdevice.mountpoint.clone().unwrap())
                .join_normalized(self.backup_device.destination_path.clone())
                .to_string();

        format!("/{}", relative_path)
    }

    /// Returns the output file path for the backup.
    fn backup_file_path(&self) -> String {
        let relative_path = RelativePath::new(&self.backup_dir_path())
            .join_normalized(self.file_name())
            .to_string();

        format!("/{}", relative_path)
    }

    /// Generates the file name for the backup image.
    fn file_name(&self) -> String {
        format!(
            "{}_{}_{}",
            current_date(),
            self.backup_device.name,
            self.suffix_file_name_pattern().replace(" ", "-")
        )
    }

    /// Generates the stable postfix file name for the backup image.
    ///
    /// The stable postfix file name is generated by combining the model and serial
    /// number of the block device associated with the backup. Any spaces in the
    /// names are replaced with hyphens.
    ///
    /// # Returns
    ///
    /// The stable postfix file name as a string.
    fn suffix_file_name_pattern(&self) -> String {
        format!(
            "{}.img",
            vec![
                self.backup_device.blockdevice.model.clone(),
                self.backup_device.blockdevice.serial.clone(),
            ]
            .into_iter()
            .filter_map(|x| x)
            .collect::<Vec<String>>()
            .join("_")
            .replace(" ", "-")
        )
    }

    /// Checks if the number of existing backups exceeds the specified number of copies.
    fn needs_deletion(&self) -> bool {
        let present_number_of_copies = self
            .dst_filesystem
            .present_number_of_copies(&self.suffix_file_name_pattern(), &self.backup_dir_path());
        present_number_of_copies >= self.backup_device.copies as usize
    }

    /// Validates the state of the backup process by performing the following checks:
    /// 1. Checks if the target file is already present. If it is, an error is returned.
    /// 2. Checks if the oldest backup needs to be deleted based on the configured number of copies.
    ///    If a deletion is required, the oldest backup is deleted.
    /// 3. If no deletion is needed, checks if the target filesystem has enough space to accommodate
    ///    the new backup. If there is insufficient space, an error is returned.
    /// If all checks pass, `Ok(())` is returned indicating that the state is valid and the backup
    /// process can proceed.
    fn validate_state(&self) -> Result<(), String> {
        self.target_file_is_present()?;
        let needed_deletion = self.delete_oldest_backup_if_needed()?;
        if !needed_deletion {
            self.target_filesystem_has_enough_space()?;
        }
        Ok(())
    }

    /// Side-Effect: Deletes the oldest backup file if the number of existing backups exceeds the specified number of copies.
    fn delete_oldest_backup_if_needed(&self) -> Result<bool, String> {
        let needs_deletion = self.needs_deletion();
        if needs_deletion {
            if self.backup_args.dry {
                info!(
                    "[DRY RUN] Would delete oldest backup file with suffix: {} in {}",
                    self.suffix_file_name_pattern(),
                    self.backup_dir_path()
                );
            } else {
                self.dst_filesystem.delete_oldest_backup(
                    &self.suffix_file_name_pattern(),
                    &self.backup_dir_path(),
                )?;
            }
        }
        Ok(needs_deletion)
    }

    /// Checks if the target filesystem has enough space to accommodate the backup of the device.
    /// It compares the available space on the filesystem with the total size of the device to be backed up.
    /// If there is sufficient space, `Ok(())` is returned, indicating that the backup can proceed.
    /// If there is not enough space, an error is returned with a descriptive message.
    /// If either available_space or needed_space is None then proceed with an Ok as well.
    fn target_filesystem_has_enough_space(&self) -> Result<(), String> {
        let available_space = self.dst_filesystem.available_space()?;
        let needed_space = self.backup_device.total_size()?;

        if let Some(available_space) = available_space {
            if let Some(needed_space) = needed_space {
                let remaining_space: i64 = available_space as i64 - needed_space as i64;
                if remaining_space > 0 {
                    return Ok(());
                } else {
                    return Err(format!(
                        "Not enough space on destination filesystem {}, to backup device {}",
                        self.dst_filesystem.device_path, self.backup_device.device_path
                    ));
                }
            }
        }
        warn!("Could not check if sufficient space is available");
        Ok(())
    }

    /// Checks if the target backup file is already present.
    ///
    /// If the backup file already exists at the specified output file path,
    /// this function returns an error indicating that the backup should be skipped.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: If the backup file does not exist and can proceed.
    /// - `Err(String)`: If the backup file is already present.
    fn target_file_is_present(&self) -> Result<(), String> {
        let file_path = self.backup_file_path();
        let path = Path::new(&file_path);

        if path.exists() && path.is_file() {
            Err(format!(
                "Backup file for today is already present {}. Skipping it",
                file_path
            ))
        } else {
            Ok(())
        }
    }
}
