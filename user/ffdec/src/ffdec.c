/* ffdec - proces dekoderu medii nad FFmpeg (docs/cs/multimedia.md).
 *
 * Spousti ho jen mediad, jednou na relaci, bez jakychkoli handlu krome
 * bootstrap kanalu (ridici kanal, protokol mediaproto). Vstup cte jen pres
 * kanal zdroje, ktery dostane v PROBE (libavformat s vlastnim AVIOContext);
 * nema soubory, sit ani ovladace. Video prevede (libswscale) do bufferu NV12
 * z poolu aplikace a posune plot snimku; zvuk prevede (libswresample) na
 * s16 stereo do kruhu audiod. Licence: FFmpeg LGPL 2.1+, tento soubor je
 * nas kod (MIT/Apache jako zbytek systemu), FFmpeg je dynamicky (libav*.so).
 *
 * Dekoduje dopredu, dokud ma volny buffer a misto v kruhu; tempo urcuje
 * aplikace (RELEASE) a prehravani zvuku. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <magnumopus/sys.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>

#include "proto.h"

#define MSG_MAX 4096
#define SOURCE_TIMEOUT_NS 5000000000LL
#define AVIO_BUF 65536

/* Hlavicka kruhu audiod (libs/audioproto/src/ring.rs). */
#define RING_CAPACITY 8
#define RING_FRAME 12
#define RING_WRITE 64
#define RING_READ 128
#define RING_DATA 4096

typedef struct {
    uint8_t *map;
    uint64_t map_len;
    bufdesc desc;
    int free;
} vbuf;

typedef struct {
    mo_handle_t ctl, src;
    uint64_t size; /* velikost zdroje (UINT64_MAX = nezname) */
    uint64_t pos;  /* pozice cteni AVIOContext */

    AVFormatContext *fmt;
    int vi, ai; /* vybrane proudy (-1 = zadny) */
    AVCodecContext *vdec, *adec;
    struct SwsContext *sws;
    SwrContext *swr;

    vbuf pool[MAX_POOL];
    int npool;
    mo_handle_t frame_fence;
    uint64_t serial;

    uint8_t *ring;
    uint64_t ring_len;
    uint32_t ring_cap;
    mo_handle_t ring_event;
    uint32_t ring_rate;
    int anchor_pending; /* poslat AUDIO_BASE s dalsim zvukem */

    AVPacket *pkt;
    AVFrame *vframe; /* dekodovany snimek cekajici na volny buffer */
    int vframe_ready;
    uint8_t *apend; /* prevedeny zvuk cekajici na misto v kruhu */
    size_t apend_len, apend_off;
    int64_t apend_pts;
    AVFrame *aframe;

    int playing, eof, end_sent;
    int64_t skip_until; /* presny SEEK: zahodit snimky pred timto pts (us) */
} dec;

/* ------------------------------------------------------------------ */
/* Zpravy */

static void send_msg(dec *d, const wbuf *w) {
    if (w->ok)
        (void)mo_channel_write(d->ctl, w->b, w->len, NULL, 0);
}

static void reply(dec *d, int status, const uint8_t *data, size_t len) {
    uint8_t b[MSG_MAX];
    wbuf w;
    w_init(&w, b, sizeof b);
    w_u32(&w, TAG_REPLY);
    w_i32(&w, status);
    if (status == 0 && data)
        w_bytes(&w, data, len);
    send_msg(d, &w);
}

static void ev_frame(dec *d, int slot, int64_t pts, uint32_t dur, uint64_t serial) {
    uint8_t b[64];
    wbuf w;
    w_init(&w, b, sizeof b);
    w_u32(&w, EV_FRAME);
    w_u8(&w, (uint8_t)slot);
    w_u64(&w, pts < 0 ? 0 : (uint64_t)pts);
    w_u32(&w, dur);
    w_u64(&w, serial);
    send_msg(d, &w);
}

static void ev_simple(dec *d, uint32_t ev) {
    uint8_t b[8];
    wbuf w;
    w_init(&w, b, sizeof b);
    w_u32(&w, ev);
    send_msg(d, &w);
}

