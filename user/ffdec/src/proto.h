/* proto.h - kodovani zprav protokolu medii pro ffdec (libs/mediaproto,
 * docs/cs/multimedia.md). Little-endian, retezec = delka u16 + UTF-8.
 * Cisla operaci a rozlozeni musi sedet s mediaproto (Rust); host test
 * libs/mediaproto/src/c_header_tests.rs cte tenhle soubor. */
#ifndef FFDEC_PROTO_H
#define FFDEC_PROTO_H

#include <stddef.h>
#include <stdint.h>

/* Pozadavky (mediad -> dekoder). */
#define OP_SELECT 3u
#define OP_POOL 4u
#define OP_AUDIO 5u
#define OP_PLAY 6u
#define OP_PAUSE 7u
#define OP_SEEK 8u
#define OP_RELEASE 9u
#define OP_CLOSE 10u
#define OP_PROBE 50u

/* Udalosti (dekoder -> mediad). */
#define EV_FRAME 100u
#define EV_AUDIO_BASE 101u
#define EV_END 102u
#define EV_ERROR 103u

/* Znacka odpovedi. */
#define TAG_REPLY 0u

/* Zdroj bajtu (dekoder -> mediad). */
#define SRC_READ_AT 1u
#define SRC_SIZE 2u
#define SRC_MAX_READ 61440u /* mediaproto::source::MAX_READ (60 KiB) */

#define MAX_POOL 16
#define MAX_STREAMS 32
/* Popis bufferu bufdesc: 72 B. */
#define BUFDESC_LEN 72
#define FOURCC_NV12 0x3231564Eu

/* Druhy proudu (mediaproto::Kind). */
#define KIND_VIDEO 1
#define KIND_AUDIO 2
#define KIND_SUBTITLE 3

/* Zapis zpravy do bufferu pevne velikosti (pri preteceni ok = 0). */
typedef struct {
    uint8_t *b;
    size_t len, cap;
    int ok;
} wbuf;

void w_init(wbuf *w, uint8_t *b, size_t cap);
void w_u8(wbuf *w, uint8_t v);
void w_u16(wbuf *w, uint16_t v);
void w_u32(wbuf *w, uint32_t v);
void w_i32(wbuf *w, int32_t v);
void w_u64(wbuf *w, uint64_t v);
void w_str(wbuf *w, const char *s);
void w_bytes(wbuf *w, const void *d, size_t n);

/* Cteni zpravy (mimo zpravu ok = 0, nikdy cteni za konec). */
typedef struct {
    const uint8_t *b;
    size_t len, pos;
    int ok;
} rbuf;

void r_init(rbuf *r, const uint8_t *b, size_t len);
uint8_t r_u8(rbuf *r);
uint32_t r_u32(rbuf *r);
int32_t r_i32(rbuf *r);
uint64_t r_u64(rbuf *r);
int r_end(const rbuf *r);

/* Popis bufferu (jen co dekoder potrebuje). */
typedef struct {
    uint32_t fourcc, width, height;
    uint8_t planes;
    uint64_t offset[4];
    uint32_t pitch[4];
} bufdesc;

int r_bufdesc(rbuf *r, bufdesc *d);

#endif
