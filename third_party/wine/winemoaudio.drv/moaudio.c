/*
 * Zvukovy ovladac Wine pro MagnumOpus (mmdevapi -> audiod pres moaudio)
 *
 * Copyright 2026 MagnumOpus-OS
 * Postaveno podle dlls/wineoss.drv/oss.c (Copyright 2011 Andrew Eikum for
 * CodeWeavers, 2022 Huw Davies).
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin St, Fifth Floor, Boston, MA 02110-1301, USA
 */

/* Ovladac zvuku W4 (docs/cs/windows-vrstva.md, kapitola W4):
 * - jeden vystup a jeden vstup, oba na relaci zvuku procesu (role Audio,
 *   moaudio.h); bez relace je ovladac nedostupny (mmdevapi zkusi dalsi),
 * - data aplikace jdou do mistniho bufferu (jako wineoss), vlakno casovace
 *   mmdevapi je kazdou periodu presune do sdileneho kruhu audiod,
 * - zamek streamu drzi jen praci s pameti: moaudio_put/moaudio_read
 *   necekaji a sluzbu nevolaji. Volani sluzby (start, stop, hlasitost)
 *   dela vlakno casovace az po odemceni: blokujici volani nesmi drzet
 *   zamek, na kterem cekaji vlakna aplikace a mmdevapi,
 * - otevreni vstupu muze cekat na rozhodnuti uzivatele v systemovem dialogu
 *   povoleni mikrofonu (nahravani jen s [audio] capture v manifestu). */

#if 0
#pragma makedep unix
#endif

#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <moaudio.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "winternl.h"
#include "initguid.h"
#include "audioclient.h"
#include "mmddk.h"

#include "wine/debug.h"
#include "wine/unixlib.h"

#include "../mmdevapi/unixlib.h"

WINE_DEFAULT_DEBUG_CHANNEL(moaudio);

struct mo_stream
{
    WAVEFORMATEX *fmt;
    EDataFlow flow;
    UINT flags;
    AUDCLNT_SHAREMODE share;
    HANDLE event;

    int id;                 /* stream moaudio */
    UINT32 ring_frames;     /* velikost sdileneho kruhu */

    BOOL playing, mute, please_quit;
    BOOL svc_started;       /* sluzba stream spustila (jen vlakno casovace) */
    UINT32 volume, svc_volume; /* hlasitost 0-100 chtena / u sluzby */
    UINT64 written_frames, last_pos_frames;
    UINT32 period_frames, bufsize_frames, held_frames, tmp_buffer_frames;
    UINT32 in_ring_frames;  /* snimky predane do kruhu, ktere sluzba jeste neodebrala */
    UINT32 lcl_offs_frames; /* posun v local_buffer, kde zacinaji platna data */
    REFERENCE_TIME period;

    BYTE *local_buffer, *tmp_buffer;
    INT32 getbuf_last; /* < 0 pri pouziti tmp_buffer */

    pthread_mutex_t lock;
};

static const REFERENCE_TIME def_period = 100000;
static const REFERENCE_TIME min_period = 50000;

/* Jmena koncovych bodu a jmeno zarizeni (jen jedno). */
static const WCHAR out_name[] = {'Z','v','u','k',' ','M','a','g','n','u','m','O','p','u','s',0};
static const WCHAR in_name[] = {'M','i','k','r','o','f','o','n',' ','M','a','g','n','u','m','O','p','u','s',0};
static const char device_name[] = "mo";

static NTSTATUS mo_not_implemented(void *args)
{
    return STATUS_SUCCESS;
}

static void mo_lock(struct mo_stream *stream)
{
    pthread_mutex_lock(&stream->lock);
}

static void mo_unlock(struct mo_stream *stream)
{
    pthread_mutex_unlock(&stream->lock);
}

static NTSTATUS mo_unlock_result(struct mo_stream *stream, HRESULT *result, HRESULT value)
{
    *result = value;
    mo_unlock(stream);
    return STATUS_SUCCESS;
}

