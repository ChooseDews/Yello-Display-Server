/* Host test for the firmware streaming FramePatch decoder
 * (firmware/main/patch_stream.c + tinfl.c) - the exact code that runs
 * on the ESP32, driven here with generated packets fed in several
 * fragment sizes, against a trivially-simple reference bundle parser.
 *
 * Build/run via run_tests.sh. Prints one line per check and exits
 * non-zero on the first failure count > 0. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../main/patch_stream.h"

#define FB_W 240
#define FB_H 320
static uint8_t s_fb[FB_W * FB_H * 2];
static uint8_t s_expected[FB_W * FB_H * 2];
static int s_blit_calls;
static int s_failures;

static void blit(void *ctx, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint8_t *px)
{
    (void)ctx;
    s_blit_calls++;
    for (uint16_t row = 0; row < h; row++) {
        memcpy(&s_fb[((size_t)(y + row) * FB_W + x) * 2], px + (size_t)row * w * 2, (size_t)w * 2);
    }
}

static uint8_t *read_file(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        return NULL;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *buf = malloc(n > 0 ? n : 1);
    if (fread(buf, 1, n, f) != (size_t)n) {
        fclose(f);
        free(buf);
        return NULL;
    }
    fclose(f);
    *len = n;
    return buf;
}

/* Reference: apply a raw bundle to s_expected with a naive parser. */
static void apply_bundle_reference(const uint8_t *bundle, size_t len)
{
    size_t ofs = 0;
    while (ofs < len) {
        uint16_t x = bundle[ofs] | (bundle[ofs + 1] << 8);
        uint16_t y = bundle[ofs + 2] | (bundle[ofs + 3] << 8);
        uint16_t w = bundle[ofs + 4] | (bundle[ofs + 5] << 8);
        uint16_t h = bundle[ofs + 6] | (bundle[ofs + 7] << 8);
        ofs += 8;
        for (uint16_t row = 0; row < h; row++) {
            memcpy(&s_expected[((size_t)(y + row) * FB_W + x) * 2], bundle + ofs + (size_t)row * w * 2,
                   (size_t)w * 2);
        }
        ofs += (size_t)w * h * 2;
    }
}

static patch_stream_t s_ps;

/* Run one packet through the stream in `chunk`-sized feeds.
 * Returns end() status; begin status in *begin_out when non-NULL. */
static patch_stream_status_t run_packet(const char *dir, const char *name, size_t chunk,
                                        patch_stream_status_t *begin_out)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/%s.packet", dir, name);
    size_t pkt_len;
    uint8_t *pkt = read_file(path, &pkt_len);
    if (!pkt) {
        fprintf(stderr, "cannot read %s\n", path);
        exit(2);
    }
    const yello_frame_patch_header_t *hdr = (const yello_frame_patch_header_t *)pkt;
    patch_stream_status_t bst = patch_stream_begin(&s_ps, hdr, FB_W, FB_H);
    if (begin_out) {
        *begin_out = bst;
    }
    size_t payload_len = pkt_len - sizeof(*hdr);
    size_t ofs = 0;
    while (ofs < payload_len) {
        size_t take = chunk;
        if (take > payload_len - ofs) {
            take = payload_len - ofs;
        }
        bool final = (ofs + take >= payload_len);
        patch_stream_feed(&s_ps, pkt + sizeof(*hdr) + ofs, take, final);
        ofs += take;
    }
    if (payload_len == 0) {
        patch_stream_feed(&s_ps, pkt + sizeof(*hdr), 0, true);
    }
    patch_stream_status_t est = patch_stream_end(&s_ps);
    free(pkt);
    return est;
}

static void check(int cond, const char *what)
{
    if (cond) {
        printf("  ok: %s\n", what);
    } else {
        printf("  FAIL: %s\n", what);
        s_failures++;
    }
}

static void expect_framebuffer_matches(const char *dir, const char *name)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/%s.bundle", dir, name);
    size_t len;
    uint8_t *bundle = read_file(path, &len);
    if (!bundle) {
        fprintf(stderr, "cannot read %s\n", path);
        exit(2);
    }
    apply_bundle_reference(bundle, len);
    free(bundle);
    check(memcmp(s_fb, s_expected, sizeof(s_fb)) == 0, "framebuffer matches reference bundle");
}

