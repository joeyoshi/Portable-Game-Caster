#[derive(Debug, Clone)]
pub enum AppState {
    Idle,
    Discovering,
    Resolving,
    Connecting,
    WaitingForStream,
    Playing,
    Error(String),
}

impl AppState {
    pub fn message(&self) -> String {
        match self {
            AppState::Idle => "Ready".into(),
            AppState::Discovering => {
                "Searching for Portable Game Caster…".into()
            }
            AppState::Resolving => {
                "Resolving host…".into()
            }
            AppState::Connecting => {
                "Connecting to host…".into()
            }
            AppState::WaitingForStream => {
                "Waiting for stream…".into()
            }
            AppState::Playing => {
                "Stream connected".into()
            }
            AppState::Error(message) => {
                format!("Error: {message}")
            }
        }
    }
}