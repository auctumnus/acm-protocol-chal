use rand::seq::SliceRandom;
use std::{
    ffi::OsString,
    io::ErrorKind,
    net::SocketAddr,
    os::unix::prelude::OsStrExt,
    time::{Duration, SystemTime},
};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

use clap::Parser;

/// ACM protocol challenge for the spring semester CTF
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port for the server to listen on
    #[arg(short, long)]
    port: u16,

    /// Flag to give the user on challenge completion. If not present, assumed to be provided
    /// in a `FLAG` environment variable.
    #[arg(short, long)]
    flag: Option<OsString>,
}

/// Keywords for the challenge.
const WORDS: [&str; 32] = [
    "sky", "lichen", "window", "road", "wall", "hill", "sand", "soil", "loam", "sun", "star",
    "root", "rain", "hand", "green", "blue", "red", "steam", "steel", "leaf", "house", "brush",
    "stair", "flower", "log", "vase", "painting", "cottage", "frog", "stone", "pond", "river",
];

/// Maximum tries to perform IO on the socket.
const MAX_TRIES: usize = 100;

/// Read a message from the socket, placing the result into the buffer.
/// The buffer will be cleared before the message is placed into it.
/// ## Arguments
///  - `socket`: reference to a readable tcp stream
///  - `buf`: mutable buffer to place the data in
/// ## Returns
/// `Ok(n)` with `n` being the number of bytes read on success, or an Error on
/// failure. The function will retry up to `MAX_TRIES` to read from the socket.
async fn read_message(socket: &TcpStream, buf: &mut Vec<u8>) -> Result<usize, io::Error> {
    buf.clear();
    let mut readable_tries = 0;
    let mut read_tries = 0;
    loop {
        match socket.readable().await {
            Ok(_) => {}
            Err(e) => {
                if readable_tries > MAX_TRIES {
                    return Err(e);
                }
                readable_tries += 1;
                continue;
            }
        };
        match socket.try_read_buf(buf) {
            Ok(n) if n == 0 => break Err(ErrorKind::BrokenPipe.into()),
            Ok(n) => break Ok(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => {
                eprintln!("failed to read from socket: {e}");
                if read_tries > MAX_TRIES {
                    eprintln!("tried to read {read_tries} times and failed");
                    return Err(e);
                }
                read_tries += 1;
            }
        };
    }
}

/// Write a message to the socket.
/// ## Arguments
///  - `socket`: reference to a writable tcp stream
///  - `buf`: buffer of data to be written
/// ## Returns
/// `Ok(())` on success, or an Error on failure. The function will retry up to
/// `MAX_TRIES` to send the message.
async fn write_message(socket: &TcpStream, buf: &[u8]) -> Result<(), io::Error> {
    let buf_len = buf.len();
    let mut position = 0;
    let mut tries = 0;
    loop {
        match socket.writable().await {
            Ok(_) => {}
            Err(_) => continue,
        };

        match socket.try_write(&buf[position..buf_len]) {
            Ok(n) => {
                if (position + n) == buf_len {
                    break Ok(());
                }
                position += n;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => {
                if tries > MAX_TRIES {
                    eprintln!("failed to write to socket: {e}");
                    break Err(e);
                }
                tries += 1;
            }
        }
    }
}

fn took_too_long(start_time: SystemTime) -> bool {
    start_time
        .elapsed()
        .unwrap_or(Duration::default())
        .as_secs()
        > 5
}

fn correct_response(word: &str) -> &str {
    // why god do i have to do .as_bytes()
    // why is &&&str not just &str
    let index = WORDS
        .iter()
        .enumerate()
        .find(|(_, w)| word == **w)
        .map(|(i, _)| i)
        .expect("couldn't find word in word vec");
    let response_index = (index + 3) % WORDS.len();
    WORDS[response_index]
}

async fn handle_connection(
    flag: &OsString,
    socket: &TcpStream,
    addr: &SocketAddr,
) -> Result<(), io::Error> {
    println!("received connection: {addr}");

    let mut rng = rand::thread_rng();
    let start_time = SystemTime::now();
    let mut keywords = WORDS;
    keywords.shuffle(&mut rng);

    let mut buf = vec![];
    read_message(socket, &mut buf).await?;

    // check for client hello
    if buf.starts_with(b"hello") {
        write_message(socket, b"hello! let's play a game :3\n").await?;
    } else {
        write_message(socket, b"that's not a nice greeting...\n").await?;
        return Ok(());
    }

    read_message(socket, &mut buf).await?;

    if !buf.starts_with(b"ok") {
        write_message(socket, b"okay, we can play later then...").await?;
        return Ok(());
    }

    for i in 0..4 {
        if took_too_long(start_time) {
            write_message(socket, b"you took too long!").await?;
            return Ok(());
        }

        let start = i * 8;
        let end = (i + 1) * 8;
        let keywords = &keywords[start..end];
        let words = keywords.join(" ");
        let words = [words, String::from("\n")].concat(); // add newline

        write_message(socket, words.as_bytes()).await?;
        read_message(socket, &mut buf).await?;

        // SAFETY: lol idc
        let response_words = unsafe { std::str::from_utf8_unchecked(&buf) };
        let response_words = response_words.split(' ');

        let correct_responses = keywords.iter().map(|s| correct_response(s));

        for (ours, theirs) in correct_responses.zip(response_words) {
            if ours != theirs {
                println!("expected {ours}, got {theirs}");
                write_message(socket, b"you said the wrong word!\n").await?;
                return Ok(());
            }
        }
    }
    // SAFETY: i immediately just make this bytes anyways, not like it really
    // matters
    let flag = unsafe { std::str::from_utf8_unchecked(flag.as_bytes()) };
    let win_message = format!("good job! the flag is {flag}\n");
    write_message(socket, win_message.as_bytes()).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), ()> {
    let args = Args::parse();

    let flag = args
        .flag
        .or(std::env::var_os("FLAG"))
        .expect("couldn't get flag (either provide it in `--flag`, or a `FLAG` env var");

    let address = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = TcpListener::bind(address)
        .await
        .unwrap_or_else(|_| panic!("could not bind to {address}, dying"));

    println!("starting server on {}", listener.local_addr().unwrap());

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                handle_connection(&flag, &socket, &addr)
                    .await
                    .unwrap_or_else(|e| eprintln!("handling connection failed: {e}"));
                println!("shutting down connection");
                let shutdown_status = socket
                    .into_std()
                    .map(|s| s.shutdown(std::net::Shutdown::Both));
                match shutdown_status {
                    Ok(_) => println!("successfully shut down connection"),
                    Err(e) => eprintln!("failed to shut down connection: {e}"),
                };
            }
            Err(e) => eprintln!("{e}"),
        }
    }
}
