/*
 * moaudio.h - zvuk aplikace pres audiod (libs/mowin, os/audio.rs; soucast
 * libc.a). Pouziva ho ovladac zvuku Wine (winemo.drv, mmdevapi); stejne
 * jadro ma mosdl_audio_* (SDL). Popis: docs/cs/zvuk.md, kapitola Relace
 * zvuku pro potomky, a docs/cs/windows-vrstva.md, kapitola W4.
 *
 * Relace: od runtime (role Audio od rodice nebo relace aplikace od initu).
 * Prava aplikace: prehravani; nahravani jen s [audio] capture v manifestu
 * a po povoleni uzivatelem v systemovem dialogu (OPEN vstupu do te doby
 * ceka).
 *
 * Zapis a cekani na misto bezi bez zamku knihovny; volajici ale nesmi pri
 * nich drzet vlastni zamky, ktere potrebuji jina vlakna (Wine: zamek
 * streamu mmdevapi).
 *
 * Navratove hodnoty int: 0 / id > 0 = uspech, zaporne = kod chyby jadra
 * (abi::Error, docs/cs/chyby.md).
 */
#ifndef MOAUDIO_H
#define MOAUDIO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Vzorky prokladane po kanalech, little endian (= MOSDL_SAMPLE_*). */
enum {
    MOAUDIO_SAMPLE_U8 = 0,
    MOAUDIO_SAMPLE_S16 = 1,
    MOAUDIO_SAMPLE_S32 = 2,
    MOAUDIO_SAMPLE_F32 = 3
};

/* Smer streamu (moaudio_open). */
enum {
    MOAUDIO_OUTPUT = 0,
    MOAUDIO_INPUT = 1
};

/* Ma proces relaci zvuku? 1 ano, 0 ne. */
int moaudio_available(void);
/* Otevre stream; vrati id > 0, nebo zaporny kod (ACCESS_DENIED =
   nahravani nepovoleno). *period_frames = takt sluzby ve snimcich. */
int moaudio_open(int dir, uint32_t rate, uint32_t channels, uint32_t sample, uint32_t latency_us, uint32_t *period_frames);
/* Zapise cele snimky; pri plnem bufferu ceka (nejdel timeout_ns, < 0 bez
   limitu; 0 = jen co se vejde, jinak TIMED_OUT). Stream se spusti, jakmile
   se buffer poprve naplni. */
int moaudio_write(int id, const void *data, uint32_t bytes, int64_t timeout_ns);
/* Zapise jen to, co se hned vejde (cele snimky), bez cekani a bez volani
   sluzby (stream nespousti); vrati pocet zapsanych snimku. Pro volajici,
   kteri pisou pod svym zamkem a moaudio_start volaji mimo nej. */
int moaudio_put(int id, const void *data, uint32_t bytes);
/* Precte dostupna nahrana data; vrati pocet bajtu. */
int moaudio_read(int id, void *data, uint32_t bytes);
/* Ceka, az se vejde `bytes` bajtu (0 = misto je). */
int moaudio_wait_space(int id, uint32_t bytes, int64_t timeout_ns);
/* Ceka na udalost streamu (misto / nova data); vyprseni neni chyba. */
int moaudio_wait(int id, int64_t timeout_ns);
int moaudio_start(int id);
int moaudio_stop(int id);
/* Hlasitost streamu 0-100 %. */
int moaudio_volume(int id, uint32_t percent);
/* *queued = snimky v bufferu (vystup: jeste nezahrane, vstup: k precteni),
   *capacity = velikost bufferu ve snimcich. */
int moaudio_fill(int id, uint32_t *queued, uint32_t *capacity);
/* *played = snimky, ktere uz zazneli, *written = snimky zapsane celkem. */
int moaudio_position(int id, uint64_t *played, uint64_t *written);
/* Zavre stream (zbytek se nedohrava). */
void moaudio_close(int id);

#ifdef __cplusplus
}
#endif

#endif
