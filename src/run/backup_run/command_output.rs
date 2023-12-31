use std::process::{Command, Output, Stdio};

/// Executes a command and captures its output.
/// Command output is still printed to stdout and stderr.
///
/// # Arguments
///
/// * `command_parts` - The parts of the command.
/// * `description` - The description of the command.
/// * `is_sudo_needed` - Indicates whether sudo should be used for the command (if available).
///
/// # Returns
///
/// * `Ok(output)` if the command executes successfully and captures the output.
/// * `Err` with an error message if the command encounters an error.
pub fn command_output(
    command_parts: Vec<&str>,
    description: &str,
    is_sudo_needed: Option<bool>,
) -> Result<Output, String> {
    let command_parts = {
        if is_sudo_needed.unwrap_or(false) {
            append_sudo_if_available(command_parts, Some(description))
        } else {
            command_parts
        }
    };

    trace!("Command: {}", command_parts.join(" "));
    match Command::new(command_parts[0])
        .args(&command_parts[1..])
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            match output.status.success() {
                true => Ok(output),
                false => Err(format!(
                    "Error running {}: {}",
                    &command_parts.join(" "),
                    String::from_utf8_lossy(&output.stderr)
                )),
            }
        }
        Err(err) => Err(format!("{}: {}", err, command_parts.join(" "))),
    }
}

fn append_sudo_if_available<'a>(
    command_parts: Vec<&'a str>,
    description: Option<&str>,
) -> Vec<&'a str> {
    let mut updated_command_parts = Vec::new();

    if is_sudo_available() {
        updated_command_parts.push("sudo");
        let sudo_message = "Sudo is needed";
        match description {
            Some(description) => info!("{} to {}", sudo_message, description),
            None => info!("{}", sudo_message),
        };
    }

    updated_command_parts.extend_from_slice(command_parts.as_slice());
    updated_command_parts
}

fn is_sudo_available() -> bool {
    Command::new("sudo").arg("--version").output().is_ok()
}
