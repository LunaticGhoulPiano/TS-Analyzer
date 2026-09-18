#ifndef TSAN_TSDUCK_BRIDGE_H
#define TSAN_TSDUCK_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct TSAN_TSDUCK_SESSION TSAN_TSDUCK_SESSION;

typedef struct TSAN_TSDUCK_SNAPSHOT {
    uint64_t packets;
    uint64_t continuity_errors;
    uint64_t valid_sections;
    uint64_t pat_sections;
    uint64_t pmt_sections;
    uint32_t standards;
} TSAN_TSDUCK_SNAPSHOT;

typedef struct TSAN_VIDEO_SPS {
    uint32_t width;
    uint32_t height;
    uint32_t frame_rate_numerator;
    uint32_t frame_rate_denominator;
} TSAN_VIDEO_SPS;

int tsan_tsduck_parse_sps(uint8_t stream_type, const uint8_t *nal, size_t length, TSAN_VIDEO_SPS *out);
int tsan_tsduck_create(TSAN_TSDUCK_SESSION **session);
int tsan_tsduck_feed(TSAN_TSDUCK_SESSION *session, const uint8_t *data, size_t length);
int tsan_tsduck_snapshot(const TSAN_TSDUCK_SESSION *session, TSAN_TSDUCK_SNAPSHOT *snapshot);
void tsan_tsduck_destroy(TSAN_TSDUCK_SESSION *session);

#ifdef __cplusplus
}
#endif

#endif
