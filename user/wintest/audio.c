/* Rezim zvuku testu Windows vrstvy (W4, docs/cs/windows-vrstva.md,
 * kapitola W4): ton pres winmm -> mmdevapi -> winemoaudio.drv -> audiod.
 *
 *   wine wintest.exe ton <soubor.wav>
 *
 * Zapise WAV (PCM 16 bit, 44,1 kHz, mono, 1 s tonu 440 Hz), prehraje ho
 * synchronne pres PlaySound a vypise "ton: ok <ms>" (doba prehravani:
 * skutecne prehrani trva aspon delku zaznamu, bez ovladace PlaySound
 * selze hned) nebo "ton: chyba ...". Kod konce 0 = prehrano. */

#include <math.h>
#include <stdio.h>
#include <string.h>
#include <windows.h>
#include <mmsystem.h>

#define RATE 44100
#define SECONDS 1

static void put16(unsigned char *p, unsigned v)
{
    p[0] = v & 0xff;
    p[1] = (v >> 8) & 0xff;
}

static void put32(unsigned char *p, unsigned v)
{
    put16(p, v & 0xffff);
    put16(p + 2, v >> 16);
}

/* WAV s tonem 440 Hz; 0 = zapsano. */
static int write_wav(const char *path)
{
    static unsigned char buf[44 + RATE * SECONDS * 2];
    unsigned data = RATE * SECONDS * 2, i;
    FILE *f;

    memcpy(buf, "RIFF", 4);
    put32(buf + 4, 36 + data);
    memcpy(buf + 8, "WAVEfmt ", 8);
    put32(buf + 16, 16);
    put16(buf + 20, 1);         /* PCM */
    put16(buf + 22, 1);         /* mono */
    put32(buf + 24, RATE);
    put32(buf + 28, RATE * 2);
    put16(buf + 32, 2);
    put16(buf + 34, 16);
    memcpy(buf + 36, "data", 4);
    put32(buf + 40, data);
    for (i = 0; i < RATE * SECONDS; i++)
    {
        short v = (short)(sin(2.0 * 3.14159265358979 * 440.0 * i / RATE) * 8000.0);
        put16(buf + 44 + i * 2, (unsigned short)v);
    }
    if (!(f = fopen(path, "wb"))) return 1;
    if (fwrite(buf, 1, sizeof(buf), f) != sizeof(buf))
    {
        fclose(f);
        return 1;
    }
    return fclose(f) ? 1 : 0;
}

int audio_tone(const char *path)
{
    DWORD start, ms;

    if (write_wav(path))
    {
        printf("ton: chyba zapisu %s\n", path);
        return 1;
    }
    printf("ton: zarizeni vystupu %u\n", waveOutGetNumDevs());
    fflush(stdout);
    start = GetTickCount();
    if (!PlaySoundA(path, NULL, SND_FILENAME | SND_SYNC | SND_NODEFAULT))
    {
        printf("ton: chyba PlaySound %lu\n", GetLastError());
        return 2;
    }
    ms = GetTickCount() - start;
    printf("ton: ok %lu\n", ms);
    fflush(stdout);
    return 0;
}
