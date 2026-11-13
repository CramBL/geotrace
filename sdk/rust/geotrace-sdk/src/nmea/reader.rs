use std::io::BufRead;

use crate::NavFileBuilder;

use super::{NmeaDiagnosticKind, NmeaError, NmeaIngestor, NmeaRecording};

impl NmeaIngestor {
    /// Read bounded input lines and construct a recording with the supplied builder.
    /// The recorder retains generated records. Input I/O, missing navigation timelines and
    /// recording construction return distinct [`NmeaError`] variants.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use std::time::Duration;
    ///
    /// use geotrace_sdk::{NavFileBuilder, NmeaIngestor, SentenceType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let source = Cursor::new(concat!(
    ///     "2026-10-23 120000.000 ",
    ///     "$GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A*52\n",
    /// ));
    /// let result = NmeaIngestor::builder()
    ///     .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
    ///     .sentence_period(SentenceType::RMC, Duration::from_secs(1))
    ///     .build()?
    ///     .ingest_reader(source, NavFileBuilder::new())?;
    /// let mut gtd = Vec::new();
    /// result.recording.write(&mut gtd)?;
    /// assert_eq!(result.recording.nav_points().len(), 1);
    /// assert!(result.diagnostics.samples().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn ingest_reader(
        mut self,
        mut source: impl BufRead,
        builder: NavFileBuilder,
    ) -> Result<NmeaRecording, NmeaError> {
        let mut recorder = builder.open();
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            let mut consumed = false;
            let mut too_long = false;
            loop {
                let available = source.fill_buf().map_err(|source| NmeaError::Input {
                    line: self.statistics.lines.saturating_add(1),
                    source,
                })?;
                if available.is_empty() {
                    break;
                }
                consumed = true;
                let end = available
                    .iter()
                    .position(|byte| matches!(*byte, b'\n' | b'\r'))
                    .map(|index| index + 1);
                let count = end.unwrap_or(available.len());
                let carriage_return = end.is_some_and(|end| available.get(end - 1) == Some(&b'\r'));
                let remaining = self
                    .config
                    .max_line_bytes
                    .get()
                    .saturating_sub(buffer.len());
                buffer.extend(available.iter().take(count.min(remaining)));
                too_long |= count > remaining;
                source.consume(count);
                if end.is_some() {
                    if carriage_return {
                        let following = source.fill_buf().map_err(|source| NmeaError::Input {
                            line: self.statistics.lines.saturating_add(1),
                            source,
                        })?;
                        if following.first() == Some(&b'\n') {
                            if buffer.len() < self.config.max_line_bytes.get() {
                                buffer.push(b'\n');
                            } else {
                                too_long = true;
                            }
                            source.consume(1);
                        }
                    }
                    break;
                }
            }
            if !consumed {
                break;
            }
            if too_long {
                self.statistics.lines = self.statistics.lines.saturating_add(1);
                self.report(
                    NmeaDiagnosticKind::LineTooLong,
                    None,
                    "Input line exceeds the configured byte limit",
                );
                continue;
            }
            match std::str::from_utf8(&buffer) {
                Ok(line) => {
                    for record in self.push_line(line) {
                        recorder.add(record);
                    }
                }
                Err(error) => {
                    self.statistics.lines = self.statistics.lines.saturating_add(1);
                    self.report(
                        NmeaDiagnosticKind::InvalidText,
                        None,
                        format!("Input line is not UTF-8: {error}"),
                    );
                }
            }
        }
        let completion = self.finish();
        for record in completion.records {
            recorder.add(record);
        }
        if completion.statistics.navigation_fixes == 0 {
            return Err(NmeaError::MissingNavigationTimeline {
                diagnostics: completion.diagnostics,
                statistics: completion.statistics,
            });
        }
        let recording = recorder.finish().map_err(NmeaError::Recording)?;
        Ok(NmeaRecording {
            recording,
            diagnostics: completion.diagnostics,
            statistics: completion.statistics,
        })
    }
}
