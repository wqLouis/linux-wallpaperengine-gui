//! Detect currently connected display outputs for `linux-wallpaperengine`'s
//! `--screen-root` argument. Tries `wlr-randr` first (the native Wayland
//! tool emitted by wlroots-based compositors), then falls back to `xrandr`
//! (X11 / Xwayland). Returns a sorted, deduplicated list of connected
//! output names suitable for passing to `--screen-root`.

use std::process::Command;

/// Return the list of currently connected display output names
/// (e.g. `["DP-3"]`, `["eDP-1", "HDMI-A-1"]`).
///
/// `None` if neither `wlr-randr` nor `xrandr` is available, or if both
/// fail — the caller should treat that as a hard error.
pub fn detect_connected_displays() -> Option<Vec<String>> {
    if let Some(v) = detect_via("wlr-randr", &[]) {
        return Some(v);
    }
    if let Some(v) = detect_via("xrandr", &["--query"]) {
        return Some(v);
    }
    None
}

fn detect_via(bin: &str, args: &[&str]) -> Option<Vec<String>> {
    let output = Command::new(bin).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let names = parse_connected(&stdout);
    if names.is_empty() {
        None
    } else {
        Some(dedupe_and_sort(names))
    }
}

/// Heuristic parser that handles both `wlr-randr` (no flags) and
/// `xrandr --query` output. Returns a list of connected output names
/// (sorted, deduplicated, deterministic).
fn parse_connected(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // wlr-randr format:
        //   <NAME> "<DESCRIPTION>"
        //     Enabled: yes
        //     ...
        // We match a line that has a token followed by a quoted string.
        if let Some(name) = parse_wlr_randr_head(line) {
            // Look ahead for "Enabled: yes" in the indented block.
            let mut j = i + 1;
            let mut enabled = false;
            while j < lines.len() && (lines[j].starts_with('\t') || lines[j].starts_with("  ")) {
                if lines[j].trim() == "Enabled: yes" {
                    enabled = true;
                    break;
                }
                j += 1;
            }
            if enabled && !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
            i = j;
            continue;
        }

        // xrandr --query format:
        //   <NAME> connected ...
        //   <NAME> connected primary ...
        //   <NAME> disconnected ...
        if let Some(name) = parse_xrandr_line(line) {
            if !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
        i += 1;
    }
    out.sort();
    out
}

/// Returns Some(name) if `line` looks like `DP-3 "Description"`.
fn parse_wlr_randr_head(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next()?;
    if first.is_empty() || first.starts_with('{') {
        return None;
    }
    let rest = parts.next()?;
    // A wlr-randr head line ends with a quoted description.
    if rest.trim_start().starts_with('"') {
        Some(first)
    } else {
        None
    }
}

/// Returns Some(name) if `line` is an xrandr "connected" entry.
fn parse_xrandr_line(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next()?;
    if first.is_empty() || first.starts_with('(') || first.contains(':') {
        return None;
    }
    let rest = parts.next()?;
    if rest.contains("connected") && !rest.contains("disconnected") {
        Some(first)
    } else {
        None
    }
}

fn dedupe_and_sort(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xrandr_output() {
        let s = "\
Screen 0: minimum 16 x 16, current 3840 x 2160, maximum 32767 x 32767
DP-3 connected 3840x2160+0+0 (normal left inverted right x axis y axis) 600mm x 340mm
   3840x2160    159.94*+
HDMI-A-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 600mm x 340mm
   1920x1080     60.00*+
VGA-1 disconnected (normal left inverted right x axis y axis)
";
        let mut v = parse_connected(s);
        v.sort();
        assert_eq!(v, vec!["DP-3".to_string(), "HDMI-A-1".to_string()]);
    }

    #[test]
    fn parses_wlr_randr_text_output() {
        let s = "\
DP-3 \"BOE 0x0812 Unknown\"
\tEnabled: yes
\tPosition: 0,0
\tTransform: normal
\tScale: 1.000000
\tModes:
\t\t3840x2160 px, 159.940001 Hz
HDMI-A-1 \"LG Electronics 27GN950\"
\tEnabled: no
\tPosition: 0,0
VGA-1 \"Old VGA\"
\tEnabled: yes
";
        let v = parse_connected(s);
        assert_eq!(v, vec!["DP-3".to_string(), "VGA-1".to_string()]);
    }

    #[test]
    fn parses_wlr_randr_with_spaces_indent() {
        // Some wlr-randr versions use spaces instead of tabs.
        let s = "\
DP-3 \"BOE\"
  Enabled: yes
  Position: 0,0
";
        let v = parse_connected(s);
        assert_eq!(v, vec!["DP-3".to_string()]);
    }

    #[test]
    fn ignores_xrandr_disconnected() {
        let s = "DP-3 connected 3840x2160+0+0 ...\nVGA-1 disconnected (normal ...)\n";
        let v = parse_connected(s);
        assert_eq!(v, vec!["DP-3".to_string()]);
    }

    #[test]
    fn dedupes_repeated_names() {
        let s = "DP-3 connected ...\nDP-3 connected ...\n";
        let v = parse_connected(s);
        assert_eq!(v, vec!["DP-3".to_string()]);
    }

    /// Live test: parse the *actual* xrandr output from this system.
    /// Skipped if xrandr isn't installed.
    #[test]
    fn live_xrandr_parses_at_least_one_output() {
        let out = std::process::Command::new("xrandr")
            .arg("--query")
            .output();
        let output = match out {
            Ok(o) if o.status.success() => o,
            _ => return, // not in a graphical session — skip
        };
        let s = String::from_utf8_lossy(&output.stdout);
        let names = parse_connected(&s);
        eprintln!("live xrandr → parsed: {:?}", names);
        assert!(
            !names.is_empty(),
            "live xrandr produced no connected outputs, raw was:\n{}",
            s
        );
    }
}