static struct mo_stream *handle_get_stream(stream_handle h)
{
    return (struct mo_stream *)(UINT_PTR)h;
}

static NTSTATUS mo_process_attach(void *args)
{
    return STATUS_SUCCESS;
}

static NTSTATUS mo_main_loop(void *args)
{
    struct main_loop_params *params = args;
    NtSetEvent(params->event, NULL);
    return STATUS_SUCCESS;
}

/* Bez relace zvuku (proces bez role Audio, MO_AUDIO odebrane) ovladac
 * nefunguje. */
static NTSTATUS mo_test_connect(void *args)
{
    struct test_connect_params *params = args;
    params->priority = moaudio_available() ? Priority_Preferred : Priority_Unavailable;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_get_endpoint_ids(void *args)
{
    struct get_endpoint_ids_params *params = args;
    const WCHAR *name = params->flow == eCapture ? in_name : out_name;
    unsigned int name_bytes = (wcslen(name) + 1) * sizeof(WCHAR);
    unsigned int dev_bytes = (sizeof(device_name) + 1) & ~1;
    unsigned int needed = sizeof(struct endpoint) + name_bytes + dev_bytes;

    params->num = 1;
    params->default_idx = 0;
    if (needed > params->size)
    {
        params->size = needed;
        params->result = HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
        return STATUS_SUCCESS;
    }
    params->endpoints[0].name = sizeof(struct endpoint);
    memcpy((char *)params->endpoints + sizeof(struct endpoint), name, name_bytes);
    params->endpoints[0].device = sizeof(struct endpoint) + name_bytes;
    memcpy((char *)params->endpoints + sizeof(struct endpoint) + name_bytes, device_name, sizeof(device_name));
    params->result = S_OK;
    return STATUS_SUCCESS;
}

/* Format Wine -> MOAUDIO_SAMPLE_* (-1 = nepodporovany). */
static int mo_sample(const WAVEFORMATEX *fmt)
{
    const WAVEFORMATEXTENSIBLE *fmtex = (const WAVEFORMATEXTENSIBLE *)fmt;
    BOOL ext = fmt->wFormatTag == WAVE_FORMAT_EXTENSIBLE && fmt->cbSize >= sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);

    if (fmt->nChannels == 0 || fmt->nChannels > 8 || fmt->nSamplesPerSec < 8000 || fmt->nSamplesPerSec > 192000)
        return -1;
    if (fmt->nBlockAlign != fmt->nChannels * fmt->wBitsPerSample / 8)
        return -1;
    if (fmt->wFormatTag == WAVE_FORMAT_PCM || (ext && IsEqualGUID(&fmtex->SubFormat, &KSDATAFORMAT_SUBTYPE_PCM)))
    {
        switch (fmt->wBitsPerSample)
        {
        case 8: return MOAUDIO_SAMPLE_U8;
        case 16: return MOAUDIO_SAMPLE_S16;
        case 32: return MOAUDIO_SAMPLE_S32;
        }
        return -1;
    }
    if ((fmt->wFormatTag == WAVE_FORMAT_IEEE_FLOAT || (ext && IsEqualGUID(&fmtex->SubFormat, &KSDATAFORMAT_SUBTYPE_IEEE_FLOAT)))
        && fmt->wBitsPerSample == 32)
        return MOAUDIO_SAMPLE_F32;
    return -1;
}

static WAVEFORMATEXTENSIBLE *clone_format(const WAVEFORMATEX *fmt)
{
    WAVEFORMATEXTENSIBLE *ret;
    size_t size = fmt->wFormatTag == WAVE_FORMAT_EXTENSIBLE ? sizeof(WAVEFORMATEXTENSIBLE) : sizeof(WAVEFORMATEX);

    if (!(ret = malloc(size))) return NULL;
    memcpy(ret, fmt, size);
    ret->Format.cbSize = size - sizeof(WAVEFORMATEX);
    return ret;
}

