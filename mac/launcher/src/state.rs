#[derive(Debug, Clone)]
pub enum AppState {
    Idle,

    Discovering,

    Resolving(String),

    Connecting(String),

    WaitingForStream {
        host: String,
        fallback_seconds_remaining: Option<u8>,
    },

    Playing(String),

    ReconnectingStream {
        host: String,
        seconds_remaining: u8,
    },

    ReconnectingHost {
        host: String,
        seconds_remaining: u8,
    },

    Error(String),
}


impl AppState {
    pub fn message(&self) -> String {
        match self {
            AppState::Idle => {
                "Ready".into()
            }

            AppState::Discovering => {
                "Searching for Portable Game Caster…".into()
            }

            AppState::Resolving(host) => {
                format!("Found {host}")
            }

            AppState::Connecting(host) => {
                format!("Connecting to {host}…")
            }

            AppState::WaitingForStream {
                host,
                fallback_seconds_remaining: None,
            } => {
                format!("Waiting for stream from {host}…")
            }

            AppState::WaitingForStream {
                host: _,
                fallback_seconds_remaining: Some(seconds_remaining),
            } => {
                format!(
                    "Stream startup is taking longer than expected… {seconds_remaining}s"
                )
            }

            AppState::Playing(host) => {
                format!("Connected to {host}")
            }

            AppState::ReconnectingStream {
                host,
                seconds_remaining,
            } => {
                format!(
                    "Connection to stream on {host} lost. Reconnecting… {seconds_remaining}"
                )
            }

            AppState::ReconnectingHost {
                host,
                seconds_remaining,
            } => {
                format!(
                    "Connection to {host} lost. Reconnecting… {seconds_remaining}"
                )
            }

            AppState::Error(message) => {
                message.clone()
            }
        }
    }


    pub fn is_busy(&self) -> bool {
        matches!(
            self,

            AppState::Discovering
                | AppState::Resolving(_)
                | AppState::Connecting(_)
                | AppState::WaitingForStream { .. }
                | AppState::ReconnectingStream { .. }
                | AppState::ReconnectingHost { .. }
        )
    }
}
