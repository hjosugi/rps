//! #6: what talking to a warm sidecar process (tier T2 of DESIGN.md §5)
//! costs per request, before the sidecar does any work.
//!
//! A request is a frame — a little-endian `u32` length, then the payload —
//! and the server answers with the same frame. Sizes: 64 B (a member call
//! with a few scalar arguments), 4 KiB (a small object graph), 64 KiB (a
//! pipeline batch). Candidates:
//!
//! * thread: the server is a thread of this process, behind a Unix domain
//!   socket. The floor of the transport.
//! * process: the server is a separate process, this bench binary re-run with
//!   `--ipc-echo-server`, as a sidecar would be.
//! * process, connect each time: a new connection per request, as an rpsh
//!   process that talks to the sidecar once would pay.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use rpsh_research::{Report, iterations, measure};

const SERVER_FLAG: &str = "--ipc-echo-server";
const SIZES: [(usize, &str, u64); 3] = [
    (64, "64 B", 20_000),
    (4 * 1024, "4 KiB", 20_000),
    (64 * 1024, "64 KiB", 5_000),
];

/// Answers every frame on every connection with the same frame.
fn serve(listener: UnixListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { return };
        let _ = echo(&mut stream);
    }
}

fn echo(stream: &mut UnixStream) -> io::Result<()> {
    let mut frame = Vec::new();
    loop {
        let mut header = [0u8; 4];
        match stream.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let length = u32::from_le_bytes(header) as usize;
        frame.clear();
        frame.extend_from_slice(&header);
        frame.resize(4 + length, 0);
        stream.read_exact(&mut frame[4..])?;
        stream.write_all(&frame)?;
    }
}

fn frame(payload_length: usize) -> Vec<u8> {
    let mut frame = (payload_length as u32).to_le_bytes().to_vec();
    frame.extend((0..payload_length).map(|index| index as u8));
    frame
}

/// Sends `request` (a whole frame) and reads the answer into `response`.
fn round_trip(stream: &mut UnixStream, request: &[u8], response: &mut Vec<u8>) -> usize {
    stream.write_all(request).expect("write request");
    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .expect("read response header");
    let length = u32::from_le_bytes(header) as usize;
    response.resize(length, 0);
    stream.read_exact(response).expect("read response");
    length
}

fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rpsh-ipc-{}-{name}.sock", std::process::id()))
}

fn bind(path: &Path) -> UnixListener {
    let _ = fs::remove_file(path);
    UnixListener::bind(path).unwrap_or_else(|error| panic!("bind {}: {error}", path.display()))
}

/// The sidecar stand-in: bind, say "ready", serve until killed.
fn server_main(path: &str) {
    let listener = bind(Path::new(path));
    println!("ready");
    let _ = io::stdout().flush();
    serve(listener);
}

fn start_server_process(path: &Path) -> Child {
    let mut child = Command::new(std::env::current_exe().expect("current executable"))
        .arg(SERVER_FLAG)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the server process");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("server stdout"))
        .read_line(&mut line)
        .expect("read the server's ready line");
    assert_eq!(line.trim(), "ready", "the server process did not start");
    child
}

fn measure_persistent(report: &mut Report, candidate: &str, path: &Path) {
    let mut stream = UnixStream::connect(path).expect("connect");
    let mut response = Vec::new();
    for (size, label, count) in SIZES {
        let request = frame(size);
        report.add(measure(
            &format!("round trip {label}"),
            candidate,
            iterations(count),
            || round_trip(&mut stream, &request, &mut response),
        ));
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() >= 3 && arguments[1] == SERVER_FLAG {
        server_main(&arguments[2]);
        return;
    }

    let mut report = Report::new(
        "dotnet_ipc",
        "request/response over a Unix domain socket: the transport cost of a T2 sidecar (#6)",
    );
    report.header();

    let thread_path = socket_path("thread");
    let listener = bind(&thread_path);
    thread::spawn(move || serve(listener));
    measure_persistent(&mut report, "thread", &thread_path);

    let process_path = socket_path("process");
    let mut server = start_server_process(&process_path);
    measure_persistent(&mut report, "process", &process_path);

    let request = frame(64);
    let mut response = Vec::new();
    report.add(measure(
        "connect + round trip 64 B",
        "process, connect each time",
        iterations(5_000),
        || {
            let mut stream = UnixStream::connect(&process_path).expect("connect");
            round_trip(&mut stream, &request, &mut response)
        },
    ));

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&thread_path);
    let _ = fs::remove_file(&process_path);
    report.save();
}
