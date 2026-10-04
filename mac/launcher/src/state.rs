#[derive(Debug, Clone)]
pub enum AppState {
    Idle,

    Discovering {
        seconds_remaining: Option<u8>,
    },

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
            AppState::Idle => "Ready".into(),
            AppState::Discovering { .. } => "Searching for a Portable Game Caster host…".into(),
            AppState::Resolving(_) => "Host found.".into(),
            AppState::Connecting(_) => "Connecting to streaming service…".into(),
            AppState::WaitingForStream {
                fallback_seconds_remaining: None,
                ..
            } => "Waiting for stream.".into(),
            AppState::WaitingForStream {
                fallback_seconds_remaining: Some(_),
                ..
            } => "Stream is taking longer than expected.".into(),
            AppState::Playing(_) => "Connected.".into(),
            AppState::ReconnectingStream { .. } => "Connection to stream lost.".into(),
            AppState::ReconnectingHost { .. } => "Connection to host lost.".into(),
            AppState::Error(message) => message.clone(),
        }
    }


    pub fn detail_message(&self) -> Option<String> {
        match self {
            AppState::Discovering {
                seconds_remaining: Some(seconds_remaining),
            } => Some(format!("Searching… {seconds_remaining}")),

            AppState::WaitingForStream {
                fallback_seconds_remaining: Some(seconds_remaining),
                ..
            } => Some(format!("Waiting for stream… {seconds_remaining}")),

            AppState::ReconnectingStream {
                seconds_remaining,
                ..
            }
            | AppState::ReconnectingHost {
                seconds_remaining,
                ..
            } => Some(format!("Reconnecting… {seconds_remaining}")),

            _ => None,
        }
    }


    pub fn host(&self) -> Option<&str> {
        match self {
            AppState::Resolving(host)
            | AppState::Connecting(host)
            | AppState::Playing(host) => Some(host),

            AppState::WaitingForStream { host, .. }
            | AppState::ReconnectingStream { host, .. }
            | AppState::ReconnectingHost { host, .. } => Some(host),

            AppState::Idle
            | AppState::Discovering { .. }
            | AppState::Error(_) => None,
        }
    }


    pub fn is_busy(&self) -> bool {
        matches!(
            self,

            AppState::Discovering { .. }
                | AppState::Resolving(_)
                | AppState::Connecting(_)
                | AppState::WaitingForStream { .. }
                | AppState::ReconnectingStream { .. }
                | AppState::ReconnectingHost { .. }
        )
    }
}
