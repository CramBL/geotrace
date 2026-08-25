//! The transport the four environment schedulers fetch archive days on.

use std::sync::Arc;
use std::time::Duration;

use gt_fetch::{Connection, OfflineTransport, TransportSource};
use gt_store::EnvironmentArchive;

/// The transport one scheduler's day fetches run on, connected on the first
/// fetch and kept until the host changes.
pub struct DayFetchTransport {
    /// Nothing here decides whether requests may leave the machine: the
    /// application supplies this.
    source: TransportSource,
    pacing: Option<Duration>,
    /// Used only in the log line for a failed [`TransportSource::connect`].
    archive: EnvironmentArchive,
    connection: Option<Arc<Connection>>,
}

impl DayFetchTransport {
    /// A transport that sleeps the calling thread to keep its sends `interval`
    /// apart.
    pub fn paced(source: TransportSource, interval: Duration, archive: EnvironmentArchive) -> Self {
        Self {
            source,
            pacing: Some(interval),
            archive,
            connection: None,
        }
    }

    /// A transport that does not pace its sends, for a scheduler that spaces
    /// its own dispatches.
    pub fn unpaced(source: TransportSource, archive: EnvironmentArchive) -> Self {
        Self {
            source,
            pacing: None,
            archive,
            connection: None,
        }
    }

    /// The open connection, connecting one on the first call.
    ///
    /// On a failed [`TransportSource::connect`], this logs the error and
    /// returns [`Connection::Offline`] without storing it: the day fails
    /// through the worker like any other failure, and the next call attempts
    /// [`TransportSource::connect`] again.
    ///
    /// [`super::snap::SnapScheduler`] does not fetch through this type. It
    /// records a failed [`TransportSource::connect`] as
    /// [`super::snap::SnapActivity::Failed`] on the track's row.
    pub fn connect_or_offline(&mut self) -> Arc<Connection> {
        if let Some(connection) = self.connection.as_ref() {
            return Arc::clone(connection);
        }
        match self.source.connect(self.pacing) {
            Ok(connection) => {
                let connection = Arc::new(connection);
                self.connection = Some(Arc::clone(&connection));
                connection
            }
            Err(err) => {
                log::error!(
                    "The {} transport is unavailable: {err}",
                    self.archive.label_in_sentence()
                );
                Arc::new(Connection::Offline(OfflineTransport))
            }
        }
    }

    /// The next [`Self::connect_or_offline`] builds a connection pool and
    /// pacing state for the changed host.
    pub fn drop_the_connection(&mut self) {
        self.connection = None;
    }
}

#[cfg(test)]
mod tests {
    use gt_ui_types::ArcIdentity;

    use super::*;

    fn transport() -> DayFetchTransport {
        DayFetchTransport::paced(
            TransportSource::Offline,
            Duration::from_secs(1),
            EnvironmentArchive::AircraftInterference,
        )
    }

    #[test]
    fn every_fetch_shares_one_connection_until_it_is_dropped() {
        let mut transport = transport();
        let first = transport.connect_or_offline();
        let second = transport.connect_or_offline();
        assert_eq!(ArcIdentity::of(&first), ArcIdentity::of(&second));

        transport.drop_the_connection();
        let after_drop = transport.connect_or_offline();
        assert_ne!(ArcIdentity::of(&first), ArcIdentity::of(&after_drop));
    }
}
