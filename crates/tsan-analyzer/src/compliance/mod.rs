mod atsc_a65;
mod china_dtmb;
mod isdb_arib;
mod table_rules;

use std::collections::BTreeSet;

use crate::tr101290::{self, Tr101290Rules};
use crate::transport::{AnalysisReport, BroadcastStandard, SignalledModulation, TrEvent};

use table_rules::ProfileCheckReport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComplianceStatus {
    Pass,
    Fail,
    NotObserved,
    NotApplicable,
}

impl ComplianceStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Fail => "Fail",
            Self::NotObserved => "Not observed",
            Self::NotApplicable => "Not applicable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardReference {
    pub code: &'static str,
    pub title: &'static str,
    pub scope: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComplianceFamily {
    Mpeg,
    Dvb,
    Atsc,
    NorthAmericanCable,
    JapaneseCable,
    Isdb,
    Sbtvd,
    ChinaDtv,
}

impl ComplianceFamily {
    pub const ALL: [Self; 8] = [
        Self::Mpeg,
        Self::Dvb,
        Self::Atsc,
        Self::NorthAmericanCable,
        Self::JapaneseCable,
        Self::Isdb,
        Self::Sbtvd,
        Self::ChinaDtv,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mpeg => "MPEG",
            Self::Dvb => "DVB",
            Self::Atsc => "ATSC",
            Self::NorthAmericanCable => "North American Cable",
            Self::JapaneseCable => "Japanese Cable",
            Self::Isdb => "ISDB",
            Self::Sbtvd => "SBTVD / ISDB-T International",
            Self::ChinaDtv => "China DTV",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComplianceSystem {
    Mpeg2TransportStream,
    DvbMpegTs,
    Atsc1,
    J83B,
    J83C,
    IsdbTJapan,
    IsdbTInternational, // i.e. SBTVD / ISDB-Tb
    Dtmb,
}

impl ComplianceSystem {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mpeg2TransportStream => "MPEG-2 Transport Stream",
            Self::DvbMpegTs => "DVB over MPEG-2 TS",
            Self::Atsc1 => "ATSC 1.0",
            Self::J83B => "ITU-T J.83 Annex B",
            Self::J83C => "ITU-T J.83 Annex C",
            Self::IsdbTJapan => "ISDB-T Japan",
            Self::IsdbTInternational => "ISDB-T International",
            Self::Dtmb => "DTMB",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComplianceHierarchy {
    pub family: ComplianceFamily,
    pub system: ComplianceSystem,
    pub signaling: &'static str,
    pub delivery: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComplianceProfile {
    MpegPsiOnly,
    DvbSi,
    DvbT,
    DvbT2,
    DvbCAnnexA,              // i.e. DVB-C / ITU-T J.83 Annex A
    Atsc1A65C2006,           // i.e. ATSC 1.0 / A/65C:2006
    Atsc1A65_2013,           // i.e. ATSC 1.0 / A/65:2013
    J83BAtscCablePsip,       // i.e. ITU-T J.83 Annex B / constellation unknown
    J83BQam64AtscCablePsip,  // i.e. ITU-T J.83 Annex B / 64-QAM
    J83BQam256AtscCablePsip, // i.e. ITU-T J.83 Annex B / 256-QAM
    J83CQam64AribSi,         // i.e. ITU-T J.83 Annex C / Japanese digital cable
    J83CQam256AribSi,        // i.e. ITU-T J.83 Annex C / Japanese digital cable
    IsdbTJapan,
    IsdbTInternational, // i.e. SBTVD / ISDB-Tb
    ChinaDtvSi,         // i.e. DTMB / GB/T 28161 SI
}

impl ComplianceProfile {
    pub const ALL: [Self; 15] = [
        Self::MpegPsiOnly,
        Self::DvbSi,
        Self::DvbT,
        Self::DvbT2,
        Self::DvbCAnnexA,
        Self::Atsc1A65C2006,
        Self::Atsc1A65_2013,
        Self::J83BAtscCablePsip,
        Self::J83BQam64AtscCablePsip,
        Self::J83BQam256AtscCablePsip,
        Self::J83CQam64AribSi,
        Self::J83CQam256AribSi,
        Self::IsdbTJapan,
        Self::IsdbTInternational,
        Self::ChinaDtvSi,
    ];

    pub fn suggested(report: &AnalysisReport) -> Self {
        match report.standard {
            BroadcastStandard::DvbSi => Self::DvbSi,
            BroadcastStandard::AtscPsip => Self::Atsc1A65_2013,
            BroadcastStandard::AtscCablePsip => match report.signalled_modulation {
                SignalledModulation::Qam64 => Self::J83BQam64AtscCablePsip,
                SignalledModulation::Qam256 => Self::J83BQam256AtscCablePsip,
                _ => Self::J83BAtscCablePsip,
            },
            BroadcastStandard::Isdb => Self::IsdbTJapan,
            BroadcastStandard::Dtmb => Self::ChinaDtvSi,
            BroadcastStandard::Scte | BroadcastStandard::Unknown => Self::MpegPsiOnly,
        }
    }

    pub const fn hierarchy(self) -> ComplianceHierarchy {
        match self {
            Self::MpegPsiOnly => ComplianceHierarchy {
                family: ComplianceFamily::Mpeg,
                system: ComplianceSystem::Mpeg2TransportStream,
                signaling: "MPEG PSI",
                delivery: "Delivery unknown from TS",
            },
            Self::DvbSi => ComplianceHierarchy {
                family: ComplianceFamily::Dvb,
                system: ComplianceSystem::DvbMpegTs,
                signaling: "DVB-SI",
                delivery: "Delivery unknown from TS",
            },
            Self::DvbT => ComplianceHierarchy {
                family: ComplianceFamily::Dvb,
                system: ComplianceSystem::DvbMpegTs,
                signaling: "DVB-SI",
                delivery: "DVB-T (profile selection)",
            },
            Self::DvbT2 => ComplianceHierarchy {
                family: ComplianceFamily::Dvb,
                system: ComplianceSystem::DvbMpegTs,
                signaling: "DVB-SI",
                delivery: "DVB-T2 (profile selection)",
            },
            Self::DvbCAnnexA => ComplianceHierarchy {
                family: ComplianceFamily::Dvb,
                system: ComplianceSystem::DvbMpegTs,
                signaling: "DVB-SI",
                delivery: "DVB-C / J.83 Annex A (profile selection)",
            },
            Self::Atsc1A65C2006 => ComplianceHierarchy {
                family: ComplianceFamily::Atsc,
                system: ComplianceSystem::Atsc1,
                signaling: "A/65C:2006 + Amendment 1 PSIP",
                delivery: "Terrestrial / 8-VSB (signalled, not RF-verified)",
            },
            Self::Atsc1A65_2013 => ComplianceHierarchy {
                family: ComplianceFamily::Atsc,
                system: ComplianceSystem::Atsc1,
                signaling: "A/65:2013 PSIP",
                delivery: "Terrestrial / 8-VSB (signalled, not RF-verified)",
            },
            Self::J83BAtscCablePsip => ComplianceHierarchy {
                family: ComplianceFamily::NorthAmericanCable,
                system: ComplianceSystem::J83B,
                signaling: "ATSC Cable PSIP",
                delivery: "QAM constellation unknown from TS",
            },
            Self::J83BQam64AtscCablePsip => ComplianceHierarchy {
                family: ComplianceFamily::NorthAmericanCable,
                system: ComplianceSystem::J83B,
                signaling: "ATSC Cable PSIP",
                delivery: "64-QAM (signalled, not RF-verified)",
            },
            Self::J83BQam256AtscCablePsip => ComplianceHierarchy {
                family: ComplianceFamily::NorthAmericanCable,
                system: ComplianceSystem::J83B,
                signaling: "ATSC Cable PSIP",
                delivery: "256-QAM (signalled, not RF-verified)",
            },
            Self::J83CQam64AribSi => ComplianceHierarchy {
                family: ComplianceFamily::JapaneseCable,
                system: ComplianceSystem::J83C,
                signaling: "ARIB-compatible SI",
                delivery: "64-QAM (profile selection)",
            },
            Self::J83CQam256AribSi => ComplianceHierarchy {
                family: ComplianceFamily::JapaneseCable,
                system: ComplianceSystem::J83C,
                signaling: "ARIB-compatible SI",
                delivery: "256-QAM (profile selection)",
            },
            Self::IsdbTJapan => ComplianceHierarchy {
                family: ComplianceFamily::Isdb,
                system: ComplianceSystem::IsdbTJapan,
                signaling: "ARIB STD-B10",
                delivery: "ARIB STD-B31 (profile selection)",
            },
            Self::IsdbTInternational => ComplianceHierarchy {
                family: ComplianceFamily::Sbtvd,
                system: ComplianceSystem::IsdbTInternational,
                signaling: "ABNT NBR 15603",
                delivery: "ABNT NBR 15601 (profile selection)",
            },
            Self::ChinaDtvSi => ComplianceHierarchy {
                family: ComplianceFamily::ChinaDtv,
                system: ComplianceSystem::Dtmb,
                signaling: "GB/T 28161",
                delivery: "GB 20600 (profile selection)",
            },
        }
    }

    pub const fn variant_label(self) -> &'static str {
        match self {
            Self::MpegPsiOnly => "PSI only / delivery unknown",
            Self::DvbSi => "DVB-SI / delivery unknown",
            Self::DvbT => "DVB-SI / DVB-T",
            Self::DvbT2 => "DVB-SI / DVB-T2",
            Self::DvbCAnnexA => "DVB-SI / DVB-C (J.83 Annex A)",
            Self::Atsc1A65C2006 => "A/65C:2006 PSIP / terrestrial",
            Self::Atsc1A65_2013 => "A/65:2013 PSIP / terrestrial",
            Self::J83BAtscCablePsip => "ATSC Cable PSIP / QAM unknown",
            Self::J83BQam64AtscCablePsip => "ATSC Cable PSIP / 64-QAM signalled",
            Self::J83BQam256AtscCablePsip => "ATSC Cable PSIP / 256-QAM signalled",
            Self::J83CQam64AribSi => "ARIB SI / 64-QAM",
            Self::J83CQam256AribSi => "ARIB SI / 256-QAM",
            Self::IsdbTJapan => "ARIB SI / ISDB-T",
            Self::IsdbTInternational => "SBTVD SI / ISDB-Tb",
            Self::ChinaDtvSi => "China DTV SI / DTMB",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::MpegPsiOnly => "MPEG › MPEG-2 TS › PSI only",
            Self::DvbSi => "DVB › MPEG-2 TS › DVB-SI › delivery unknown",
            Self::DvbT => "DVB › MPEG-2 TS › DVB-SI › DVB-T",
            Self::DvbT2 => "DVB › MPEG-2 TS › DVB-SI › DVB-T2",
            Self::DvbCAnnexA => "DVB › MPEG-2 TS › DVB-SI › DVB-C / J.83 Annex A",
            Self::Atsc1A65C2006 => "ATSC › 1.0 › A/65C:2006 PSIP › terrestrial",
            Self::Atsc1A65_2013 => "ATSC › 1.0 › A/65:2013 PSIP › terrestrial",
            Self::J83BAtscCablePsip => {
                "North American Cable › J.83B › ATSC Cable PSIP › QAM unknown"
            }
            Self::J83BQam64AtscCablePsip => {
                "North American Cable › J.83B › ATSC Cable PSIP › 64-QAM signalled"
            }
            Self::J83BQam256AtscCablePsip => {
                "North American Cable › J.83B › ATSC Cable PSIP › 256-QAM signalled"
            }
            Self::J83CQam64AribSi => "Japanese Cable › J.83C › ARIB SI › 64-QAM",
            Self::J83CQam256AribSi => "Japanese Cable › J.83C › ARIB SI › 256-QAM",
            Self::IsdbTJapan => "ISDB › ISDB-T Japan › ARIB SI",
            Self::IsdbTInternational => "SBTVD › ISDB-T International › ABNT SI",
            Self::ChinaDtvSi => "China DTV › DTMB › GB/T 28161 SI",
        }
    }

    pub const fn references(self) -> &'static [StandardReference] {
        match self {
            Self::MpegPsiOnly => &MPEG_TS_REFERENCES,
            Self::DvbSi => &DVB_REFERENCES,
            Self::DvbT => &DVB_T_REFERENCES,
            Self::DvbT2 => &DVB_T2_REFERENCES,
            Self::DvbCAnnexA => &DVB_C_REFERENCES,
            Self::Atsc1A65C2006 | Self::Atsc1A65_2013 => &ATSC_REFERENCES,
            Self::J83BAtscCablePsip
            | Self::J83BQam64AtscCablePsip
            | Self::J83BQam256AtscCablePsip => &ATSC_CABLE_REFERENCES,
            Self::J83CQam64AribSi | Self::J83CQam256AribSi => &J83C_REFERENCES,
            Self::IsdbTJapan => &ISDB_JAPAN_REFERENCES,
            Self::IsdbTInternational => &ISDB_INTERNATIONAL_REFERENCES,
            Self::ChinaDtvSi => &CHINA_DTV_REFERENCES,
        }
    }

    const fn is_dvb(self) -> bool {
        matches!(
            self,
            Self::DvbSi | Self::DvbT | Self::DvbT2 | Self::DvbCAnnexA
        )
    }

    const fn is_atsc1(self) -> bool {
        matches!(self, Self::Atsc1A65C2006 | Self::Atsc1A65_2013)
    }

    const fn is_atsc_cable(self) -> bool {
        matches!(
            self,
            Self::J83BAtscCablePsip | Self::J83BQam64AtscCablePsip | Self::J83BQam256AtscCablePsip
        )
    }

    const fn has_explicit_delivery(self) -> bool {
        !matches!(
            self,
            Self::MpegPsiOnly | Self::DvbSi | Self::J83BAtscCablePsip
        )
    }

    const fn tr101290_rules(self) -> Tr101290Rules {
        if self.is_atsc1() || self.is_atsc_cable() {
            Tr101290Rules::ATSC_A78
        } else {
            Tr101290Rules::MPEG_TS
        }
    }
}

const MPEG_SYSTEMS: StandardReference = StandardReference {
    code: "ISO/IEC 13818-1 / ITU-T H.222.0",
    title: "MPEG-2 Systems",
    scope: "TS packets, PSI, PES, continuity, PCR, PTS/DTS and CRC",
};
const TR101290: StandardReference = StandardReference {
    code: "ETSI TR 101 290 V1.4.1",
    title: "Measurement guidelines for DVB systems",
    scope: "Priority 1/2 TS health plus DVB Priority 3 checks",
};
const DVB_SI: StandardReference = StandardReference {
    code: "ETSI EN 300 468",
    title: "DVB Service Information",
    scope: "DVB-SI tables, descriptors, PID and table_id assignments",
};
const DVB_T: StandardReference = StandardReference {
    code: "ETSI EN 300 744",
    title: "DVB-T",
    scope: "Terrestrial delivery profile",
};
const DVB_T2: StandardReference = StandardReference {
    code: "ETSI EN 302 755",
    title: "DVB-T2",
    scope: "Second-generation terrestrial delivery profile",
};
const DVB_C: StandardReference = StandardReference {
    code: "ETSI EN 300 429 / ITU-T J.83 Annex A",
    title: "DVB-C",
    scope: "Cable delivery profile",
};
const ATSC_PSIP: StandardReference = StandardReference {
    code: "ATSC A/65; ATSC A/69",
    title: "Program and System Information Protocol",
    scope: "MGT, VCT, RRT, EIT, ETT and STT",
};
const ATSC_A78: StandardReference = StandardReference {
    code: "ATSC A/78:2015",
    title: "Transport Stream Verification",
    scope: "ATSC 1.0 transport verification",
};
const J83B: StandardReference = StandardReference {
    code: "ITU-T J.83 Annex B",
    title: "Digital cable television System B",
    scope: "North American cable delivery",
};
const J83C: StandardReference = StandardReference {
    code: "ITU-T J.83 Annex C",
    title: "Digital cable television System C",
    scope: "Japanese cable delivery",
};
const ARIB_SI: StandardReference = StandardReference {
    code: "ARIB STD-B10",
    title: "Service Information for Digital Broadcasting",
    scope: "Japanese SI tables and identifiers",
};
const ABNT_SI: StandardReference = StandardReference {
    code: "ABNT NBR 15603-1/-2/-3",
    title: "Multiplexing and service information",
    scope: "ISDB-T International/SBTVD SI",
};
const CHINA_SI: StandardReference = StandardReference {
    code: "GB/T 28161-2011",
    title: "Digital television broadcasting service information",
    scope: "China DTV SI tables",
};

const MPEG_TS_REFERENCES: [StandardReference; 2] = [MPEG_SYSTEMS, TR101290];
const DVB_REFERENCES: [StandardReference; 3] = [MPEG_SYSTEMS, TR101290, DVB_SI];
const DVB_T_REFERENCES: [StandardReference; 4] = [MPEG_SYSTEMS, TR101290, DVB_SI, DVB_T];
const DVB_T2_REFERENCES: [StandardReference; 4] = [MPEG_SYSTEMS, TR101290, DVB_SI, DVB_T2];
const DVB_C_REFERENCES: [StandardReference; 4] = [MPEG_SYSTEMS, TR101290, DVB_SI, DVB_C];
const ATSC_REFERENCES: [StandardReference; 4] = [MPEG_SYSTEMS, TR101290, ATSC_A78, ATSC_PSIP];
const ATSC_CABLE_REFERENCES: [StandardReference; 5] =
    [MPEG_SYSTEMS, TR101290, ATSC_A78, ATSC_PSIP, J83B];
const J83C_REFERENCES: [StandardReference; 4] = [MPEG_SYSTEMS, TR101290, J83C, ARIB_SI];
const ISDB_JAPAN_REFERENCES: [StandardReference; 3] = [MPEG_SYSTEMS, TR101290, ARIB_SI];
const ISDB_INTERNATIONAL_REFERENCES: [StandardReference; 3] = [MPEG_SYSTEMS, TR101290, ABNT_SI];
const CHINA_DTV_REFERENCES: [StandardReference; 3] = [MPEG_SYSTEMS, TR101290, CHINA_SI];

#[derive(Clone, Debug)]
pub struct ComplianceIndicator {
    pub group: &'static str,
    pub name: &'static str,
    pub observed: Option<u64>,
    pub status: ComplianceStatus,
    pub note: &'static str,
    pub reference: &'static str,
}

#[derive(Clone, Debug)]
pub struct ComplianceReport {
    pub profile: ComplianceProfile,
    pub hierarchy: ComplianceHierarchy,
    pub standards: &'static [StandardReference],
    pub indicators: Vec<ComplianceIndicator>,
    pub events: Vec<TrEvent>,
}

impl ComplianceReport {
    pub fn count(&self, name: &str) -> Option<u64> {
        self.indicators
            .iter()
            .find(|item| item.name == name)
            .and_then(|item| item.observed)
    }
}

fn measured_status(observed: Option<u64>) -> ComplianceStatus {
    match observed {
        Some(0) => ComplianceStatus::Pass,
        Some(_) => ComplianceStatus::Fail,
        None => ComplianceStatus::NotObserved,
    }
}

fn includes_indicator(profile: ComplianceProfile, priority: u8, _name: &str) -> bool {
    priority <= 2 || profile.is_dvb()
}

fn indicator_group(profile: ComplianceProfile, priority: u8) -> &'static str {
    match priority {
        1 => "TR 101 290 Priority 1 — MPEG-2 TS core",
        2 => "TR 101 290 Priority 2 — MPEG-2 TS timing",
        _ if profile.is_dvb() => "TR 101 290 Priority 3 — DVB-SI",
        _ => "TR 101 290 Priority 3 — profile-specific signalling",
    }
}

fn indicator_reference(profile: ComplianceProfile, priority: u8) -> &'static str {
    if profile.is_dvb() && priority == 3 {
        "ETSI TR 101 290; ETSI EN 300 468"
    } else if profile.is_atsc1() || profile.is_atsc_cable() {
        "ETSI TR 101 290; ATSC A/78:2015"
    } else {
        "ETSI TR 101 290; ISO/IEC 13818-1 / ITU-T H.222.0"
    }
}

