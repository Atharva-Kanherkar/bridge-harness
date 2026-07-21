use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::Shutdown,
    thread,
};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(unix)]
fn main() -> io::Result<()> {
    let socket_path = std::env::var_os("BRIDGE_BROWSER_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("dev.bridge.deck.browser.sock"));
    let stream = UnixStream::connect(socket_path)?;
    let read_stream = stream.try_clone()?;
    let write_stream = stream.try_clone()?;
    let browser_to_bridge = thread::spawn(move || -> io::Result<()> {
        let mut stdin = io::stdin().lock();
        let mut socket = write_stream;
        loop {
            let mut length = [0_u8; 4];
            if stdin.read_exact(&mut length).is_err() {
                break;
            }
            let length = u32::from_le_bytes(length) as usize;
            if length > 8 * 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native message exceeds 8 MiB",
                ));
            }
            let mut payload = vec![0_u8; length];
            stdin.read_exact(&mut payload)?;
            socket.write_all(&payload)?;
            socket.write_all(b"\n")?;
            socket.flush()?;
        }
        let _ = socket.shutdown(Shutdown::Write);
        Ok(())
    });
    let bridge_to_browser = thread::spawn(move || -> io::Result<()> {
        let reader = BufReader::new(read_stream);
        let mut stdout = io::stdout().lock();
        for line in reader.lines() {
            let payload = line?.into_bytes();
            stdout.write_all(&(payload.len() as u32).to_le_bytes())?;
            stdout.write_all(&payload)?;
            stdout.flush()?;
        }
        Ok(())
    });
    browser_to_bridge
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("browser relay panicked")))?;
    bridge_to_browser
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("Bridge relay panicked")))
}

#[cfg(not(unix))]
fn main() {
    eprintln!("Bridge browser native messaging is currently supported on Unix platforms");
}
