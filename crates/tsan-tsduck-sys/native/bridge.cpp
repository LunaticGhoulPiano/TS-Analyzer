#include <cstring>
#include <new>

#include "bridge.h"
#include "tsAVCSequenceParameterSet.h"
#include "tsContinuityAnalyzer.h"
#include "tsHEVCSequenceParameterSet.h"
#include "tsDuckContext.h"
#include "tsNullReport.h"
#include "tsSection.h"
#include "tsSectionDemux.h"
#include "tsTSAnalyzer.h"
#include "tsTSPacket.h"
#include "tsTSPacketMetadata.h"

struct TSAN_TSDUCK_SESSION final : private ts::SectionHandlerInterface {
    ts::DuckContext duck {&NULLREP};
    ts::SectionDemux sections {duck, nullptr, this, ts::AllPIDs()};
    ts::ContinuityAnalyzer continuity {ts::AllPIDs(), &NULLREP};
    ts::TSAnalyzer analyzer {duck};
    uint64_t packets = 0;
    uint64_t valid_sections = 0;
    uint64_t pat_sections = 0;
    uint64_t pmt_sections = 0;

    void handleSection(ts::SectionDemux &, const ts::Section &section) override {
        ++valid_sections;
        if (section.tableId() == 0x00) ++pat_sections;
        else if (section.tableId() == 0x02) ++pmt_sections;
    }
};

extern "C" int tsan_tsduck_parse_sps(uint8_t stream_type, const uint8_t *nal, size_t length, TSAN_VIDEO_SPS *out) {
    if (nal == nullptr || out == nullptr || length == 0) return -1;
    *out = {};
    try {
        if (stream_type == 0x1b) {
            const ts::AVCSequenceParameterSet sps(nal, length);
            if (! sps.valid) return -4;
            out->width = sps.frameWidth();
            out->height = sps.frameHeight();
            if (sps.vui_parameters_present_flag && sps.vui.timing_info_present_flag && sps.vui.num_units_in_tick != 0) {
                out->frame_rate_numerator = sps.vui.time_scale;
                out->frame_rate_denominator = 2 * sps.vui.num_units_in_tick;
            }
        }
        else if (stream_type == 0x24) {
            const ts::HEVCSequenceParameterSet sps(nal, length);
            if (! sps.valid) return -4;
            out->width = sps.frameWidth();
            out->height = sps.frameHeight();
            if (sps.vui_parameters_present_flag && sps.vui.vui_timing_info_present_flag && sps.vui.vui_num_units_in_tick != 0) {
                out->frame_rate_numerator = sps.vui.vui_time_scale;
                out->frame_rate_denominator = sps.vui.vui_num_units_in_tick;
                if (sps.vui.vui_poc_proportional_to_timing_flag) {
                    out->frame_rate_denominator *= sps.vui.vui_num_ticks_poc_diff_one_minus1 + 1;
                }
            }
        }
        else return -1;
        return 0;
    }
    catch (...) {
        return -2;
    }
}

extern "C" int tsan_tsduck_create(TSAN_TSDUCK_SESSION **session) {
    if (session == nullptr) return -1;
    *session = nullptr;
    try {
        *session = new TSAN_TSDUCK_SESSION();
        return 0;
    }
    catch (...) {
        return -2;
    }
}

extern "C" int tsan_tsduck_feed(TSAN_TSDUCK_SESSION *session, const uint8_t *data, size_t length) {
    if (session == nullptr || (data == nullptr && length != 0)) return -1;
    if (length % ts::PKT_SIZE != 0) return -3;
    try {
        for (size_t offset = 0; offset < length; offset += ts::PKT_SIZE) {
            ts::TSPacket packet;
            std::memcpy(packet.b, data + offset, ts::PKT_SIZE);
            if (! packet.hasValidSync()) return -3;
            session->continuity.feedPacket(static_cast<const ts::TSPacket &>(packet));
            session->sections.feedPacket(packet);
            ts::TSPacketMetadata metadata;
            session->analyzer.feedPacket(packet, metadata);
            ++session->packets;
        }
        return 0;
    }
    catch (...) {
        return -2;
    }
}

extern "C" int tsan_tsduck_snapshot(const TSAN_TSDUCK_SESSION *session, TSAN_TSDUCK_SNAPSHOT *snapshot) {
    if (session == nullptr || snapshot == nullptr) return -1;
    try {
        snapshot->packets = session->packets;
        snapshot->continuity_errors = session->continuity.errorCount();
        snapshot->valid_sections = session->valid_sections;
        snapshot->pat_sections = session->pat_sections;
        snapshot->pmt_sections = session->pmt_sections;
        snapshot->standards = static_cast<uint16_t>(session->duck.standards());
        return 0;
    }
    catch (...) {
        return -2;
    }
}

extern "C" void tsan_tsduck_destroy(TSAN_TSDUCK_SESSION *session) {
    try {
        delete session;
    }
    catch (...) {
    }
}
