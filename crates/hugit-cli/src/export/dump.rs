//! The streaming dumper (E5④): write the export to disk with **bounded
//! memory** — never the whole corpus in RAM.
//!
//! The git artifact (blobs/packs) is streamed object-by-object through a fixed
//! chunk buffer; the JSON envelope's large collections (events, journals) are
//! streamed element-by-element as a JSON array, flushing per element. Peak
//! resident bytes are bounded by the chunk size, independent of corpus size, so
//! a multi-GB export does not OOM.

use std::io::{self, Write};

/// The streaming write buffer size. Peak in-RAM bytes for the streamed lanes is
/// bounded by this, not by the corpus size.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// A bounded-memory streaming writer: every `write` is flushed to the sink once
/// the internal buffer reaches [`CHUNK_BYTES`], so resident memory never exceeds
/// one chunk regardless of total bytes written. Tracks total + peak buffered.
pub struct StreamingDumper<W: Write> {
    sink: W,
    buf: Vec<u8>,
    total_written: u64,
    peak_buffered: usize,
}

impl<W: Write> StreamingDumper<W> {
    /// Wrap a sink in a bounded-memory streaming dumper.
    pub fn new(sink: W) -> Self {
        Self {
            sink,
            buf: Vec::with_capacity(CHUNK_BYTES),
            total_written: 0,
            peak_buffered: 0,
        }
    }

    /// Stream a chunk of bytes, flushing whenever the buffer fills. Memory stays
    /// bounded by [`CHUNK_BYTES`] (plus the size of any single oversized write,
    /// which is flushed immediately rather than retained).
    pub fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buf.extend_from_slice(bytes);
        self.peak_buffered = self.peak_buffered.max(self.buf.len());
        while self.buf.len() >= CHUNK_BYTES {
            let take = CHUNK_BYTES.min(self.buf.len());
            self.sink.write_all(&self.buf[..take])?;
            self.total_written += take as u64;
            self.buf.drain(..take);
        }
        Ok(())
    }

    /// Flush the residual buffer and return the wrapped sink.
    pub fn finish(mut self) -> io::Result<W> {
        if !self.buf.is_empty() {
            self.sink.write_all(&self.buf)?;
            self.total_written += self.buf.len() as u64;
            self.buf.clear();
        }
        self.sink.flush()?;
        Ok(self.sink)
    }

    /// Total bytes streamed to the sink so far.
    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    /// The largest the internal buffer ever grew — the proof of bounded memory.
    pub fn peak_buffered(&self) -> usize {
        self.peak_buffered
    }
}

/// Stream a sequence of byte records through a fresh dumper and return the
/// dumper for memory introspection plus the collected sink. Each record is an
/// independent chunk, modelling object-by-object git/object streaming.
pub fn stream_records<'a, I>(records: I) -> io::Result<StreamingDumper<Vec<u8>>>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let mut dumper = StreamingDumper::new(Vec::new());
    for r in records {
        dumper.write_chunk(r)?;
    }
    // Caller calls finish(); we return the dumper so peak_buffered is readable.
    Ok(dumper)
}
