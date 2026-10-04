// -----------------------------------------------------------------------------
// Stream demand signal
// -----------------------------------------------------------------------------
//
// MediaMTX decides when the gameplay path has demand (a reader asked for it and
// nobody is publishing) and runs its `runOnDemand` command for as long as that
// demand lasts. That command is this executable in `--demand-signal` mode: it
// opens a localhost connection to the Host and holds it open. MediaMTX
// terminates it when demand ends, which closes the connection.
//
// So, for the Host:
//
//     signal connection open   = demand present
//     signal connection closed = demand ended
//
// MediaMTX only signals demand. It never launches or owns FFmpeg.

use std::env;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::Event;


// Passed to MediaMTX's environment so the signal helper can find the Host.
pub const ADDRESS_ENV: &str = "PGC_DEMAND_ADDR";

// Lets mediamtx.yml refer to this executable without a hardcoded path.
pub const HOST_EXECUTABLE_ENV: &str = "PGC_HOST_EXE";

pub const SIGNAL_FLAG: &str = "--demand-signal";

const HELLO: &str = "PGC-DEMAND 1";

const HELLO_TIMEOUT: Duration = Duration::from_secs(2);


// -----------------------------------------------------------------------------
// Host side
// -----------------------------------------------------------------------------

type Connections =
    Arc<Mutex<HashMap<u64, TcpStream>>>;


pub struct Listener {
    address: SocketAddr,
    connections: Connections,
}


impl Listener {
    pub fn address(&self) -> SocketAddr {
        self.address
    }


    // Drops every current demand signal. Used when MediaMTX exits: signals from
    // a MediaMTX that no longer exists must not keep FFmpeg wanted, whether or
    // not their helper processes died with it.
    pub fn reset(&self) {
        let connections =
            self.connections
                .lock()
                .expect("demand connection lock poisoned");

        for connection in connections.values() {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}


pub fn listen(events: Sender<Event>) -> io::Result<Listener> {
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;

    let address =
        listener.local_addr()?;

    let connections =
        Connections::default();

    let accepted =
        Arc::clone(&connections);

    thread::spawn(move || {
        let mut next_id = 0_u64;

        for connection in listener.incoming() {
            let Ok(connection) = connection else {
                continue;
            };

            next_id += 1;

            let id = next_id;
            let events = events.clone();
            let connections = Arc::clone(&accepted);

            thread::spawn(move || {
                hold_signal(connection, id, &events, &connections);
            });
        }
    });

    Ok(Listener {
        address,
        connections,
    })
}


fn hold_signal(
    connection: TcpStream,
    id: u64,
    events: &Sender<Event>,
    connections: &Connections,
) {
    // Ignore anything on localhost that is not the signal helper.
    let _ = connection.set_read_timeout(Some(HELLO_TIMEOUT));

    let mut reader =
        BufReader::new(connection);

    let mut hello =
        String::new();

    if reader.read_line(&mut hello).is_err() || hello.trim_end() != HELLO {
        return;
    }

    let _ = reader.get_ref().set_read_timeout(None);

    if let Ok(handle) = reader.get_ref().try_clone() {
        connections
            .lock()
            .expect("demand connection lock poisoned")
            .insert(id, handle);
    }

    if events.send(Event::DemandOpened(id)).is_ok() {
        // The helper never sends anything else. This returns when it is
        // terminated or the Host resets its signals.
        let mut sink = [0_u8; 64];

        loop {
            match reader.read(&mut sink) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }

        let _ = events.send(Event::DemandClosed(id));
    }

    connections
        .lock()
        .expect("demand connection lock poisoned")
        .remove(&id);
}


// -----------------------------------------------------------------------------
// Helper side (run by MediaMTX runOnDemand)
// -----------------------------------------------------------------------------

pub fn run_signal() -> Result<(), Box<dyn std::error::Error>> {
    let address =
        env::var(ADDRESS_ENV)
            .map_err(|_| format!("{ADDRESS_ENV} is not set; the demand signal must be started by the PGC Host's MediaMTX."))?;

    let mut connection =
        TcpStream::connect(address.parse::<SocketAddr>()?)?;

    connection.write_all(format!("{HELLO}\n").as_bytes())?;

    // Hold the connection until MediaMTX terminates this process or the Host
    // goes away.
    let mut sink = [0_u8; 64];

    loop {
        match connection.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::mpsc;


    fn expect(events: &mpsc::Receiver<Event>) -> Event {
        events
            .recv_timeout(Duration::from_secs(5))
            .expect("expected a demand event")
    }


    #[test]
    fn connection_lifetime_is_demand_lifetime() {
        let (events_tx, events_rx) = mpsc::channel();

        let listener = listen(events_tx).unwrap();

        let mut helper = TcpStream::connect(listener.address()).unwrap();

        helper.write_all(format!("{HELLO}\n").as_bytes()).unwrap();

        let Event::DemandOpened(opened) = expect(&events_rx) else {
            panic!("expected DemandOpened");
        };

        drop(helper);

        let Event::DemandClosed(closed) = expect(&events_rx) else {
            panic!("expected DemandClosed");
        };

        assert_eq!(opened, closed);
    }


    #[test]
    fn reset_ends_demand_and_releases_helper() {
        let (events_tx, events_rx) = mpsc::channel();

        let listener = listen(events_tx).unwrap();

        let mut helper = TcpStream::connect(listener.address()).unwrap();

        helper.write_all(format!("{HELLO}\n").as_bytes()).unwrap();

        assert!(matches!(expect(&events_rx), Event::DemandOpened(_)));

        listener.reset();

        assert!(matches!(expect(&events_rx), Event::DemandClosed(_)));

        // The helper's blocking read ends, so the helper process would exit.
        helper.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let mut sink = [0_u8; 8];

        assert!(matches!(helper.read(&mut sink), Ok(0) | Err(_)));
    }


    #[test]
    fn unrelated_connections_are_not_demand() {
        let (events_tx, events_rx) = mpsc::channel();

        let listener = listen(events_tx).unwrap();

        let mut stranger = TcpStream::connect(listener.address()).unwrap();

        stranger.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();

        drop(stranger);

        assert!(events_rx.recv_timeout(Duration::from_millis(500)).is_err());
    }
}
