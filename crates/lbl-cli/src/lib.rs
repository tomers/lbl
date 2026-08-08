//! Shared CLI presentation for lbl binaries.
//!
//! Help styling follows the same preset as `cargo` subcommands (via
//! [`clap_cargo::style::CLAP_STYLING`]): green section headers, cyan flags,
//! and colored error hints — without hand-picking ANSI codes in each binary.
//!
//! Colorized stdout/stderr (including half-block console previews) also needs
//! Windows virtual-terminal mode; see [`enable_ansi_support`] and
//! [`color_for_tty`].

pub use clap_cargo::style::CLAP_STYLING;

/// Enable console interpretation of ANSI SGR / CSI (Windows virtual terminal).
///
/// Safe to call many times. On non-Windows this is a no-op that reports support.
/// On Windows it sets `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on stdout and stderr
/// (see [console virtual terminal sequences][vt]). Without that flag, classic
/// `conhost` prints escape sequences as literal text instead of colors.
///
/// Returns whether ANSI output is usable for this process after the attempt.
///
/// [vt]: https://learn.microsoft.com/en-us/windows/console/console-virtual-terminal-sequences
pub fn enable_ansi_support() -> bool {
    #[cfg(windows)]
    {
        // Once per process: SetConsoleMode is process-wide for the handles.
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| anstyle_query::windows::enable_ansi_colors() == Some(true))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Whether colored terminal output should be emitted for a TTY stream.
///
/// Honors `NO_COLOR`. On Windows, enables virtual-terminal processing first and
/// only returns true when that succeeds so callers never paint raw escapes into
/// a classic console that does not interpret them.
pub fn color_for_tty(is_tty: bool) -> bool {
    if !is_tty || std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    enable_ansi_support()
}