static void reset_fb(void)
{
    memset(s_fb, 0xA5, sizeof(s_fb));
    memset(s_expected, 0xA5, sizeof(s_expected));
    s_blit_calls = 0;
}

int main(int argc, char **argv)
{
    const char *dir = argc > 1 ? argv[1] : "cases";
    patch_stream_init(&s_ps, blit, NULL);
    const size_t chunks[] = {1 << 20, 1024, 7, 1};

    for (size_t ci = 0; ci < sizeof(chunks) / sizeof(chunks[0]); ci++) {
        size_t chunk = chunks[ci];
        printf("chunk=%zu\n", chunk);
        patch_stream_init(&s_ps, blit, NULL);
        reset_fb();

        patch_stream_status_t st = run_packet(dir, "ui_full", chunk, NULL);
        check(st == PATCH_STREAM_OK, "ui_full (compressed, full) decodes");
        expect_framebuffer_matches(dir, "ui_full");
        check(s_blit_calls > 1, "ui_full blitted in strips, not one block");

        st = run_packet(dir, "zones_raw", chunk, NULL);
        check(st == PATCH_STREAM_OK, "zones_raw (raw fallback, odd shapes) decodes");
        expect_framebuffer_matches(dir, "zones_raw");

        st = run_packet(dir, "zones_zlib", chunk, NULL);
        check(st == PATCH_STREAM_OK, "zones_zlib (compressed multi-zone) decodes");
        expect_framebuffer_matches(dir, "zones_zlib");

        /* Sequence gap: seq 9 when 3 is expected must be rejected and
         * leave the framebuffer untouched; the following full frame
         * (any seq) resynchronises. */
        uint8_t before[sizeof(s_fb)];
        memcpy(before, s_fb, sizeof(s_fb));
        patch_stream_status_t bst;
        st = run_packet(dir, "gap_patch", chunk, &bst);
        check(bst == PATCH_STREAM_SEQ_GAP, "gap_patch rejected at begin (SEQ_GAP)");
        check(st == PATCH_STREAM_SEQ_GAP, "gap_patch end reports SEQ_GAP");
        check(memcmp(before, s_fb, sizeof(s_fb)) == 0, "gap_patch left framebuffer untouched");

        st = run_packet(dir, "resync_full", chunk, NULL);
        check(st == PATCH_STREAM_OK, "resync_full (full frame) applies after a gap");
        expect_framebuffer_matches(dir, "resync_full");
    }

    /* Error cases (whole-packet feeds are fine here). */
    printf("error cases\n");
    patch_stream_init(&s_ps, blit, NULL);
    reset_fb();
    patch_stream_status_t st = run_packet(dir, "corrupt", 1 << 20, NULL);
    check(st != PATCH_STREAM_OK, "corrupt compressed stream fails (inflate/adler)");

    patch_stream_init(&s_ps, blit, NULL);
    st = run_packet(dir, "wbits15", 1 << 20, NULL);
    check(st == PATCH_STREAM_INFLATE_FAILED, "windowBits=15 stream rejected by 4 KiB dictionary");

    patch_stream_init(&s_ps, blit, NULL);
    st = run_packet(dir, "truncated", 1 << 20, NULL);
    check(st != PATCH_STREAM_OK, "truncated compressed stream fails");

    /* Photo case is optional (bench asset may not exist). */
    char photo_path[512];
    snprintf(photo_path, sizeof(photo_path), "%s/photo_full.packet", dir);
    FILE *probe = fopen(photo_path, "rb");
    if (probe) {
        fclose(probe);
        patch_stream_init(&s_ps, blit, NULL);
        reset_fb();
        st = run_packet(dir, "photo_full", 1024, NULL);
        check(st == PATCH_STREAM_OK, "photo_full (95 KiB stream, 4 KiB dict wrap) decodes");
        expect_framebuffer_matches(dir, "photo_full");
    }

    printf(s_failures == 0 ? "ALL PASS\n" : "%d FAILURES\n", s_failures);
    return s_failures == 0 ? 0 : 1;
}