static void ev_error(dec *d, int code) {
    uint8_t b[16];
    wbuf w;
    w_init(&w, b, sizeof b);
    w_u32(&w, EV_ERROR);
    w_i32(&w, code);
    w_u8(&w, 0);
    send_msg(d, &w);
}

/* ------------------------------------------------------------------ */
/* Zdroj bajtu (AVIOContext nad kanalem zdroje) */

static int src_call(dec *d, const uint8_t *req, size_t rlen, uint8_t *out, size_t cap, size_t *got) {
    int r = mo_channel_write(d->src, req, rlen, NULL, 0);
    if (r < 0)
        return r;
    static uint8_t b[SRC_MAX_READ + 64];
    size_t len = 0, nh = 0;
    r = mo_channel_read(d->src, b, sizeof b, &len, NULL, 0, &nh, mo_now_ns() + SOURCE_TIMEOUT_NS);
    if (r < 0)
        return r;
    rbuf rb;
    r_init(&rb, b, len);
    int32_t st = r_i32(&rb);
    if (!rb.ok)
        return MO_ERR_INVALID_ARGS;
    if (st != 0)
        return st;
    size_t n = len - rb.pos;
    if (n > cap)
        return MO_ERR_INVALID_ARGS;
    memcpy(out, b + rb.pos, n);
    *got = n;
    return 0;
}

static int src_read(void *opaque, uint8_t *buf, int size) {
    dec *d = opaque;
    if (size <= 0)
        return AVERROR(EINVAL);
    uint32_t want = (uint32_t)size > SRC_MAX_READ ? SRC_MAX_READ : (uint32_t)size;
    uint8_t req[16];
    wbuf w;
    w_init(&w, req, sizeof req);
    w_u32(&w, SRC_READ_AT);
    w_u64(&w, d->pos);
    w_u32(&w, want);
    size_t got = 0;
    int r = src_call(d, req, w.len, buf, want, &got);
    if (r < 0)
        return AVERROR(EIO);
    if (got == 0)
        return AVERROR_EOF;
    d->pos += got;
    return (int)got;
}

static int64_t src_seek(void *opaque, int64_t off, int whence) {
    dec *d = opaque;
    switch (whence & ~AVSEEK_FORCE) {
    case AVSEEK_SIZE:
        return d->size == UINT64_MAX ? -1 : (int64_t)d->size;
    case SEEK_SET:
        d->pos = (uint64_t)off;
        break;
    case SEEK_CUR:
        d->pos = (uint64_t)((int64_t)d->pos + off);
        break;
    case SEEK_END:
        if (d->size == UINT64_MAX)
            return -1;
        d->pos = (uint64_t)((int64_t)d->size + off);
        break;
    default:
        return -1;
    }
    return (int64_t)d->pos;
}

/* ------------------------------------------------------------------ */
/* PROBE: otevreni zdroje a popis proudu (mediaproto::Info) */

