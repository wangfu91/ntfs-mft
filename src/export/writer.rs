use super::{ExportError, ExportItem, ExportItemV1, ExportRecordV1};
use crate::{MftError, MftResult, OfflineMftRecord, RawMftEntry};
use std::io::{self, Write};

/// Streaming JSONL writer using at most one item's serialized buffer.
///
/// Preserves call order. No scan collection, sorting, path lookup or implicit flush.
/// A write/flush failure poisons this writer so later items cannot follow a truncated
/// line. Previous complete lines remain usable; discard an incomplete final line.
pub struct JsonlWriter<W: Write> {
    sink: W,
    buffer: Vec<u8>,
    failed: bool,
}

impl<W: Write> JsonlWriter<W> {
    /// Wrap a destination. Use a `BufWriter` if small sink writes are expensive.
    pub fn new(sink: W) -> Self {
        Self {
            sink,
            buffer: Vec::new(),
            failed: false,
        }
    }

    /// Write a complete versioned item followed by one LF byte.
    pub fn write_item(&mut self, item: &ExportItem) -> MftResult<()> {
        self.check()?;
        self.buffer.clear();
        serde_json::to_writer(&mut self.buffer, item)?;
        self.buffer.push(b'\n');
        if let Err(error) = self.sink.write_all(&self.buffer) {
            self.failed = true;
            return Err(error.into());
        }
        Ok(())
    }

    /// Convert and write an existing scan entry without retaining it.
    pub fn write_entry(&mut self, entry: &RawMftEntry) -> MftResult<()> {
        self.write_record(ExportRecordV1::from_entry(entry))
    }

    /// Convert and write a raw record with diagnostic metadata evidence.
    pub fn write_diagnostic(&mut self, record: &crate::RawMftDiagnosticRecord) -> MftResult<()> {
        self.write_record(ExportRecordV1::from_diagnostic(record))
    }

    /// Convert and write an offline record with presence states and diagnostics.
    pub fn write_offline(&mut self, record: &OfflineMftRecord) -> MftResult<()> {
        self.write_record(ExportRecordV1::from_offline(record))
    }

    /// Write a caller-customized record DTO, for example after supplying path evidence.
    pub fn write_record(&mut self, record: ExportRecordV1) -> MftResult<()> {
        self.write_item(&ExportItem::V1(ExportItemV1::Record(Box::new(record))))
    }

    /// Preserve an item-level error as its own line. No record slot is guessed.
    pub fn write_error(&mut self, error: &MftError) -> MftResult<()> {
        self.write_item(&ExportItem::V1(ExportItemV1::Error(ExportError::from(
            error,
        ))))
    }

    /// Flush the sink and report errors. Call this before declaring export complete.
    pub fn flush(&mut self) -> MftResult<()> {
        self.check()?;
        self.sink.flush().inspect_err(|_| self.failed = true)?;
        Ok(())
    }

    /// Flush successfully before returning ownership of the destination.
    pub fn finish(mut self) -> MftResult<W> {
        self.flush()?;
        Ok(self.sink)
    }

    fn check(&self) -> MftResult<()> {
        if self.failed {
            return Err(
                io::Error::other("JSONL writer is unusable after an output failure").into(),
            );
        }
        Ok(())
    }
}
