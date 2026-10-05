mod discovery;
mod logging;
mod player;
mod state;
mod ui;
mod worker;


// PGC discovery/stream protocol this Client speaks. Independent of the Client
// product version; must match the Host's advertised protocol version.
const PROTOCOL_VERSION: &str = "1";


// How this build was produced. Versioning work will select this at build time;
// until then every build is a development build.
const BUILD_CHANNEL: logging::BuildChannel = logging::BuildChannel::Development;


fn main() {
    logging::init_from_args();

    ui::run_app();
}