static HRESULT check_format(AUDCLNT_SHAREMODE share, const WAVEFORMATEX *fmt)
{
    if (!fmt) return E_POINTER;
    if (share == AUDCLNT_SHAREMODE_EXCLUSIVE) return AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED;
    if (mo_sample(fmt) < 0) return AUDCLNT_E_UNSUPPORTED_FORMAT;
    return S_OK;
}

static NTSTATUS mo_create_stream(void *args)
{
    struct create_stream_params *params = args;
    struct mo_stream *stream;
    WAVEFORMATEXTENSIBLE *fmtex;
    UINT32 period_frames = 0, queued = 0, cap = 0;
    SIZE_T size;
    int id;

    params->result = check_format(params->share, params->fmt);
    if (FAILED(params->result)) return STATUS_SUCCESS;

    /* Bez zamku: otevreni vstupu muze cekat na dialog povoleni. */
    id = moaudio_open(params->flow == eCapture ? MOAUDIO_INPUT : MOAUDIO_OUTPUT, params->fmt->nSamplesPerSec,
                      params->fmt->nChannels, mo_sample(params->fmt), (UINT32)(params->period / 10) * 2, &period_frames);
    if (id < 0)
    {
        WARN("moaudio_open: %d\n", id);
        /* ACCESS_DENIED (nahravani nepovoleno) = jako odepreny mikrofon. */
        params->result = params->flow == eCapture ? E_ACCESSDENIED : AUDCLNT_E_DEVICE_INVALIDATED;
        return STATUS_SUCCESS;
    }
    moaudio_fill(id, &queued, &cap);

    if (!(stream = calloc(1, sizeof(*stream))))
    {
        moaudio_close(id);
        params->result = E_OUTOFMEMORY;
        return STATUS_SUCCESS;
    }
    stream->id = id;
    stream->ring_frames = cap;
    stream->flow = params->flow;
    stream->volume = stream->svc_volume = 100;
    pthread_mutex_init(&stream->lock, NULL);

    if (!(fmtex = clone_format(params->fmt)))
    {
        params->result = E_OUTOFMEMORY;
        goto exit;
    }
    stream->fmt = &fmtex->Format;
    stream->period = params->period;
    stream->period_frames = (UINT64)params->fmt->nSamplesPerSec * params->period / 10000000;
    if (!stream->period_frames) stream->period_frames = 1;
    stream->bufsize_frames = (UINT64)params->duration * params->fmt->nSamplesPerSec / 10000000;
    if (stream->bufsize_frames < stream->period_frames * 2) stream->bufsize_frames = stream->period_frames * 2;
    size = stream->bufsize_frames * params->fmt->nBlockAlign;
    if (NtAllocateVirtualMemory(GetCurrentProcess(), (void **)&stream->local_buffer, 0, &size, MEM_COMMIT, PAGE_READWRITE))
    {
        params->result = E_OUTOFMEMORY;
        goto exit;
    }
    stream->share = params->share;
    stream->flags = params->flags;
    TRACE("stream %d: %u Hz, %u kanalu, perioda %u, buffer %u, kruh %u\n", id, params->fmt->nSamplesPerSec,
          params->fmt->nChannels, stream->period_frames, stream->bufsize_frames, cap);

exit:
    if (FAILED(params->result))
    {
        moaudio_close(id);
        if (stream->local_buffer)
        {
            size = 0;
            NtFreeVirtualMemory(GetCurrentProcess(), (void **)&stream->local_buffer, &size, MEM_RELEASE);
        }
        pthread_mutex_destroy(&stream->lock);
        free(stream->fmt);
        free(stream);
    }
    else
    {
        *params->channel_count = params->fmt->nChannels;
        *params->stream = (stream_handle)(UINT_PTR)stream;
    }
    return STATUS_SUCCESS;
}

