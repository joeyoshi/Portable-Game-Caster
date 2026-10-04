mod discovery;
mod logging;
mod player;
mod state;
mod ui;
mod worker;


fn main() {
    logging::init_from_args();

    ui::run_app();
}