use crate::state::AppState;

pub fn set_state(state: &AppState) {
    println!("{}", state.message());
}