static int probe(dec *d, uint64_t size, mo_handle_t src, uint8_t *out, size_t cap, size_t *len) {
    d->src = src;
    d->size = size;
    d->pos = 0;
    uint8_t *iobuf = av_malloc(AVIO_BUF);
    AVIOContext *io = avio_alloc_context(iobuf, AVIO_BUF, 0, d, src_read, NULL, src_seek);
    d->fmt = avformat_alloc_context();
    if (!iobuf || !io || !d->fmt)
        return MO_ERR_NO_MEMORY;
    d->fmt->pb = io;
    d->fmt->flags |= AVFMT_FLAG_CUSTOM_IO;
    if (avformat_open_input(&d->fmt, NULL, NULL, NULL) < 0)
        return -9; /* NOT_SUPPORTED: neznamy nebo poskozeny format */
    if (avformat_find_stream_info(d->fmt, NULL) < 0)
        return -10; /* CORRUPTED */
    wbuf w;
    w_init(&w, out, cap);
    int64_t dur = d->fmt->duration;
    w_u64(&w, dur > 0 ? (uint64_t)dur : 0);
    unsigned n = d->fmt->nb_streams > MAX_STREAMS ? MAX_STREAMS : d->fmt->nb_streams;
    w_u8(&w, (uint8_t)n);
    for (unsigned i = 0; i < n; i++) {
        AVStream *s = d->fmt->streams[i];
        AVCodecParameters *p = s->codecpar;
        uint8_t kind = p->codec_type == AVMEDIA_TYPE_VIDEO ? KIND_VIDEO : p->codec_type == AVMEDIA_TYPE_AUDIO ? KIND_AUDIO : KIND_SUBTITLE;
        w_u8(&w, kind);
        w_str(&w, avcodec_get_name(p->codec_id));
        w_u32(&w, kind == KIND_VIDEO ? (uint32_t)p->width : 0);
        w_u32(&w, kind == KIND_VIDEO ? (uint32_t)p->height : 0);
        AVRational fr = s->avg_frame_rate.num > 0 ? s->avg_frame_rate : s->r_frame_rate;
        w_u32(&w, kind == KIND_VIDEO && fr.num > 0 ? (uint32_t)fr.num : 0);
        w_u32(&w, kind == KIND_VIDEO && fr.den > 0 ? (uint32_t)fr.den : 0);
        w_u32(&w, kind == KIND_AUDIO ? (uint32_t)p->sample_rate : 0);
        w_u8(&w, kind == KIND_AUDIO ? (uint8_t)p->ch_layout.nb_channels : 0);
        AVDictionaryEntry *lang = av_dict_get(s->metadata, "language", NULL, 0);
        w_str(&w, lang ? lang->value : "");
        w_u8(&w, avcodec_find_decoder(p->codec_id) != NULL);
    }
    if (!w.ok)
        return MO_ERR_BUFFER_TOO_SMALL;
    *len = w.len;
    return 0;
}

/* ------------------------------------------------------------------ */
/* SELECT: otevreni dekoderu */

static AVCodecContext *open_decoder(dec *d, int idx) {
    AVStream *s = d->fmt->streams[idx];
    const AVCodec *c = avcodec_find_decoder(s->codecpar->codec_id);
    if (!c)
        return NULL;
    AVCodecContext *ctx = avcodec_alloc_context3(c);
    if (!ctx || avcodec_parameters_to_context(ctx, s->codecpar) < 0) {
        avcodec_free_context(&ctx);
        return NULL;
    }
    ctx->pkt_timebase = s->time_base;
    /* Vlakna podle poctu CPU (FFmpeg cte sysconf). */
    ctx->thread_count = 0;
    if (avcodec_open2(ctx, c, NULL) < 0) {
        avcodec_free_context(&ctx);
        return NULL;
    }
    return ctx;
}

static int select_streams(dec *d, int32_t v, int32_t a) {
    if (!d->fmt)
        return -16; /* BAD_STATE */
    int n = (int)d->fmt->nb_streams;
    if ((v >= 0 && (v >= n || d->fmt->streams[v]->codecpar->codec_type != AVMEDIA_TYPE_VIDEO)) ||
        (a >= 0 && (a >= n || d->fmt->streams[a]->codecpar->codec_type != AVMEDIA_TYPE_AUDIO)))
        return MO_ERR_INVALID_ARGS;
    avcodec_free_context(&d->vdec);
    avcodec_free_context(&d->adec);
    d->vi = v;
    d->ai = a;
    if (v >= 0 && !(d->vdec = open_decoder(d, v)))
        return -9;
    if (a >= 0 && !(d->adec = open_decoder(d, a)))
        return -9;
    for (int i = 0; i < n; i++)
        d->fmt->streams[i]->discard = (i == v || i == a) ? AVDISCARD_DEFAULT : AVDISCARD_ALL;
    return 0;
}

/* ------------------------------------------------------------------ */
/* POOL a AUDIO */