fn delivery_indicator(profile: ComplianceProfile) -> Option<ComplianceIndicator> {
    profile.has_explicit_delivery().then(|| ComplianceIndicator {
        group: "Delivery / RF",
        name: "delivery_layer_measurement",
        observed: None,
        status: ComplianceStatus::NotApplicable,
        note: "Static TS analysis cannot verify RF modulation, FEC, MER or BER; delivery values shown by this profile are signalled hints or manual profile selections.",
        reference: profile.hierarchy().delivery,
    })
}

fn profile_checks(report: &AnalysisReport, profile: ComplianceProfile) -> ProfileCheckReport {
    match profile {
        ComplianceProfile::Atsc1A65C2006 => atsc_a65::analyze(
            report,
            atsc_a65::ServiceType::Terrestrial,
            "ATSC A/65C:2006 + Amendment 1; ATSC A/69",
        ),
        ComplianceProfile::Atsc1A65_2013 => atsc_a65::analyze(
            report,
            atsc_a65::ServiceType::Terrestrial,
            "ATSC A/65:2013; ATSC A/69",
        ),
        ComplianceProfile::J83BAtscCablePsip
        | ComplianceProfile::J83BQam64AtscCablePsip
        | ComplianceProfile::J83BQam256AtscCablePsip => atsc_a65::analyze(
            report,
            atsc_a65::ServiceType::Cable,
            "ATSC A/65:2013; ATSC A/69; ITU-T J.83 Annex B",
        ),
        ComplianceProfile::J83CQam64AribSi | ComplianceProfile::J83CQam256AribSi => {
            isdb_arib::analyze(report, isdb_arib::ProfileKind::J83C)
        }
        ComplianceProfile::IsdbTJapan => isdb_arib::analyze(report, isdb_arib::ProfileKind::Japan),
        ComplianceProfile::IsdbTInternational => {
            isdb_arib::analyze(report, isdb_arib::ProfileKind::International)
        }
        ComplianceProfile::ChinaDtvSi => china_dtmb::analyze(report),
        ComplianceProfile::MpegPsiOnly
        | ComplianceProfile::DvbSi
        | ComplianceProfile::DvbT
        | ComplianceProfile::DvbT2
        | ComplianceProfile::DvbCAnnexA => ProfileCheckReport::default(),
    }
}

