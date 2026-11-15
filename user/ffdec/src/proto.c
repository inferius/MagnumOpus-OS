/* proto.c - kodovani zprav protokolu medii (viz proto.h). */
#include <string.h>

#include "proto.h"

void w_init(wbuf *w, uint8_t *b, size_t cap) {
    w->b = b;
    w->len = 0;
    w->cap = cap;
    w->ok = 1;
}

static void w_put(wbuf *w, const void *d, size_t n) {
    if (!w->ok || w->cap - w->len < n) {
        w->ok = 0;
        return;
    }
    memcpy(w->b + w->len, d, n);
    w->len += n;
}

static void w_le(wbuf *w, uint64_t v, int n) {
    uint8_t t[8];
    for (int i = 0; i < n; i++)
        t[i] = (uint8_t)(v >> (8 * i));
    w_put(w, t, (size_t)n);
}

void w_u8(wbuf *w, uint8_t v) { w_le(w, v, 1); }
void w_u16(wbuf *w, uint16_t v) { w_le(w, v, 2); }
void w_u32(wbuf *w, uint32_t v) { w_le(w, v, 4); }
void w_i32(wbuf *w, int32_t v) { w_le(w, (uint32_t)v, 4); }
void w_u64(wbuf *w, uint64_t v) { w_le(w, v, 8); }

void w_str(wbuf *w, const char *s) {
    size_t n = s ? strlen(s) : 0;
    if (n > 0xFFFF)
        n = 0xFFFF;
    w_u16(w, (uint16_t)n);
    w_put(w, s, n);
}

void w_bytes(wbuf *w, const void *d, size_t n) { w_put(w, d, n); }

void r_init(rbuf *r, const uint8_t *b, size_t len) {
    r->b = b;
    r->len = len;
    r->pos = 0;
    r->ok = 1;
}

static uint64_t r_le(rbuf *r, int n) {
    if (!r->ok || r->len - r->pos < (size_t)n) {
        r->ok = 0;
        return 0;
    }
    uint64_t v = 0;
    for (int i = 0; i < n; i++)
        v |= (uint64_t)r->b[r->pos + (size_t)i] << (8 * i);
    r->pos += (size_t)n;
    return v;
}

uint8_t r_u8(rbuf *r) { return (uint8_t)r_le(r, 1); }
uint32_t r_u32(rbuf *r) { return (uint32_t)r_le(r, 4); }
int32_t r_i32(rbuf *r) { return (int32_t)(uint32_t)r_le(r, 4); }
uint64_t r_u64(rbuf *r) { return r_le(r, 8); }
int r_end(const rbuf *r) { return r->ok && r->pos == r->len; }

int r_bufdesc(rbuf *r, bufdesc *d) {
    d->fourcc = r_u32(r);
    (void)r_u64(r); /* modifikator */
    d->width = r_u32(r);
    d->height = r_u32(r);
    d->planes = r_u8(r);
    (void)r_u8(r); /* umisteni */
    (void)r_u8(r); /* cache */
    (void)r_u8(r);
    for (int i = 0; i < 4; i++) {
        d->offset[i] = r_u64(r);
        d->pitch[i] = r_u32(r);
    }
    return r->ok && d->planes >= 1 && d->planes <= 4;
}