static int set_pool(dec *d, rbuf *r, mo_handle_t *hs, size_t nh) {
    unsigned n = r_u8(r);
    if (n == 0 || n > MAX_POOL || nh != n + 1)
        return MO_ERR_INVALID_ARGS;
    for (int i = 0; i < d->npool; i++)
        mo_vmo_unmap(d->pool[i].map, d->pool[i].map_len);
    d->npool = 0;
    for (unsigned i = 0; i < n; i++) {
        bufdesc bd;
        if (!r_bufdesc(r, &bd) || bd.fourcc != FOURCC_NV12 || bd.planes != 2 || bd.width == 0 || bd.height == 0)
            return MO_ERR_INVALID_ARGS;
        uint64_t end_y = bd.offset[0] + (uint64_t)bd.pitch[0] * bd.height;
        uint64_t end_uv = bd.offset[1] + (uint64_t)bd.pitch[1] * ((bd.height + 1) / 2);
        uint64_t need = end_y > end_uv ? end_y : end_uv;
        void *map = NULL;
        int rc = mo_vmo_map(hs[i], 0, need, 1, &map);
        mo_handle_close(hs[i]);
        if (rc < 0)
            return rc; /* VMO mensi nez popis: mapovani selze */
        d->pool[i] = (vbuf){.map = map, .map_len = need, .desc = bd, .free = 1};
        d->npool = (int)i + 1;
    }
    if (!r_end(r))
        return MO_ERR_INVALID_ARGS;
    d->frame_fence = hs[n];
    uint64_t v = 0;
    if (mo_fence_value(d->frame_fence, &v) == 0)
        d->serial = v;
    return 0;
}

static int set_audio(dec *d, rbuf *r, mo_handle_t *hs, size_t nh) {
    uint32_t rate = r_u32(r);
    uint8_t ch = r_u8(r), sample = r_u8(r);
    uint32_t cap = r_u32(r);
    if (!r_end(r) || nh != 2)
        return MO_ERR_INVALID_ARGS;
    /* s16 stereo (audioproto::Sample::S16 = 2). */
    if (ch != 2 || sample != 2 || rate == 0 || cap == 0)
        return -9;
    uint64_t len = RING_DATA + (uint64_t)cap * 4;
    void *map = NULL;
    int rc = mo_vmo_map(hs[0], 0, len, 1, &map);
    mo_handle_close(hs[0]);
    if (rc < 0)
        return rc;
    uint8_t *m = map;
    uint32_t c, fb;
    memcpy(&c, m + RING_CAPACITY, 4);
    memcpy(&fb, m + RING_FRAME, 4);
    if (c != cap || fb != 4) {
        mo_vmo_unmap(map, len);
        return MO_ERR_INVALID_ARGS;
    }
    d->ring = m;
    d->ring_len = len;
    d->ring_cap = cap;
    d->ring_event = hs[1];
    d->ring_rate = rate;
    return 0;
}

/* ------------------------------------------------------------------ */
/* Vystup */

static int free_slot(dec *d) {
    for (int i = 0; i < d->npool; i++)
        if (d->pool[i].free)
            return i;
    return -1;
}

static int64_t to_us(int64_t ts, AVRational tb, int64_t start) {
    if (ts == AV_NOPTS_VALUE)
        return 0;
    if (start != AV_NOPTS_VALUE)
        ts -= start;
    return av_rescale_q(ts, tb, AV_TIME_BASE_Q);
}

/* Snimek do volneho bufferu (NV12 ve velikosti bufferu). */
static int out_video(dec *d, AVFrame *f) {
    int slot = free_slot(d);
    if (slot < 0)
        return 0;
    vbuf *b = &d->pool[slot];
    d->sws = sws_getCachedContext(d->sws, f->width, f->height, f->format, (int)b->desc.width, (int)b->desc.height, AV_PIX_FMT_NV12,
                                  SWS_BILINEAR, NULL, NULL, NULL);
    if (!d->sws)
        return -1;
    uint8_t *dst[4] = {b->map + b->desc.offset[0], b->map + b->desc.offset[1], NULL, NULL};
    int stride[4] = {(int)b->desc.pitch[0], (int)b->desc.pitch[1], 0, 0};
    sws_scale(d->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, dst, stride);
    AVStream *s = d->fmt->streams[d->vi];
    int64_t pts = to_us(f->best_effort_timestamp, s->time_base, s->start_time);
    int64_t dur = f->duration > 0 ? av_rescale_q(f->duration, s->time_base, AV_TIME_BASE_Q) : 0;
    if (dur <= 0 && s->avg_frame_rate.num > 0)
        dur = av_rescale_q(1, av_inv_q(s->avg_frame_rate), AV_TIME_BASE_Q);
    b->free = 0;
    d->serial++;
    mo_fence_signal(d->frame_fence, d->serial);
    ev_frame(d, slot, pts, (uint32_t)(dur > 0 ? dur : 40000), d->serial);
    return 1;
}

