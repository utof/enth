//! Playback: one command that plays one WAV file.
//!
//! The default is `pw-play --media-role=Production`. `PipeWire`'s default stream role is "Music", and on the
//! owner's machine stream-restore keeps that role muted; a role of our own starts unmuted and changes no system
//! setting. `--player` replaces the whole command (the file path is appended as the last argument), for example
//! `--player "aplay -q"`. If the replacement is `pw-play` without a `--media-role`, the role is added anyway.

use std::path::Path;
use std::process::{Command, Stdio};

/// The default player command line.
pub const DEFAULT_PLAYER: &str = "pw-play --media-role=Production";

/// Something that plays one WAV file and returns when it has finished.
pub trait Player {
    /// Play `wav` to the end.
    ///
    /// # Errors
    /// The player could not run or exited with a failure.
    fn play(&mut self, wav: &Path) -> Result<(), String>;
}

/// A player that runs an external command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPlayer {
    program: String,
    args: Vec<String>,
}

impl CommandPlayer {
    /// Parse a command line (split on whitespace; no quoting). Adds `--media-role=Production` to `pw-play`.
    ///
    /// # Errors
    /// An empty command line.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.split_whitespace().map(str::to_string);
        let program = parts.next().ok_or(
            "--player is empty; give a command that plays one WAV file, e.g. \"aplay -q\"",
        )?;
        let mut args: Vec<String> = parts.collect();
        let base = Path::new(&program)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if base == "pw-play" && !args.iter().any(|a| a.starts_with("--media-role")) {
            args.insert(0, "--media-role=Production".to_string());
        }
        Ok(Self { program, args })
    }

    /// The command and arguments (without the file), for messages.
    #[must_use]
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Player for CommandPlayer {
    fn play(&mut self, wav: &Path) -> Result<(), String> {
        let status = Command::new(&self.program)
            .args(&self.args)
            .arg(wav)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| {
                format!(
                    "cannot run {:?}: {e} (set --player to a command that plays one WAV file)",
                    self.program
                )
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{} exited with {status}", self.program))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_pw_play_with_the_production_role() {
        let p = CommandPlayer::parse(DEFAULT_PLAYER).unwrap();
        assert_eq!(p.command_line(), "pw-play --media-role=Production");
    }

    #[test]
    fn bare_pw_play_gets_the_role_but_an_explicit_role_stays() {
        assert_eq!(
            CommandPlayer::parse("pw-play").unwrap().command_line(),
            "pw-play --media-role=Production"
        );
        assert_eq!(
            CommandPlayer::parse("/usr/bin/pw-play --volume=0.5")
                .unwrap()
                .command_line(),
            "/usr/bin/pw-play --media-role=Production --volume=0.5"
        );
        assert_eq!(
            CommandPlayer::parse("pw-play --media-role=Music")
                .unwrap()
                .command_line(),
            "pw-play --media-role=Music"
        );
        assert_eq!(
            CommandPlayer::parse("aplay -q").unwrap().command_line(),
            "aplay -q"
        );
        assert!(CommandPlayer::parse("   ").is_err());
    }

    /// `true` and `false` play nothing: they check only that success and failure are told apart.
    #[test]
    fn exit_status_is_reported() {
        let f = Path::new("/nonexistent.wav");
        assert!(CommandPlayer::parse("true").unwrap().play(f).is_ok());
        assert!(CommandPlayer::parse("false").unwrap().play(f).is_err());
        assert!(
            CommandPlayer::parse("definitely-not-a-player-xyz")
                .unwrap()
                .play(f)
                .is_err()
        );
    }
}