static NTSTATUS mo_release_stream(void *args)
{
    struct release_stream_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    SIZE_T size;

    if (params->timer_thread)
    {
        stream->please_quit = TRUE;
        NtWaitForSingleObject(params->timer_thread, FALSE, NULL);
        NtClose(params->timer_thread);
    }
    /* Vlakno casovace skoncilo: zavreni (volani sluzby) bez zamku. */
    moaudio_close(stream->id);
    if (stream->local_buffer)
    {
        size = 0;
        NtFreeVirtualMemory(GetCurrentProcess(), (void **)&stream->local_buffer, &size, MEM_RELEASE);
    }
    if (stream->tmp_buffer)
    {
        size = 0;
        NtFreeVirtualMemory(GetCurrentProcess(), (void **)&stream->tmp_buffer, &size, MEM_RELEASE);
    }
    free(stream->fmt);
    pthread_mutex_destroy(&stream->lock);
    free(stream);
    params->result = S_OK;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_start(void *args)
{
    struct start_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    if ((stream->flags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK) && !stream->event)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_EVENTHANDLE_NOT_SET);
    if (stream->playing)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_NOT_STOPPED);
    /* START u sluzby posle vlakno casovace (mimo zamek). */
    stream->playing = TRUE;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_stop(void *args)
{
    struct stop_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    if (!stream->playing)
        return mo_unlock_result(stream, &params->result, S_FALSE);
    stream->playing = FALSE;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_reset(void *args)
{
    struct reset_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    if (stream->playing)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_NOT_STOPPED);
    if (stream->getbuf_last)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_BUFFER_OPERATION_PENDING);
    if (stream->flow == eRender)
    {
        stream->written_frames = 0;
        stream->last_pos_frames = 0;
    }
    else
        stream->written_frames += stream->held_frames;
    /* Snimky uz predane do kruhu dohraje sluzba po dalsim startu (kruh
     * vyprazdnuje jen sluzba). */
    stream->held_frames = 0;
    stream->lcl_offs_frames = 0;
    stream->in_ring_frames = 0;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static void silence_buffer(struct mo_stream *stream, BYTE *buffer, UINT32 frames)
{
    memset(buffer, mo_sample(stream->fmt) == MOAUDIO_SAMPLE_U8 ? 128 : 0, frames * stream->fmt->nBlockAlign);
}

/* Presun z mistniho bufferu do kruhu (zamek drzen; jen pamet). */
static void mo_write_data(struct mo_stream *stream)
{
    UINT32 queued = 0, cap = 0, advanced, limit, new_frames, write_offs, chunk;
    UINT32 ba = stream->fmt->nBlockAlign;
    BYTE *buf;
    int n;

    if (moaudio_fill(stream->id, &queued, &cap) < 0) return;
    if (queued > stream->in_ring_frames) queued = stream->in_ring_frames;
    advanced = stream->in_ring_frames - queued;
    if (advanced > stream->held_frames) advanced = stream->held_frames;
    stream->lcl_offs_frames = (stream->lcl_offs_frames + advanced) % stream->bufsize_frames;
    stream->held_frames -= advanced;
    stream->in_ring_frames = queued;

    /* Predstih v kruhu: tri periody (nejvic cely kruh). */
    limit = stream->period_frames * 3;
    if (limit > cap) limit = cap;
    while (stream->in_ring_frames < limit && stream->held_frames > stream->in_ring_frames)
    {
        new_frames = stream->held_frames - stream->in_ring_frames;
        write_offs = (stream->lcl_offs_frames + stream->in_ring_frames) % stream->bufsize_frames;
        chunk = stream->bufsize_frames - write_offs;
        if (chunk > new_frames) chunk = new_frames;
        if (chunk > limit - stream->in_ring_frames) chunk = limit - stream->in_ring_frames;
        buf = stream->local_buffer + write_offs * ba;
        if (stream->mute) silence_buffer(stream, buf, chunk);
        n = moaudio_put(stream->id, buf, chunk * ba);
        if (n <= 0) return;
        stream->in_ring_frames += n;
        if ((UINT32)n < chunk) return;
    }
}