static uint64_t ring_load(dec *d, int off) {
    return __atomic_load_n((uint64_t *)(d->ring + off), __ATOMIC_ACQUIRE);
}

/* Zapise cekajici zvuk do kruhu; vraci 1, kdyz je vse zapsane. */
static int out_audio(dec *d) {
    if (!d->ring)
        return 1;
    while (d->apend_off < d->apend_len) {
        uint64_t wp = ring_load(d, RING_WRITE), rp = ring_load(d, RING_READ);
        uint64_t used = wp - rp;
        if (used > d->ring_cap)
            used = d->ring_cap; /* nesmyslna pozice od druhe strany = plno */
        uint64_t room = d->ring_cap - used;
        if (room == 0)
            return 0;
        if (d->anchor_pending) {
            uint8_t b[32];
            wbuf w;
            w_init(&w, b, sizeof b);
            w_u32(&w, EV_AUDIO_BASE);
            w_u64(&w, wp);
            w_u64(&w, d->apend_pts < 0 ? 0 : (uint64_t)d->apend_pts);
            send_msg(d, &w);
            d->anchor_pending = 0;
        }
        uint64_t frames = (d->apend_len - d->apend_off) / 4;
        if (frames > room)
            frames = room;
        uint64_t idx = wp % d->ring_cap;
        uint64_t first = d->ring_cap - idx < frames ? d->ring_cap - idx : frames;
        memcpy(d->ring + RING_DATA + idx * 4, d->apend + d->apend_off, first * 4);
        memcpy(d->ring + RING_DATA, d->apend + d->apend_off + first * 4, (frames - first) * 4);
        d->apend_off += frames * 4;
        __atomic_store_n((uint64_t *)(d->ring + RING_WRITE), wp + frames, __ATOMIC_RELEASE);
    }
    return 1;
}

/* Prevede dekodovany zvuk na s16 stereo ve frekvenci kruhu. */
static int convert_audio(dec *d, AVFrame *f) {
    if (!d->swr) {
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        if (swr_alloc_set_opts2(&d->swr, &stereo, AV_SAMPLE_FMT_S16, (int)d->ring_rate, &f->ch_layout, f->format, f->sample_rate, 0, NULL) < 0 ||
            swr_init(d->swr) < 0)
            return -1;
    }
    int max = swr_get_out_samples(d->swr, f->nb_samples);
    if (max <= 0)
        return 0;
    free(d->apend);
    d->apend = malloc((size_t)max * 4);
    if (!d->apend)
        return -1;
    uint8_t *out[1] = {d->apend};
    int n = swr_convert(d->swr, out, max, (const uint8_t **)f->extended_data, f->nb_samples);
    if (n < 0)
        return -1;
    AVStream *s = d->fmt->streams[d->ai];
    d->apend_pts = to_us(f->best_effort_timestamp, s->time_base, s->start_time);
    d->apend_len = (size_t)n * 4;
    d->apend_off = 0;
    return 0;
}

/* Precte dalsi snimek z dekoderu (video do vframe, zvuk do apend). */
static int receive(dec *d, AVCodecContext *c, int video) {
    AVFrame *f = video ? d->vframe : d->aframe;
    for (;;) {
        int r = avcodec_receive_frame(c, f);
        if (r < 0)
            return r;
        AVStream *s = d->fmt->streams[video ? d->vi : d->ai];
        int64_t pts = to_us(f->best_effort_timestamp, s->time_base, s->start_time);
        if (d->skip_until > 0 && pts < d->skip_until) {
            av_frame_unref(f);
            continue;
        }
        if (video) {
            d->vframe_ready = 1;
            return 0;
        }
        int cr = d->ring ? convert_audio(d, f) : 0;
        av_frame_unref(f);
        return cr;
    }
}

