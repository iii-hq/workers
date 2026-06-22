use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn enabled_for_stdout(&self) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Auto => std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal(),
        }
    }

    /// TUI runs on an alternate screen; treat as a terminal unless explicitly disabled.
    pub fn enabled_for_tui(&self) -> bool {
        match self {
            Self::Never => false,
            Self::Always | Self::Auto => std::env::var("NO_COLOR").is_err(),
        }
    }
}
