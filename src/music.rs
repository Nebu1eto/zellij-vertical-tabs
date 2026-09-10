//! Now-playing state read from Apple Music on macOS.

use std::collections::BTreeMap;

pub(crate) const DEFAULT_MUSIC_FORMAT: &str = "{artist} - {music}";
pub(crate) const DEFAULT_MUSIC_MAX_WIDTH: usize = 40;
/// Album art cannot be drawn: Zellij discards the kitty graphics APC from plugin
/// panes and Ghostty has no sixel support, so the placeholder is a glyph for now.
pub(crate) const ALBUM_ART_GLYPH: &str = "♪";
/// Field separator in the AppleScript output: track metadata may contain
/// newlines and tabs, but never the ASCII record separator.
pub(crate) const FIELD_SEPARATOR: char = '\u{1e}';

/// One `osascript` call that never launches Music when it is closed and never
/// touches `current track` while the player is stopped, where it would error.
pub(crate) const NOW_PLAYING_SCRIPT: &str = r#"if application "Music" is running then
    tell application "Music"
        set playerState to (player state as string)
        if playerState is "stopped" then
            return "stopped"
        end if
        set rs to character id 30
        set t to current track
        return playerState & rs & (player position as string) & rs & (duration of t as string) & rs & (name of t) & rs & (artist of t) & rs & (album of t)
    end tell
else
    return "not_running"
end if"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RightPanel {
    #[default]
    Clock,
    Music,
    Both,
}

impl RightPanel {
    pub(crate) fn from_config(configuration: &BTreeMap<String, String>) -> Self {
        match configuration.get("right_panel").map(String::as_str) {
            Some("music") => RightPanel::Music,
            Some("both") => RightPanel::Both,
            _ => RightPanel::Clock,
        }
    }

    pub(crate) fn shows_music(self) -> bool {
        matches!(self, RightPanel::Music | RightPanel::Both)
    }

