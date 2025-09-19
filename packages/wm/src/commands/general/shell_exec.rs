use std::process::Command;

use anyhow::bail;

pub fn shell_exec(
  command: &str,
  _hide_window: bool,
) -> anyhow::Result<()> {
  if command.trim().is_empty() {
    return Err(
      wm_platform::Error::Platform("No command provided".into()).into(),
    );
  }

  let home = home::home_dir().ok_or(wm_platform::Error::Platform(
    "Failed to get home directory".into(),
  ))?;

  let split;

  let (program, args): (&str, &[&str]) = if command.starts_with('"') {
    // Find the closing double quote.
    let Some((closing_index, _)) = command.match_indices('"').nth(2)
    else {
      bail!("Command doesn't have an ending `\"`: '{command}'.")
    };

    let program = &command[0..closing_index];
    split = command[closing_index..]
      .split_whitespace()
      .collect::<Vec<&str>>();

    (program, &split)
  } else {
    split = command.split_whitespace().collect::<Vec<&str>>();

    match split.split_first() {
      Some((program, args)) => (program, args),
      None => unreachable!(), // Already checked if empty
    }
  };

  // TODO: How does hide_window work with std::process::Command? Might need
  // to change to "show window" instead and manually spawn a seperate
  // terminal for it. Or leave it up to the user to spawn their own

  Command::new(program)
    .args(args)
    .current_dir(&home)
    .spawn()?;

  Ok(())
}
