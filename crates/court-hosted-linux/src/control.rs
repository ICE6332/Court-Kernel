use crate::LinuxResult;
use crate::protocol::WireMessage;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::Duration;

pub struct JsonLinePeer {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl JsonLinePeer {
    pub fn new(stream: UnixStream) -> LinuxResult<Self> {
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            reader,
            writer: stream,
        })
    }

    pub fn connect_with_retry(path: &Path) -> LinuxResult<Self> {
        let mut last_error = None;
        for _ in 0..100 {
            match UnixStream::connect(path) {
                Ok(stream) => return Self::new(stream),
                Err(error) => {
                    last_error = Some(error);
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
        Err(format!(
            "failed to connect to {}: {}",
            path.display(),
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no attempts made".to_string())
        )
        .into())
    }

    pub fn send(&mut self, message: &WireMessage) -> LinuxResult<()> {
        serde_json::to_writer(&mut self.writer, message)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn recv(&mut self) -> LinuxResult<WireMessage> {
        let mut line = String::new();
        let bytes = self.reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err("control channel closed".into());
        }
        let message = serde_json::from_str(line.trim_end())?;
        Ok(message)
    }
}