/* Dekoduje, co jde; vraci 0 nebo chybu. */
static int produce(dec *d) {
    for (;;) {
        if (d->vframe_ready) {
            int r = out_video(d, d->vframe);
            if (r < 0)
                return -24;
            if (r == 0)
                return 0; /* zadny volny buffer */
            av_frame_unref(d->vframe);
            d->vframe_ready = 0;
        }
        if (d->apend_off < d->apend_len && !out_audio(d))
            return 0; /* kruh je plny */
        /* Dalsi vystup z dekoderu, ktery uz neco ma. */
        if (d->vdec && receive(d, d->vdec, 1) == 0)
            continue;
        if (d->adec && receive(d, d->adec, 0) == 0)
            continue;
        if (d->eof) {
            if (!d->end_sent) {
                d->end_sent = 1;
                ev_simple(d, EV_END);
            }
            return 0;
        }
        int r = av_read_frame(d->fmt, d->pkt);
        if (r == AVERROR_EOF) {
            d->eof = 1;
            if (d->vdec)
                avcodec_send_packet(d->vdec, NULL);
            if (d->adec)
                avcodec_send_packet(d->adec, NULL);
            continue;
        }
        if (r < 0)
            return -24; /* IO_ERROR */
        if (d->pkt->stream_index == d->vi && d->vdec)
            avcodec_send_packet(d->vdec, d->pkt);
        else if (d->pkt->stream_index == d->ai && d->adec)
            avcodec_send_packet(d->adec, d->pkt);
        av_packet_unref(d->pkt);
    }
}

/* ------------------------------------------------------------------ */
/* SEEK */

static int seek(dec *d, uint64_t pts_us, int exact, uint64_t *actual) {
    if (!d->fmt)
        return -16;
    int64_t ts = (int64_t)pts_us;
    if (d->fmt->start_time != AV_NOPTS_VALUE)
        ts += d->fmt->start_time;
    if (avformat_seek_file(d->fmt, -1, INT64_MIN, ts, ts, AVSEEK_FLAG_BACKWARD) < 0 &&
        av_seek_frame(d->fmt, -1, ts, AVSEEK_FLAG_BACKWARD) < 0)
        return -24;
    if (d->vdec)
        avcodec_flush_buffers(d->vdec);
    if (d->adec)
        avcodec_flush_buffers(d->adec);
    if (d->swr) {
        swr_free(&d->swr);
    }
    av_frame_unref(d->vframe);
    d->vframe_ready = 0;
    d->apend_len = d->apend_off = 0;
    d->eof = d->end_sent = 0;
    d->anchor_pending = d->ring != NULL;
    d->skip_until = exact ? (int64_t)pts_us : 0;
    *actual = pts_us;
    if (!exact && d->vdec) {
        /* Cas klicoveho snimku: dekodovat prvni snimek videa. */
        for (int guard = 0; guard < 4096 && !d->vframe_ready; guard++) {
            if (receive(d, d->vdec, 1) == 0)
                break;
            int r = av_read_frame(d->fmt, d->pkt);
            if (r < 0)
                break;
            if (d->pkt->stream_index == d->vi)
                avcodec_send_packet(d->vdec, d->pkt);
            else if (d->pkt->stream_index == d->ai && d->adec)
                avcodec_send_packet(d->adec, d->pkt);
            av_packet_unref(d->pkt);
        }
        if (d->vframe_ready) {
            AVStream *s = d->fmt->streams[d->vi];
            int64_t p = to_us(d->vframe->best_effort_timestamp, s->time_base, s->start_time);
            *actual = p > 0 ? (uint64_t)p : 0;
        }
    }
    return 0;
}

/* ------------------------------------------------------------------ */
/* Ridici smycka */

