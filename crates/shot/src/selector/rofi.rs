use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

pub(crate) fn select(names: &[String]) -> io::Result<Option<usize>> {
    let choices = names.join("\n");

    let mut child = Command::new("rofi")
        .args(["-dmenu", "-i", "-p", "Monitor"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(io::Error::other("failed to open rofi stdin"));
    };
    stdin.write_all(choices.as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let selected = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let selected = selected.trim();
    names
        .iter()
        .position(|name| name == selected)
        .map(Some)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("rofi returned an invalid monitor: {selected:?}"),
            )
        })
}
