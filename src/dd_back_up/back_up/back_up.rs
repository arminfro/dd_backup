use crate::dd_back_up::utils::current_date;

use super::{command_output::command_output, device::Device, filesystem::Filesystem};

pub struct BackUp<'a> {
    dst_filesystem: &'a Filesystem,
    back_up_device: &'a Device,
}

impl<'a> BackUp<'a> {
    /// Creates a new `BackUp` instance.
    ///
    /// # Arguments
    ///
    /// * `dst_filesystem` - The destination filesystem for the backup.
    /// * `back_up_device` - The device to be backed up.
    pub fn new(dst_filesystem: &'a Filesystem, back_up_device: &'a Device) -> BackUp<'a> {
        BackUp {
            dst_filesystem,
            back_up_device,
        }
    }

    /// Runs the backup process using the `dd` command.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the backup process is successful.
    /// * `Err` with an error message if the backup process encounters an error.
    pub fn run(&self) -> Result<(), String> {
        let input_file_arg = format!("if={}", self.input_file_path());
        let output_file_arg = format!("of={}", self.output_file_path());
        let command_parts = vec!["dd", &input_file_arg, &output_file_arg, "status=progress"];
        let description = format!("run dd command: {:?}", &command_parts.join(" "));
        let output = command_output(command_parts.clone(), description.as_str(), Some(true))?;

        if output.status.success() {
            println!(
                "Success running backup with dd command {}: {}",
                &command_parts.join(" "),
                String::from_utf8_lossy(&output.stdout).to_string()
            );

            Ok(())
        } else {
            Err(format!(
                "Error running dd command {}: {}",
                &command_parts.join(" "),
                String::from_utf8_lossy(&output.stderr).to_string()
            ))
        }
    }

    /// Returns the input file path for the backup.
    fn input_file_path(&self) -> String {
        self.back_up_device.device_path.clone()
    }

    /// Returns the output file path for the backup.
    fn output_file_path(&self) -> String {
        format!(
            "{}/{}",
            self.dst_filesystem.blockdevice.mountpoint.clone().unwrap(),
            self.file_name()
        )
    }

    /// Generates the file name for the backup image.
    fn file_name(&self) -> String {
        format!(
            "{}.img",
            vec![
                self.back_up_device.blockdevice.model.clone(),
                self.back_up_device.blockdevice.serial.clone(),
                Some(current_date()),
            ]
            .into_iter()
            .filter_map(|x| x)
            .collect::<Vec<String>>()
            .join("-")
            .replace(" ", "_")
        )
    }
}
