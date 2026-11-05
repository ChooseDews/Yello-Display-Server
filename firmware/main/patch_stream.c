#include "patch_stream.h"

#include <string.h>

static uint16_t read_le16(const uint8_t *p)
{
    return (uint16_t)(p[0] | ((uint16_t)p[1] << 8));
}

static patch_stream_status_t fail(patch_stream_t *ps, patch_stream_status_t status)
{
    if (ps->sticky == PATCH_STREAM_OK) {
        ps->sticky = status;
    }
    return ps->sticky;
}

/* Feed decompressed bundle bytes into the zone-record parser, blitting
 * complete strips as they fill. */
static patch_stream_status_t parser_feed(patch_stream_t *ps, const uint8_t *data, size_t len)
{
    while (len > 0) {
        if (!ps->in_zone_pixels) {
            size_t want = sizeof(ps->hdr_buf) - ps->hdr_have;
            size_t take = len < want ? len : want;
            memcpy(ps->hdr_buf + ps->hdr_have, data, take);
            ps->hdr_have += take;
            data += take;
            len -= take;
            if (ps->hdr_have < sizeof(ps->hdr_buf)) {
                break;
            }
            ps->hdr_have = 0;
            uint16_t x = read_le16(ps->hdr_buf + 0);
            uint16_t y = read_le16(ps->hdr_buf + 2);
            uint16_t w = read_le16(ps->hdr_buf + 4);
            uint16_t h = read_le16(ps->hdr_buf + 6);
            bool bounds_valid = w > 0 && h > 0 && x < ps->logical_w && y < ps->logical_h &&
                                w <= ps->logical_w - x && h <= ps->logical_h - y;
            if (!bounds_valid) {
                return fail(ps, PATCH_STREAM_MALFORMED);
            }
            ps->zones_seen++;
            if (ps->zones_seen > ps->zones_expected) {
                return fail(ps, PATCH_STREAM_MALFORMED);
            }
            ps->zone_x = x;
            ps->zone_y = y;
            ps->zone_w = w;
            ps->zone_h = h;
            ps->zone_pixels_remaining = (uint32_t)w * (uint32_t)h * 2U;
            ps->zone_rows_done = 0;
            uint16_t rows_per_strip = (uint16_t)(PATCH_STRIP_MAX_WIDTH * PATCH_STRIP_ROWS / w);
            if (rows_per_strip == 0) {
                rows_per_strip = 1;
            }
            if (rows_per_strip > h) {
                rows_per_strip = h;
            }
            ps->strip_rows_target = rows_per_strip;
            ps->strip_capacity = (size_t)rows_per_strip * w * 2;
            ps->strip_fill = 0;
            ps->in_zone_pixels = true;
        } else {
            size_t room = ps->strip_capacity - ps->strip_fill;
            size_t want = room;
            if (want > ps->zone_pixels_remaining) {
                want = ps->zone_pixels_remaining;
            }
            size_t take = len < want ? len : want;
            memcpy(ps->strip_buf + ps->strip_fill, data, take);
            ps->strip_fill += take;
            ps->zone_pixels_remaining -= (uint32_t)take;
            data += take;
            len -= take;
            if (ps->strip_fill == ps->strip_capacity) {
                ps->blit(ps->blit_ctx, ps->zone_x, (uint16_t)(ps->zone_y + ps->zone_rows_done),
                         ps->zone_w, ps->strip_rows_target, ps->strip_buf);
                ps->zone_rows_done = (uint16_t)(ps->zone_rows_done + ps->strip_rows_target);
                ps->strip_fill = 0;
                uint16_t rows_left = (uint16_t)(ps->zone_h - ps->zone_rows_done);
                if (rows_left > 0) {
                    uint16_t rows_per_strip =
                        (uint16_t)(PATCH_STRIP_MAX_WIDTH * PATCH_STRIP_ROWS / ps->zone_w);
                    if (rows_per_strip == 0) {
                        rows_per_strip = 1;
                    }
                    if (rows_per_strip > rows_left) {
                        rows_per_strip = rows_left;
                    }
                    ps->strip_rows_target = rows_per_strip;
                    ps->strip_capacity = (size_t)rows_per_strip * ps->zone_w * 2;
                }
            }
            if (ps->zone_pixels_remaining == 0) {
                if (ps->strip_fill != 0) {
                    /* Cannot happen by construction (the last strip's
                     * capacity is exactly the remaining rows), but never
                     * leave a partial strip unblitted or stuck. */
                    uint16_t rows = (uint16_t)(ps->strip_fill / ((size_t)ps->zone_w * 2));
                    if (rows > 0) {
                        ps->blit(ps->blit_ctx, ps->zone_x,
                                 (uint16_t)(ps->zone_y + ps->zone_rows_done), ps->zone_w, rows,
                                 ps->strip_buf);
                    }
                    ps->strip_fill = 0;
                }
                ps->in_zone_pixels = false;
            }
        }
    }
    return ps->sticky;
}

