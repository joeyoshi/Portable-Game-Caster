#[derive(Debug, Clone)]
pub enum AppState {
    Idle,

    Discovering,

    Resolving(String),

    Connecting(String),

    WaitingForStream(String),

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

            AppState::WaitingForStream(host) => {
                format!("Waiting for stream from {host}…")
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
                | AppState::WaitingForStream(_)
                | AppState::ReconnectingStream { .. }
                | AppState::ReconnectingHost { .. }
        )
    }
}