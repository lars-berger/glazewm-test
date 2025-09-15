use tracing::info;
use wm_platform::platform_prelude::*;

pub fn shell_exec(
  _command: &str,
  _hide_window: bool,
) -> anyhow::Result<()> {
  #[cfg(target_os = "windows")]
  {
    let (program, args) = Platform::parse_command(_command)?;
    info!("Parsed command program: '{}', args: '{}'.", program, args);

    Platform::run_command(&program, &args, _hide_window).map_err(
      |err| {
        anyhow::anyhow!(format!(
          "Failed to execute '{_command}'.\n\nError: {err}"
        ))
      },
    )?;
  }

  Ok(())
}