/* Nahrana data z kruhu do mistniho bufferu (zamek drzen; jen pamet). */
static void mo_read_data(struct mo_stream *stream)
{
    UINT32 ba = stream->fmt->nBlockAlign;
    UINT32 pos = (stream->held_frames + stream->lcl_offs_frames) % stream->bufsize_frames;
    int n = moaudio_read(stream->id, stream->local_buffer + pos * ba, (stream->bufsize_frames - pos) * ba);

    if (n <= 0) return;
    stream->held_frames += n / ba;
    if (stream->held_frames > stream->bufsize_frames)
    {
        WARN("Overflow of unread data\n");
        stream->lcl_offs_frames = (stream->lcl_offs_frames + stream->held_frames) % stream->bufsize_frames;
        stream->held_frames = stream->bufsize_frames;
    }
}

static NTSTATUS mo_timer_loop(void *args)
{
    struct timer_loop_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    LARGE_INTEGER delay, now, next;
    BOOL do_start, do_stop;
    UINT32 volume;
    int adjust;

    mo_lock(stream);
    delay.QuadPart = -stream->period;
    NtQueryPerformanceCounter(&now, NULL);
    next.QuadPart = now.QuadPart + stream->period;

    while (!stream->please_quit)
    {
        if (stream->playing)
        {
            if (stream->flow == eRender && stream->held_frames)
                mo_write_data(stream);
            else if (stream->flow == eCapture && stream->svc_started)
                mo_read_data(stream);
        }
        do_start = stream->playing && !stream->svc_started && (stream->flow == eCapture || stream->in_ring_frames);
        do_stop = !stream->playing && stream->svc_started;
        volume = stream->volume;
        if (stream->event)
            NtSetEvent(stream->event, NULL);
        mo_unlock(stream);

        /* Volani sluzby bez zamku. */
        if (do_start && moaudio_start(stream->id) == 0)
        {
            mo_lock(stream);
            stream->svc_started = TRUE;
            mo_unlock(stream);
        }
        if (do_stop)
        {
            moaudio_stop(stream->id);
            mo_lock(stream);
            stream->svc_started = FALSE;
            mo_unlock(stream);
        }
        if (volume != stream->svc_volume && moaudio_volume(stream->id, volume) == 0)
            stream->svc_volume = volume;

        NtDelayExecution(FALSE, &delay);

        mo_lock(stream);
        NtQueryPerformanceCounter(&now, NULL);
        adjust = next.QuadPart - now.QuadPart;
        if (adjust > stream->period / 2) adjust = stream->period / 2;
        else if (adjust < -stream->period / 2) adjust = -stream->period / 2;
        delay.QuadPart = -(stream->period + adjust);
        next.QuadPart += stream->period;
    }
    mo_unlock(stream);
    return STATUS_SUCCESS;
}

static HRESULT ensure_tmp(struct mo_stream *stream, UINT32 frames)
{
    SIZE_T size;

    if (stream->tmp_buffer_frames >= frames) return S_OK;
    if (stream->tmp_buffer)
    {
        size = 0;
        NtFreeVirtualMemory(GetCurrentProcess(), (void **)&stream->tmp_buffer, &size, MEM_RELEASE);
        stream->tmp_buffer = NULL;
    }
    size = frames * stream->fmt->nBlockAlign;
    if (NtAllocateVirtualMemory(GetCurrentProcess(), (void **)&stream->tmp_buffer, 0, &size, MEM_COMMIT, PAGE_READWRITE))
    {
        stream->tmp_buffer_frames = 0;
        return E_OUTOFMEMORY;
    }
    stream->tmp_buffer_frames = frames;
    return S_OK;
}

