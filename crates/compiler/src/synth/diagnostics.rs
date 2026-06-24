//! Structured compiler diagnostics (U11, KTD7).
//!
//! The Critic-Reviewer agent must reason over *structured* errors, not prose, so
//! the rustc→WASM driver (U12) emits `--error-format=json` and this module
//! distills each JSON line into a [`Diagnostic`]. The repair loop uses
//! [`diagnostic_signature`] and [`error_count`] to detect stagnation — a repair
//! that produces the same (or no fewer) errors is making no progress.

use serde::Deserialize;

/// One compiler diagnostic distilled from rustc's JSON output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
    pub code: Option<String>,
}

/// Subset of a rustc JSON diagnostic line we care about.
#[derive(Deserialize)]
struct RustcLine {
    #[serde(default)]
    message: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    code: Option<RustcCode>,
}

#[derive(Deserialize)]
struct RustcCode {
    code: String,
}

/// Parse rustc `--error-format=json` output (one JSON object per line) into the
/// error/warning diagnostics. Non-JSON lines and other levels are ignored.
pub fn parse_rustc_diagnostics(stderr: &str) -> Vec<Diagnostic> {
    stderr
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('{') {
                return None;
            }
            let parsed: RustcLine = serde_json::from_str(line).ok()?;
            if parsed.level != "error" && parsed.level != "warning" {
                return None;
            }
            Some(Diagnostic {
                level: parsed.level,
                message: parsed.message,
                code: parsed.code.map(|c| c.code),
            })
        })
        .collect()
}

/// Number of `error`-level diagnostics (warnings do not block compilation).
pub fn error_count(diags: &[Diagnostic]) -> usize {
    diags.iter().filter(|d| d.level == "error").count()
}

/// An order-independent signature of a diagnostic set, used to detect when a
/// repair attempt produced the *identical* set of problems (stagnation).
pub fn diagnostic_signature(diags: &[Diagnostic]) -> Vec<String> {
    let mut sig: Vec<String> = diags
        .iter()
        .map(|d| {
            format!(
                "{}|{}|{}",
                d.level,
                d.code.as_deref().unwrap_or(""),
                d.message
            )
        })
        .collect();
    sig.sort();
    sig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_lines_and_ignores_noise() {
        let stderr = concat!(
            "Compiling node v0.1.0\n",
            "{\"message\":\"mismatched types\",\"level\":\"error\",\"code\":{\"code\":\"E0308\"}}\n",
            "{\"message\":\"unused import\",\"level\":\"warning\"}\n",
            "some non-json trailing text\n",
        );
        let diags = parse_rustc_diagnostics(stderr);
        assert_eq!(diags.len(), 2);
        assert_eq!(error_count(&diags), 1);
        assert_eq!(diags[0].code.as_deref(), Some("E0308"));
    }

    #[test]
    fn signature_is_order_independent() {
        let a = vec![
            Diagnostic {
                level: "error".into(),
                message: "x".into(),
                code: None,
            },
            Diagnostic {
                level: "error".into(),
                message: "y".into(),
                code: None,
            },
        ];
        let b = vec![
            Diagnostic {
                level: "error".into(),
                message: "y".into(),
                code: None,
            },
            Diagnostic {
                level: "error".into(),
                message: "x".into(),
                code: None,
            },
        ];
        assert_eq!(diagnostic_signature(&a), diagnostic_signature(&b));
    }
}
