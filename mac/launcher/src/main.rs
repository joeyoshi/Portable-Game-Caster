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


// macOS Client build number: a counter of this executable's formal checkpoints,
// independent of the product version (Cargo.toml) and of other platforms'
// builds. Raised by hand, never derived. Keep CFBundleVersion in
// mac/app/Info.plist equal to it.
const BUILD_NUMBER: u32 = 1;


fn main() {
    logging::init_from_args();

    ui::run_app();
}