static NTSTATUS mo_get_render_buffer(void *args)
{
    struct get_render_buffer_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    UINT32 write_pos, frames = params->frames;
    BYTE **data = params->data;
    HRESULT hr;

    mo_lock(stream);
    if (stream->getbuf_last)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_OUT_OF_ORDER);
    if (!frames)
        return mo_unlock_result(stream, &params->result, S_OK);
    if (stream->held_frames + frames > stream->bufsize_frames)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_BUFFER_TOO_LARGE);

    write_pos = (stream->lcl_offs_frames + stream->held_frames) % stream->bufsize_frames;
    if (write_pos + frames > stream->bufsize_frames)
    {
        if (FAILED(hr = ensure_tmp(stream, frames)))
            return mo_unlock_result(stream, &params->result, hr);
        *data = stream->tmp_buffer;
        stream->getbuf_last = -frames;
    }
    else
    {
        *data = stream->local_buffer + write_pos * stream->fmt->nBlockAlign;
        stream->getbuf_last = frames;
    }
    silence_buffer(stream, *data, frames);
    return mo_unlock_result(stream, &params->result, S_OK);
}

static void mo_wrap_buffer(struct mo_stream *stream, BYTE *buffer, UINT32 written_frames)
{
    UINT32 ba = stream->fmt->nBlockAlign;
    UINT32 write_offs = (stream->lcl_offs_frames + stream->held_frames) % stream->bufsize_frames;
    UINT32 chunk_bytes = (stream->bufsize_frames - write_offs) * ba;
    UINT32 written_bytes = written_frames * ba;

    if (written_bytes <= chunk_bytes)
        memcpy(stream->local_buffer + write_offs * ba, buffer, written_bytes);
    else
    {
        memcpy(stream->local_buffer + write_offs * ba, buffer, chunk_bytes);
        memcpy(stream->local_buffer, buffer + chunk_bytes, written_bytes - chunk_bytes);
    }
}

static NTSTATUS mo_release_render_buffer(void *args)
{
    struct release_render_buffer_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    UINT32 written_frames = params->written_frames;
    BYTE *buffer;

    mo_lock(stream);
    if (!written_frames)
    {
        stream->getbuf_last = 0;
        return mo_unlock_result(stream, &params->result, S_OK);
    }
    if (!stream->getbuf_last)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_OUT_OF_ORDER);
    if (written_frames > (UINT32)(stream->getbuf_last >= 0 ? stream->getbuf_last : -stream->getbuf_last))
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_INVALID_SIZE);

    if (stream->getbuf_last >= 0)
        buffer = stream->local_buffer + stream->fmt->nBlockAlign * ((stream->lcl_offs_frames + stream->held_frames) % stream->bufsize_frames);
    else
        buffer = stream->tmp_buffer;
    if (params->flags & AUDCLNT_BUFFERFLAGS_SILENT)
        silence_buffer(stream, buffer, written_frames);
    if (stream->getbuf_last < 0)
        mo_wrap_buffer(stream, buffer, written_frames);

    stream->held_frames += written_frames;
    stream->written_frames += written_frames;
    stream->getbuf_last = 0;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_capture_buffer(void *args)
{
    struct get_capture_buffer_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    UINT32 ba, chunk_bytes;
    HRESULT hr;

    mo_lock(stream);
    if (stream->getbuf_last)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_OUT_OF_ORDER);
    if (stream->held_frames < stream->period_frames)
    {
        *params->frames = 0;
        return mo_unlock_result(stream, &params->result, AUDCLNT_S_BUFFER_EMPTY);
    }
    *params->flags = 0;
    *params->frames = stream->period_frames;
    ba = stream->fmt->nBlockAlign;
    if (stream->lcl_offs_frames + *params->frames > stream->bufsize_frames)
    {
        if (FAILED(hr = ensure_tmp(stream, *params->frames)))
            return mo_unlock_result(stream, &params->result, hr);
        *params->data = stream->tmp_buffer;
        chunk_bytes = (stream->bufsize_frames - stream->lcl_offs_frames) * ba;
        memcpy(stream->tmp_buffer, stream->local_buffer + stream->lcl_offs_frames * ba, chunk_bytes);
        memcpy(stream->tmp_buffer + chunk_bytes, stream->local_buffer, *params->frames * ba - chunk_bytes);
    }
    else
        *params->data = stream->local_buffer + stream->lcl_offs_frames * ba;
    stream->getbuf_last = *params->frames;
    if (params->devpos)
        *params->devpos = stream->written_frames;
    if (params->qpcpos)
    {
        LARGE_INTEGER stamp, freq;
        NtQueryPerformanceCounter(&stamp, &freq);
        *params->qpcpos = (stamp.QuadPart * (INT64)10000000) / freq.QuadPart;
    }
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_release_capture_buffer(void *args)
{
    struct release_capture_buffer_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    UINT32 done = params->done;

    mo_lock(stream);
    if (!done)
    {
        stream->getbuf_last = 0;
        return mo_unlock_result(stream, &params->result, S_OK);
    }
    if (!stream->getbuf_last)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_OUT_OF_ORDER);
    if ((UINT32)stream->getbuf_last != done)
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_INVALID_SIZE);
    stream->written_frames += done;
    stream->held_frames -= done;
    stream->lcl_offs_frames = (stream->lcl_offs_frames + done) % stream->bufsize_frames;
    stream->getbuf_last = 0;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_is_format_supported(void *args)
{
    struct is_format_supported_params *params = args;
    params->result = check_format(params->share, params->fmt_in);
    return STATUS_SUCCESS;
}