void patch_stream_init(patch_stream_t *ps, patch_blit_fn blit, void *blit_ctx)
{
    memset(ps, 0, sizeof(*ps));
    ps->blit = blit;
    ps->blit_ctx = blit_ctx;
    ps->sticky = PATCH_STREAM_OK;
}

void patch_stream_reset(patch_stream_t *ps)
{
    patch_blit_fn blit = ps->blit;
    void *ctx = ps->blit_ctx;
    bool have_seq = ps->have_seq;
    uint32_t expected = ps->expected_seq;
    uint32_t last_good = ps->last_good_seq;
    patch_stream_init(ps, blit, ctx);
    ps->have_seq = have_seq;
    ps->expected_seq = expected;
    ps->last_good_seq = last_good;
}

uint32_t patch_stream_last_good_seq(const patch_stream_t *ps)
{
    return ps->last_good_seq;
}

patch_stream_status_t patch_stream_begin(patch_stream_t *ps,
                                         const yello_frame_patch_header_t *hdr,
                                         uint16_t logical_w, uint16_t logical_h)
{
    patch_stream_reset(ps);
    ps->in_patch = true;
    ps->logical_w = logical_w;
    ps->logical_h = logical_h;
    ps->frame_seq = hdr->frame_seq;
    ps->zones_expected = hdr->zone_count;
    ps->uncompressed_len = hdr->uncompressed_len;
    ps->compressed = (hdr->flags & YELLO_PATCH_FLAG_COMPRESSED) != 0;
    bool full = (hdr->flags & YELLO_PATCH_FLAG_FULL_FRAME) != 0;

    if (hdr->magic != YELLO_PROTO_MAGIC || hdr->version != YELLO_PROTO_VERSION ||
        hdr->msg_type != YELLO_MSG_FRAME_PATCH || hdr->zone_count == 0 ||
        hdr->uncompressed_len == 0 || hdr->uncompressed_len > PATCH_MAX_UNCOMPRESSED_BYTES ||
        hdr->payload_len == 0 || (!ps->compressed && hdr->payload_len != hdr->uncompressed_len)) {
        return fail(ps, PATCH_STREAM_MALFORMED);
    }
    if (!full && ps->have_seq && hdr->frame_seq != ps->expected_seq) {
        return fail(ps, PATCH_STREAM_SEQ_GAP);
    }
    if (ps->compressed) {
        tinfl_init(&ps->decomp);
        ps->dict_ofs = 0;
    }
    return PATCH_STREAM_OK;
}