    pub(crate) fn shows_clock(self) -> bool {
        matches!(self, RightPanel::Clock | RightPanel::Both)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerState {
    Playing,
    Paused,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NowPlaying {
    pub(crate) state: PlayerState,
    pub(crate) position_seconds: f64,
    pub(crate) duration_seconds: f64,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
}

impl NowPlaying {
    pub(crate) fn remaining_seconds(&self) -> f64 {
        (self.duration_seconds - self.position_seconds).max(0.0)
    }
}

/// Parses the script output; `None` means nothing is playing (stopped, Music
/// closed, or unreadable output), which hides the segment.
pub(crate) fn parse_now_playing(stdout: &str) -> Option<NowPlaying> {
    let stdout = stdout.trim_end_matches(['\r', '\n']);
    let mut fields = stdout.split(FIELD_SEPARATOR);
    let state = match fields.next()?.trim() {
        "playing" => PlayerState::Playing,
        "paused" => PlayerState::Paused,
        _ => return None,
    };
    let position_seconds = parse_seconds(fields.next()?)?;
    let duration_seconds = parse_seconds(fields.next()?)?;
    let title = fields.next()?.to_string();
    let artist = fields.next()?.to_string();
    let album = fields.next()?.to_string();
    Some(NowPlaying {
        state,
        position_seconds,
        duration_seconds,
        title,
        artist,
        album,
    })
}

/// AppleScript renders decimals in the user's locale, so "12,5" is as likely as "12.5".
fn parse_seconds(value: &str) -> Option<f64> {
    value.trim().replace(',', ".").parse().ok()
}

/// Seconds until the next poll: half the remaining time while playing, so a
/// skip or pause is noticed within a bounded delay, and a slow fixed cadence
/// otherwise.
pub(crate) fn next_poll_delay_seconds(now_playing: Option<&NowPlaying>) -> u64 {
    match now_playing {
        Some(track) if track.state == PlayerState::Playing => {
            (track.remaining_seconds() / 2.0).round().clamp(3.0, 20.0) as u64
        }
        _ => 10,
    }
}

pub(crate) fn format_now_playing(format: &str, track: &NowPlaying) -> String {
    let mut result = String::with_capacity(format.len() + 32);
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find('}') {
            Some(end) => {
                let replacement = match &after[1..end] {
                    "music" | "title" => Some(track.title.as_str()),
                    "artist" => Some(track.artist.as_str()),
                    "album" => Some(track.album.as_str()),
                    "albumart" => Some(ALBUM_ART_GLYPH),
                    _ => None,
                };
                match replacement {
                    Some(value) => result.push_str(value),
                    None => result.push_str(&after[..=end]),
                }
                rest = &after[end + 1..];
            }
            None => {
                result.push_str(after);
                rest = "";
            }
        }
    }
    result.push_str(rest);
    result
}

pub(crate) fn configured_music_format(configuration: &BTreeMap<String, String>) -> String {
    configuration
        .get("music_format")
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_MUSIC_FORMAT.to_string())
}

pub(crate) fn configured_music_max_width(configuration: &BTreeMap<String, String>) -> usize {
    configuration
        .get("music_max_width")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(DEFAULT_MUSIC_MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(state: &str, position: &str, duration: &str) -> String {
        [state, position, duration, "Song", "Artist", "Album"].join(&FIELD_SEPARATOR.to_string())
    }

    #[test]
    fn parses_playing_output_with_locale_decimals() {
        let track =
            parse_now_playing(&format!("{}\n", sample("playing", "12,5", "200.25"))).unwrap();
        assert_eq!(track.state, PlayerState::Playing);
        assert_eq!(track.position_seconds, 12.5);
        assert_eq!(track.duration_seconds, 200.25);
        assert_eq!(
            (
                track.title.as_str(),
                track.artist.as_str(),
                track.album.as_str()
            ),
            ("Song", "Artist", "Album")
        );
    }

    #[test]
    fn stopped_closed_and_garbage_hide_the_segment() {
        assert_eq!(parse_now_playing("stopped\n"), None);
        assert_eq!(parse_now_playing("not_running"), None);
        assert_eq!(parse_now_playing(""), None);
        assert_eq!(
            parse_now_playing("playing\u{1e}abc\u{1e}1\u{1e}a\u{1e}b\u{1e}c"),
            None
        );
    }

    #[test]
    fn poll_delay_follows_half_the_remaining_time_within_bounds() {
        let mut track = parse_now_playing(&sample("playing", "0", "30")).unwrap();
        assert_eq!(next_poll_delay_seconds(Some(&track)), 15);
        track.position_seconds = 28.0;
        assert_eq!(next_poll_delay_seconds(Some(&track)), 3);
        track.duration_seconds = 600.0;
        assert_eq!(next_poll_delay_seconds(Some(&track)), 20);
        track.state = PlayerState::Paused;
        assert_eq!(next_poll_delay_seconds(Some(&track)), 10);
        assert_eq!(next_poll_delay_seconds(None), 10);
    }

    #[test]
    fn format_replaces_known_placeholders_and_keeps_unknown_ones() {
        let track = parse_now_playing(&sample("playing", "0", "30")).unwrap();
        assert_eq!(
            format_now_playing(
                "{albumart} {artist} - {music} ({album}) {title} {x} {",
                &track
            ),
            "♪ Artist - Song (Album) Song {x} {"
        );
    }

    #[test]
    fn right_panel_defaults_to_clock() {
        let mut configuration = BTreeMap::new();
        assert_eq!(RightPanel::from_config(&configuration), RightPanel::Clock);
        configuration.insert("right_panel".to_string(), "both".to_string());
        assert_eq!(RightPanel::from_config(&configuration), RightPanel::Both);
        configuration.insert("right_panel".to_string(), "nonsense".to_string());
        assert_eq!(RightPanel::from_config(&configuration), RightPanel::Clock);
    }
}
