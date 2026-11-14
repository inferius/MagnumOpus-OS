/*
 * Ovladac zvuku SDL pro MagnumOpus: streamy audiod pres libs/mosdl
 * (relace aplikace, include/mosdl.h). Kod MagnumOpus, licence zlib jako
 * SDL. Popis: docs/cs/sdl.md, docs/cs/zvuk.md.
 *
 * Prehravani: SDL plni blok (perioda sluzby) a PlayDevice ho zapise do
 * sdileneho kruhoveho bufferu (pri plnem bufferu ceka na misto).
 * Nahravani: OPEN vstupu muze cekat na rozhodnuti uzivatele (dialog
 * povoleni); zamitnuti = SDL_OpenAudioDevice selze. Jedno vychozi zarizeni
 * kazdeho smeru (vyber karty je vec systemu, ne aplikace).
 */
#include "SDL_internal.h"

#ifdef SDL_AUDIO_DRIVER_MAGNUMOPUS

#include <mosdl.h>

#include "../SDL_sysaudio.h"

/* Latence bufferu klienta: dost na plynule prehravani i v QEMU. */
#define MOS_LATENCY_US 50000

struct SDL_PrivateAudioData
{
    int stream;
    Uint8 *mixbuf;
};

static uint32_t MOS_Sample(SDL_AudioFormat f, SDL_AudioFormat *chosen)
{
    switch (f) {
    case SDL_AUDIO_U8:
        *chosen = SDL_AUDIO_U8;
        return MOSDL_SAMPLE_U8;
    case SDL_AUDIO_S32:
        *chosen = SDL_AUDIO_S32;
        return MOSDL_SAMPLE_S32;
    case SDL_AUDIO_F32:
        *chosen = SDL_AUDIO_F32;
        return MOSDL_SAMPLE_F32;
    default:
        /* Ostatni (S8, big endian) prevede SDL samo na S16. */
        *chosen = SDL_AUDIO_S16;
        return MOSDL_SAMPLE_S16;
    }
}

static bool MOS_OpenDevice(SDL_AudioDevice *device)
{
    SDL_AudioFormat fmt;
    uint32_t sample, frames = 0;
    int id;

    device->hidden = (struct SDL_PrivateAudioData *)SDL_calloc(1, sizeof(*device->hidden));
    if (!device->hidden) {
        return false;
    }
    sample = MOS_Sample(device->spec.format, &fmt);
    if (device->spec.channels < 1 || device->spec.channels > 8) {
        device->spec.channels = 2;
    }
    id = mosdl_audio_open(device->recording ? 1 : 0, (uint32_t)device->spec.freq, (uint32_t)device->spec.channels, sample, MOS_LATENCY_US, &frames);
    if (id < 0) {
        return SDL_SetError("magnumopus: zvuk nelze otevrit (chyba %d%s)", id, device->recording ? ", nahravani nepovoleno?" : "");
    }
    device->hidden->stream = id;
    device->spec.format = fmt;
    if (frames > 0) {
        device->sample_frames = (int)frames;
    }
    SDL_UpdatedAudioDeviceFormat(device);
    if (!device->recording) {
        device->hidden->mixbuf = (Uint8 *)SDL_malloc(device->buffer_size);
        if (!device->hidden->mixbuf) {
            return false;
        }
        SDL_memset(device->hidden->mixbuf, device->silence_value, device->buffer_size);
    } else if (mosdl_audio_start(id) < 0) {
        return SDL_SetError("magnumopus: nahravani nelze spustit");
    }
    return true;
}

static bool MOS_WaitDevice(SDL_AudioDevice *device)
{
    /* Cekani na misto dela PlayDevice (zapis ceka, kdyz je buffer plny). */
    return true;
}

static bool MOS_PlayDevice(SDL_AudioDevice *device, const Uint8 *buffer, int buflen)
{
    /* Nejdel 1 s: sluzba stoji (zadne zarizeni) = chyba zarizeni v SDL. */
    return mosdl_audio_write(device->hidden->stream, buffer, (uint32_t)buflen, 1000000000) >= 0;
}

static Uint8 *MOS_GetDeviceBuf(SDL_AudioDevice *device, int *buffer_size)
{
    return device->hidden->mixbuf;
}

static bool MOS_WaitRecordingDevice(SDL_AudioDevice *device)
{
    return mosdl_audio_wait(device->hidden->stream, 100000000) >= 0;
}

static int MOS_RecordDevice(SDL_AudioDevice *device, void *buffer, int buflen)
{
    int n = mosdl_audio_read(device->hidden->stream, buffer, (uint32_t)buflen);
    return n < 0 ? -1 : n;
}

static void MOS_FlushRecording(SDL_AudioDevice *device)
{
    Uint8 tmp[1024];
    while (mosdl_audio_read(device->hidden->stream, tmp, sizeof(tmp)) > 0) {
    }
}

static void MOS_CloseDevice(SDL_AudioDevice *device)
{
    if (device->hidden) {
        if (device->hidden->stream > 0) {
            mosdl_audio_close(device->hidden->stream);
        }
        SDL_free(device->hidden->mixbuf);
        SDL_free(device->hidden);
        device->hidden = NULL;
    }
}

static bool MOS_Init(SDL_AudioDriverImpl *impl)
{
    if (!mosdl_audio_available()) {
        return false; /* aplikace nema relaci zvuku: ovladac dummy */
    }
    impl->OpenDevice = MOS_OpenDevice;
    impl->WaitDevice = MOS_WaitDevice;
    impl->PlayDevice = MOS_PlayDevice;
    impl->GetDeviceBuf = MOS_GetDeviceBuf;
    impl->WaitRecordingDevice = MOS_WaitRecordingDevice;
    impl->RecordDevice = MOS_RecordDevice;
    impl->FlushRecording = MOS_FlushRecording;
    impl->CloseDevice = MOS_CloseDevice;
    impl->OnlyHasDefaultPlaybackDevice = true;
    impl->OnlyHasDefaultRecordingDevice = true;
    impl->HasRecordingSupport = true;
    return true;
}

AudioBootStrap MAGNUMOPUS_AUDIO_bootstrap = {
    "magnumopus", "MagnumOpus audiod", MOS_Init, false, false
};

#endif /* SDL_AUDIO_DRIVER_MAGNUMOPUS */