static patch_stream_status_t inflate_feed(patch_stream_t *ps, const uint8_t *data, size_t len,
                                          bool final_chunk)
{
    /* HAS_MORE_INPUT describes the whole feed call: set on every
     * non-final chunk, clear for the final chunk (including drain calls
     * with no input left, which let tinfl validate the stream end and
     * the Adler-32 trailer). */
    const mz_uint32 flags = TINFL_FLAG_PARSE_ZLIB_HEADER |
                            (final_chunk ? 0 : (mz_uint32)TINFL_FLAG_HAS_MORE_INPUT);
    size_t in_ofs = 0;
    for (;;) {
        size_t in_size = len - in_ofs;
        size_t out_size = PATCH_DICT_SIZE - ps->dict_ofs;
        tinfl_status status = tinfl_decompress(&ps->decomp, data + in_ofs, &in_size, ps->dict,
                                               ps->dict + ps->dict_ofs, &out_size, flags);
        size_t consumed = in_size;
        in_ofs += in_size;
        if (out_size > 0) {
            const uint8_t *produced = ps->dict + ps->dict_ofs;
            ps->dict_ofs = (ps->dict_ofs + out_size) & (PATCH_DICT_SIZE - 1);
            ps->uncompressed_seen += (uint32_t)out_size;
            if (ps->uncompressed_seen > ps->uncompressed_len) {
                return fail(ps, PATCH_STREAM_MALFORMED);
            }
            patch_stream_status_t pst = parser_feed(ps, produced, out_size);
            if (pst != PATCH_STREAM_OK) {
                return pst;
            }
        }
        if (status == TINFL_STATUS_DONE) {
            ps->inflate_done = true;
            if (in_ofs < len) {
                return fail(ps, PATCH_STREAM_MALFORMED); /* trailing garbage */
            }
            return ps->sticky;
        }
        if (status == TINFL_STATUS_ADLER32_MISMATCH) {
            return fail(ps, PATCH_STREAM_ADLER_MISMATCH);
        }
        if (status < 0) {
            return fail(ps, PATCH_STREAM_INFLATE_FAILED);
        }
        if (status == TINFL_STATUS_NEEDS_MORE_INPUT) {
            /* All input consumed and the inflater is parked mid-stream.
             * Fine between chunks; a truncated stream on the final chunk
             * is an error (end() would also catch it via inflate_done). */
            if (final_chunk && in_ofs >= len) {
                return fail(ps, PATCH_STREAM_INFLATE_FAILED);
            }
            return ps->sticky;
        }
        /* TINFL_STATUS_HAS_MORE_OUTPUT: output space filled, call again
         * with fresh dictionary space and whatever input remains. Guard
         * against a no-progress spin, which legitimate tinfl never does
         * (HAS_MORE_OUTPUT implies it filled the output buffer). */
        if (consumed == 0 && out_size == 0) {
            return fail(ps, PATCH_STREAM_INFLATE_FAILED);
        }
        if (in_ofs >= len && !final_chunk && out_size == 0) {
            return ps->sticky;
        }
    }
}

patch_stream_status_t patch_stream_feed(patch_stream_t *ps, const uint8_t *data, size_t len,
                                        bool final_chunk)
{
    if (!ps->in_patch) {
        return PATCH_STREAM_MALFORMED;
    }
    if (ps->sticky != PATCH_STREAM_OK) {
        return ps->sticky; /* drain the rest of a rejected patch */
    }
    if (ps->compressed) {
        return inflate_feed(ps, data, len, final_chunk);
    }
    ps->uncompressed_seen += (uint32_t)len;
    if (ps->uncompressed_seen > ps->uncompressed_len) {
        return fail(ps, PATCH_STREAM_MALFORMED);
    }
    return parser_feed(ps, data, len);
}

patch_stream_status_t patch_stream_end(patch_stream_t *ps)
{
    if (!ps->in_patch) {
        return PATCH_STREAM_MALFORMED;
    }
    patch_stream_status_t status = ps->sticky;
    if (status == PATCH_STREAM_OK) {
        if (ps->compressed && !ps->inflate_done) {
            status = PATCH_STREAM_INFLATE_FAILED;
        } else if (ps->uncompressed_seen != ps->uncompressed_len) {
            status = PATCH_STREAM_MALFORMED;
        } else if (ps->zones_seen != ps->zones_expected || ps->in_zone_pixels || ps->hdr_have != 0) {
            status = PATCH_STREAM_MALFORMED;
        }
    }
    ps->in_patch = false;
    if (status == PATCH_STREAM_OK) {
        ps->have_seq = true;
        ps->last_good_seq = ps->frame_seq;
        ps->expected_seq = ps->frame_seq + 1;
    }
    ps->sticky = PATCH_STREAM_OK;
    return status;
}