pub fn tr101290_report(report: &AnalysisReport, profile: ComplianceProfile) -> ComplianceReport {
    let raw = tr101290::analyze(report, profile.tr101290_rules());
    let mut indicators = raw
        .indicators
        .into_iter()
        .filter(|item| includes_indicator(profile, item.priority, item.name))
        .map(|item| ComplianceIndicator {
            group: indicator_group(profile, item.priority),
            name: item.name,
            observed: item.observed,
            status: measured_status(item.observed),
            note: item.note,
            reference: indicator_reference(profile, item.priority),
        })
        .collect::<Vec<_>>();
    if let Some(delivery) = delivery_indicator(profile) {
        indicators.push(delivery);
    }

    let included = indicators
        .iter()
        .filter(|item| item.status != ComplianceStatus::NotApplicable)
        .map(|item| item.name)
        .collect::<BTreeSet<_>>();
    let mut events = raw
        .events
        .into_iter()
        .filter(|event| included.contains(event.indicator))
        .collect::<Vec<_>>();
    let mut specific = profile_checks(report, profile);
    indicators.append(&mut specific.indicators);
    events.append(&mut specific.events);
    events.sort_by_key(|event| event.packet_index);

    ComplianceReport {
        profile,
        hierarchy: profile.hierarchy(),
        standards: profile.references(),
        indicators,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::{ComplianceFamily, ComplianceProfile, ComplianceSystem, includes_indicator};

    #[test]
    fn profile_inventory_matches_supported_product_families() {
        let labels = ComplianceProfile::ALL
            .map(ComplianceProfile::label)
            .join("\n");
        for unsupported in ["› DVB-S ›", "DVB-S2", "ATSC › 3.0", "SCTE 65"] {
            assert!(!labels.contains(unsupported));
        }
        for supported in ["DVB-T2", "J.83 Annex A", "J.83B", "J.83C", "ISDB-T", "DTMB"] {
            assert!(labels.contains(supported));
        }
    }

    #[test]
    fn hierarchy_separates_cable_annexes_and_constellations() {
        let j83b64 = ComplianceProfile::J83BQam64AtscCablePsip.hierarchy();
        let j83b256 = ComplianceProfile::J83BQam256AtscCablePsip.hierarchy();
        let j83c64 = ComplianceProfile::J83CQam64AribSi.hierarchy();
        assert_eq!(j83b64.system, ComplianceSystem::J83B);
        assert_eq!(j83c64.system, ComplianceSystem::J83C);
        assert_eq!(j83c64.family, ComplianceFamily::JapaneseCable);
        assert_ne!(j83b64.delivery, j83b256.delivery);
        assert_ne!(j83b64.signaling, j83c64.signaling);
    }

    #[test]
    fn common_checks_and_dvb_priority_three_have_separate_scope() {
        assert!(includes_indicator(
            ComplianceProfile::Atsc1A65_2013,
            1,
            "PAT_error"
        ));
        assert!(includes_indicator(
            ComplianceProfile::IsdbTJapan,
            2,
            "PCR_accuracy_error"
        ));
        assert!(!includes_indicator(
            ComplianceProfile::Atsc1A65_2013,
            3,
            "NIT_actual_error"
        ));
        assert!(includes_indicator(
            ComplianceProfile::DvbSi,
            3,
            "NIT_actual_error"
        ));
    }
}