static void request(dec *d, const uint8_t *m, size_t len, mo_handle_t *hs, size_t nh, int *quit) {
    rbuf r;
    r_init(&r, m, len);
    uint32_t op = r_u32(&r);
    uint8_t out[MSG_MAX];
    size_t olen = 0;
    int st = 0;
    switch (op) {
    case OP_PROBE: {
        uint64_t size = r_u64(&r);
        if (!r_end(&r) || nh != 1 || d->fmt)
            st = MO_ERR_INVALID_ARGS;
        else
            st = probe(d, size, hs[0], out, sizeof out, &olen);
        break;
    }
    case OP_SELECT: {
        int32_t v = r_i32(&r), a = r_i32(&r), s = r_i32(&r);
        st = (!r_end(&r) || s != -1) ? MO_ERR_INVALID_ARGS : select_streams(d, v, a);
        break;
    }
    case OP_POOL:
        st = set_pool(d, &r, hs, nh);
        break;
    case OP_AUDIO:
        st = set_audio(d, &r, hs, nh);
        break;
    case OP_PLAY:
        if (!d->vdec && !d->adec)
            st = -16;
        else if (d->vdec && d->npool == 0)
            st = -16;
        else {
            if (!d->playing && d->ring)
                d->anchor_pending = 1;
            d->playing = 1;
        }
        break;
    case OP_PAUSE:
        d->playing = 0;
        break;
    case OP_SEEK: {
        uint64_t pts = r_u64(&r);
        uint8_t exact = r_u8(&r);
        uint64_t actual = 0;
        st = (!r_end(&r) || exact > 1) ? MO_ERR_INVALID_ARGS : seek(d, pts, exact, &actual);
        if (st == 0) {
            wbuf w;
            w_init(&w, out, sizeof out);
            w_u64(&w, actual);
            olen = w.len;
        }
        break;
    }
    case OP_RELEASE: {
        uint8_t b = r_u8(&r);
        if (!r_end(&r) || b >= d->npool)
            st = MO_ERR_INVALID_ARGS;
        else if (d->pool[b].free)
            st = -16;
        else
            d->pool[b].free = 1;
        break;
    }
    case OP_CLOSE:
        *quit = 1;
        break;
    default:
        st = -9;
    }
    reply(d, st, out, olen);
}

int main(void) {
    av_log_set_level(AV_LOG_ERROR);
    dec *d = calloc(1, sizeof *d);
    if (!d)
        return 1;
    d->ctl = mo_bootstrap();
    d->vi = d->ai = -1;
    d->pkt = av_packet_alloc();
    d->vframe = av_frame_alloc();
    d->aframe = av_frame_alloc();
    if (!d->ctl || !d->pkt || !d->vframe || !d->aframe)
        return 1;
    static uint8_t msg[MSG_MAX];
    for (;;) {
        if (d->playing) {
            int r = produce(d);
            if (r < 0) {
                ev_error(d, r);
                d->playing = 0;
            }
        }
        mo_handle_t hs[2] = {d->ctl, d->ring_event};
        uint32_t masks[2] = {MO_SIG_READABLE | MO_SIG_PEER_CLOSED, MO_SIG_SIGNALED};
        uint32_t obs[2] = {0, 0};
        int waits = (d->playing && d->ring && d->apend_off < d->apend_len) ? 2 : 1;
        if (mo_wait_many(hs, masks, obs, (size_t)waits, MO_DEADLINE_INFINITE) < 0)
            return 1;
        if (waits == 2 && (obs[1] & MO_SIG_SIGNALED))
            mo_event_reset(d->ring_event);
        for (;;) {
            size_t len = 0, nh = 0;
            mo_handle_t rh[MAX_POOL + 2];
            int r = mo_channel_read(d->ctl, msg, sizeof msg, &len, rh, MAX_POOL + 2, &nh, MO_DEADLINE_NOW);
            if (r == MO_ERR_SHOULD_WAIT)
                break;
            if (r < 0)
                return 0; /* mediad zavrel kanal */
            int quit = 0;
            request(d, msg, len, rh, nh, &quit);
            if (quit)
                return 0;
        }
    }
}
