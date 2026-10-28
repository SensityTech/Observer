use std::fs::File;
use std::io::{self, Read, Write};
use std::thread;

use tokio::sync::{mpsc, oneshot};

const READ_CHUNK: usize = 64 * 1024;
/// Enough to absorb a burst of console output while the consumer catches up. A consumer that
/// stalls for longer slows the reader down, and in turn the process writing.
const QUEUED_CHUNKS: usize = 256;

/// Bytes the process writes to one of its pipes, read by a dedicated thread: the pipes are
/// synchronous on Windows, and a thread per pipe keeps blocking reads off the async runtime.
pub struct Output {
    chunks: mpsc::Receiver<Vec<u8>>,
}

impl Output {
    pub(crate) fn read(name: &str, mut pipe: File) -> io::Result<Self> {
        let (sender, chunks) = mpsc::channel(QUEUED_CHUNKS);
        thread::Builder::new().name(name.to_owned()).spawn(move || {
            let mut buffer = vec![0; READ_CHUNK];
            let mut forwarding = true;
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    // Once nobody listens, keep draining: a full pipe would block the writer.
                    Ok(read) if forwarding => {
                        forwarding = sender.blocking_send(buffer[..read].to_vec()).is_ok();
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { chunks })
    }

    /// The next bytes, in the order the process wrote them, or `None` once every process holding
    /// the pipe has closed it.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        self.chunks.recv().await
    }
}

type PendingWrite = (Vec<u8>, oneshot::Sender<io::Result<()>>);

/// The process's stdin, written by a dedicated thread so that a process that stops reading never
/// blocks the caller: each write resolves once the pipe took the bytes, and can be timed out.
/// Dropping it closes the pipe.
pub struct Stdin {
    writes: mpsc::Sender<PendingWrite>,
}

impl Stdin {
    pub(crate) fn write_to(mut pipe: File) -> io::Result<Self> {
        let (writes, mut queue) = mpsc::channel::<PendingWrite>(16);
        thread::Builder::new()
            .name("process-stdin".to_owned())
            .spawn(move || {
                while let Some((bytes, done)) = queue.blocking_recv() {
                    let result = pipe.write_all(&bytes).and_then(|()| pipe.flush());
                    let failed = result.is_err();
                    let _ = done.send(result);
                    if failed {
                        break;
                    }
                }
            })?;
        Ok(Self { writes })
    }

    pub async fn write(&self, bytes: Vec<u8>) -> io::Result<()> {
        let (done, written) = oneshot::channel();
        self.writes.send((bytes, done)).await.map_err(|_| closed())?;
        written.await.map_err(|_| closed())?
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the process's stdin is closed")
}