/* Format mixu: 48 kHz, stereo, float (audiod prevadi format i frekvenci). */
static NTSTATUS mo_get_mix_format(void *args)
{
    struct get_mix_format_params *params = args;
    WAVEFORMATEXTENSIBLE *fmt = params->fmt;

    if (params->flow != eRender && params->flow != eCapture)
    {
        params->result = E_UNEXPECTED;
        return STATUS_SUCCESS;
    }
    fmt->Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    fmt->Format.nChannels = 2;
    fmt->Format.nSamplesPerSec = 48000;
    fmt->Format.wBitsPerSample = 32;
    fmt->Format.nBlockAlign = 8;
    fmt->Format.nAvgBytesPerSec = 48000 * 8;
    fmt->Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
    fmt->Samples.wValidBitsPerSample = 32;
    fmt->dwChannelMask = KSAUDIO_SPEAKER_STEREO;
    fmt->SubFormat = KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    params->result = S_OK;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_get_device_period(void *args)
{
    struct get_device_period_params *params = args;

    if (params->def_period) *params->def_period = def_period;
    if (params->min_period) *params->min_period = min_period;
    params->result = S_OK;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_get_buffer_size(void *args)
{
    struct get_buffer_size_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    *params->frames = stream->bufsize_frames;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_latency(void *args)
{
    struct get_latency_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    *params->latency = stream->period + 6666;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_current_padding(void *args)
{
    struct get_current_padding_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    *params->padding = stream->held_frames;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_next_packet_size(void *args)
{
    struct get_next_packet_size_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    *params->frames = stream->held_frames < stream->period_frames ? 0 : stream->period_frames;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_frequency(void *args)
{
    struct get_frequency_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    *params->freq = (UINT64)stream->fmt->nSamplesPerSec * stream->fmt->nBlockAlign;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_get_position(void *args)
{
    struct get_position_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    UINT64 *pos = params->pos;

    if (params->device)
    {
        params->result = E_NOTIMPL;
        return STATUS_SUCCESS;
    }
    mo_lock(stream);
    if (stream->flow == eRender)
    {
        /* Odebrane sluzbou = zapsane minus to, co jeste drzime. */
        *pos = stream->written_frames - stream->held_frames;
        if (*pos < stream->last_pos_frames) *pos = stream->last_pos_frames;
    }
    else
        *pos = stream->written_frames + stream->held_frames;
    stream->last_pos_frames = *pos;
    *pos *= stream->fmt->nBlockAlign;
    if (params->qpctime)
    {
        LARGE_INTEGER stamp, freq;
        NtQueryPerformanceCounter(&stamp, &freq);
        *params->qpctime = (stamp.QuadPart * (INT64)10000000) / freq.QuadPart;
    }
    return mo_unlock_result(stream, &params->result, S_OK);
}

/* Hlasitost: prumer kanalu (sluzba ma jednu hlasitost streamu); posle ji
 * vlakno casovace mimo zamek. */
static NTSTATUS mo_set_volumes(void *args)
{
    struct set_volumes_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);
    float sum = 0.0f;
    UINT16 i;

    for (i = 0; i < stream->fmt->nChannels; ++i)
        sum += params->master_volume * params->volumes[i] * params->session_volumes[i];
    sum /= stream->fmt->nChannels;
    if (sum < 0.0f) sum = 0.0f;
    if (sum > 1.0f) sum = 1.0f;

    mo_lock(stream);
    stream->mute = params->master_volume == 0.0f;
    stream->volume = (UINT32)(sum * 100.0f + 0.5f);
    mo_unlock(stream);
    return STATUS_SUCCESS;
}

static NTSTATUS mo_set_event_handle(void *args)
{
    struct set_event_handle_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    if (!(stream->flags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK))
        return mo_unlock_result(stream, &params->result, AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED);
    if (stream->event)
        return mo_unlock_result(stream, &params->result, HRESULT_FROM_WIN32(ERROR_INVALID_NAME));
    stream->event = params->event;
    return mo_unlock_result(stream, &params->result, S_OK);
}

static NTSTATUS mo_is_started(void *args)
{
    struct is_started_params *params = args;
    struct mo_stream *stream = handle_get_stream(params->stream);

    mo_lock(stream);
    return mo_unlock_result(stream, &params->result, stream->playing ? S_OK : S_FALSE);
}

static NTSTATUS mo_get_prop_value(void *args)
{
    struct get_prop_value_params *params = args;
    params->result = E_NOTIMPL;
    return STATUS_SUCCESS;
}

/* MIDI a aux: zadna zarizeni. */
static NTSTATUS mo_midi_init(void *args)
{
    struct midi_init_params *params = args;
    *params->err = MMSYSERR_NOERROR;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_midi_out_message(void *args)
{
    struct midi_out_message_params *params = args;
    params->notify->send_notify = FALSE;
    *params->err = params->msg == MODM_GETNUMDEVS ? 0 : MMSYSERR_BADDEVICEID;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_midi_in_message(void *args)
{
    struct midi_in_message_params *params = args;
    params->notify->send_notify = FALSE;
    *params->err = params->msg == MIDM_GETNUMDEVS ? 0 : MMSYSERR_BADDEVICEID;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_midi_notify_wait(void *args)
{
    struct midi_notify_wait_params *params = args;
    *params->quit = TRUE;
    params->notify->send_notify = FALSE;
    return STATUS_SUCCESS;
}

static NTSTATUS mo_aux_message(void *args)
{
    struct aux_message_params *params = args;
    *params->err = params->msg == AUXDM_GETNUMDEVS ? 0 : MMSYSERR_BADDEVICEID;
    return STATUS_SUCCESS;
}

const unixlib_entry_t __wine_unix_call_funcs[] =
{
    mo_process_attach,
    mo_not_implemented,
    mo_main_loop,
    mo_get_endpoint_ids,
    mo_create_stream,
    mo_release_stream,
    mo_start,
    mo_stop,
    mo_reset,
    mo_timer_loop,
    mo_get_render_buffer,
    mo_release_render_buffer,
    mo_get_capture_buffer,
    mo_release_capture_buffer,
    mo_is_format_supported,
    mo_not_implemented,
    mo_get_mix_format,
    mo_get_device_period,
    mo_get_buffer_size,
    mo_get_latency,
    mo_get_current_padding,
    mo_get_next_packet_size,
    mo_get_frequency,
    mo_get_position,
    mo_set_volumes,
    mo_set_event_handle,
    mo_not_implemented,
    mo_test_connect,
    mo_is_started,
    mo_get_prop_value,
    mo_not_implemented,
    mo_midi_init,
    mo_not_implemented,
    mo_midi_out_message,
    mo_midi_in_message,
    mo_midi_notify_wait,
    mo_aux_message,
};

C_ASSERT(ARRAYSIZE(__wine_unix_call_funcs) == funcs_count);
