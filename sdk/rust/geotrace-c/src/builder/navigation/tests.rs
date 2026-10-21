#![expect(
    clippy::panic_in_result_fn,
    reason = "test assertions and error propagation"
)]

use std::error::Error;

use geotrace_sdk::{
    Angle, Constellation, NavFile, NavFix, PositionProvenance, SatelliteCoverage,
    SatelliteIdentity, SatelliteMembership, SatelliteObservation, SatelliteScope, SatelliteUsage,
    SolutionDimension, SolutionMethod, SolutionState,
};

use super::{GtdFileBuilder, GtdStatus, TimestampArguments};
use crate::navigation;
use crate::{
    GtdConstellation, GtdOptF32, GtdOptU32, GtdSatelliteObservation, GtdSatelliteReportInput,
    GtdSatelliteScope, GtdSatelliteUsage,
};

#[test]
fn c_ingestion_preserves_canonical_navigation_and_report_values() -> Result<(), Box<dyn Error>> {
    let mut builder = GtdFileBuilder::new();
    let mut fix = navigation::gtd_nav_fix_v2_init();
    fix.sys_time.unix_micros = 1_700_000_000_000_000;
    fix.state = 2;
    fix.method = 4;
    fix.dimension = 2;
    fix.used_count = Some(12).into();
    fix.hdop = Some(0.9).into();
    let source_time = TimestampArguments {
        gps_time: fix.gps_time,
        sys_time: fix.sys_time,
    }
    .into_nav_fix_time("test")
    .map_err(|status| format!("{status:?}"))?;
    let report = GtdSatelliteReportInput {
        gps_time: fix.gps_time,
        sys_time: fix.sys_time,
        scope: GtdSatelliteScope {
            kind: 0,
            constellation_mask: 0,
        },
        coverage: 1,
        reported_count: GtdOptU32::from(Some(8)),
    };
    let usage = [
        GtdSatelliteUsage {
            constellation: GtdConstellation::Gps as u32,
            prn: 1,
            membership: 0,
        },
        GtdSatelliteUsage {
            constellation: GtdConstellation::Gps as u32,
            prn: 1,
            membership: 2,
        },
    ];
    let observations = [GtdSatelliteObservation {
        constellation: GtdConstellation::Gps as u32,
        prn: 1,
        elevation_deg: GtdOptF32 {
            value: 0.0,
            present: 0,
        },
        azimuth_deg: GtdOptF32 {
            value: 0.0,
            present: 1,
        },
        snr_dbhz: GtdOptF32 {
            value: 0.0,
            present: 1,
        },
    }];
    // SAFETY: the builder and fix remain live for the call.
    let status = unsafe { super::gtd_builder_add_nav_fix_v2(&mut builder, &fix) };
    assert_eq!(status, GtdStatus::GTD_OK);
    // SAFETY: the builder, metadata and usage array remain live for the call.
    let status = unsafe {
        super::gtd_builder_add_satellite_usage_report(
            &mut builder,
            &report,
            usage.as_ptr(),
            usage.len(),
        )
    };
    assert_eq!(status, GtdStatus::GTD_OK);
    // SAFETY: the builder, metadata and observation array remain live for the call.
    let status = unsafe {
        super::gtd_builder_add_satellite_observation_report(
            &mut builder,
            &report,
            observations.as_ptr(),
            observations.len(),
        )
    };
    assert_eq!(status, GtdStatus::GTD_OK);
    let file = builder.into_recorder().finish()?;
    let mut bytes = Vec::new();
    file.write(&mut bytes)?;
    let decoded = NavFile::read(bytes.as_slice())?;
    let point = decoded.nav_points().first().ok_or("missing point")?;
    assert_eq!(
        point.fix,
        NavFix::builder()
            .time(source_time)
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .state(SolutionState::NoFix)
            .method(SolutionMethod::RtkFixed)
            .dimension(SolutionDimension::ThreeD)
            .used_count(12)
            .hdop(0.9)
            .position_provenance(Some(PositionProvenance::SourceProvided))
            .build()
    );
    let evidence = point.satellite_evidence();
    let usage_report = evidence.usage.first().ok_or("missing usage")?;
    assert_eq!(usage_report.scope, SatelliteScope::Global);
    assert_eq!(usage_report.coverage, SatelliteCoverage::Partial);
    assert_eq!(usage_report.reported_count, Some(8));
    assert_eq!(usage_report.time, source_time);
    let identity = SatelliteIdentity::new(Constellation::Gps, 1);
    assert_eq!(
        usage_report.satellites,
        [
            SatelliteUsage::new(identity, SatelliteMembership::Unknown),
            SatelliteUsage::new(identity, SatelliteMembership::Used)
        ]
    );
    let observation_report = evidence.observations.first().ok_or("missing observation")?;
    assert_eq!(
        observation_report.satellites,
        [SatelliteObservation::builder()
            .identity(identity)
            .azimuth(0.0)
            .snr(0.0)
            .build()]
    );
    assert_eq!(
        usage_report
            .placement
            .ok_or("missing placement")?
            .input_order,
        0
    );
    assert_eq!(
        observation_report
            .placement
            .ok_or("missing placement")?
            .input_order,
        1
    );
    assert_eq!(point.satellites(), &None);
    Ok(())
}
