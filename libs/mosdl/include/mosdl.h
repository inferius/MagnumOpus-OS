/*
 * mosdl.h - rozhrani C pro port SDL na MagnumOpus (libs/mosdl, soucast
 * libc.a). Popis: docs/cs/sdl.md.
 *
 * Jedno okno na proces (kanal okna ze startovni zpravy aplikace). Volat
 * z jednoho vlakna (SDL: hlavni vlakno videa); jen mosdl_wake smi volat
 * kterekoli vlakno.
 *
 * Navratove hodnoty int: 0 = uspech, zaporne = kod chyby jadra
 * (abi::Error, docs/cs/chyby.md).
 */
#ifndef MOSDL_H
#define MOSDL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Typ udalosti (mosdl_event.type). */
enum {
    MOSDL_EV_NONE = 0,
    /* a = kod klavesy evdev, b = akce (0 uvolneni, 1 stisk, 2 opakovani),
       c = znak podle rozlozeni (Unicode, 0 = zadny) */
    MOSDL_EV_KEY = 1,
    /* a = vlozeny znak (Unicode) */
    MOSDL_EV_TEXT = 2,
    /* a, b = poloha ukazatele v obsahu okna (logicke px) */
    MOSDL_EV_MOTION = 3,
    /* ukazatel opustil okno */
    MOSDL_EV_LEAVE = 4,
    /* a = tlacitko evdev (BTN_LEFT 0x110 ...), b = stisknuto, c, d = poloha */
    MOSDL_EV_BUTTON = 5,
    /* a = vodorovne kroky, b = svisle kroky (kladne nahoru) */
    MOSDL_EV_WHEEL = 6,
    /* a = 1 fokus ziskan, 0 ztracen */
    MOSDL_EV_FOCUS = 7,
    /* a, b = nova velikost obsahu (logicke px) */
    MOSDL_EV_RESIZE = 8,
    /* uzivatel zavira okno */
    MOSDL_EV_CLOSE = 9,
    /* kompozitor spadl, okno je zpet: prekreslit */
    MOSDL_EV_EXPOSE = 10,
    /* a, b = relativni pohyb pri zamku ukazatele */
    MOSDL_EV_RELMOTION = 11,
    /* a = 1 zamek ukazatele zapnut, 0 zrusen (i systemem) */
    MOSDL_EV_POINTER_LOCK = 12,
    /* zmenil se vystup okna (obrazovka, skala): mosdl_display_info */
    MOSDL_EV_DISPLAY = 13,
    /* a = stav okna (MOSDL_STATE_*) */
    MOSDL_EV_STATE = 14
};

/* Bity stavu okna (MOSDL_EV_STATE, guiproto::window::wstate). */
enum {
    MOSDL_STATE_MAXIMIZED = 1 << 1,
    MOSDL_STATE_FULLSCREEN = 1 << 2
};

/* Systemove tvary kurzoru (mosdl_set_cursor_shape, guiproto::cursor::Shape). */
enum {
    MOSDL_CURSOR_DEFAULT = 0,
    MOSDL_CURSOR_TEXT = 1,
    MOSDL_CURSOR_WAIT = 2,
    MOSDL_CURSOR_CROSSHAIR = 3,
    MOSDL_CURSOR_PROGRESS = 4,
    MOSDL_CURSOR_RESIZE_NWSE = 5,
    MOSDL_CURSOR_RESIZE_NESW = 6,
    MOSDL_CURSOR_RESIZE_EW = 7,
    MOSDL_CURSOR_RESIZE_NS = 8,
    MOSDL_CURSOR_MOVE = 9,
    MOSDL_CURSOR_NOT_ALLOWED = 10,
    MOSDL_CURSOR_POINTER = 11,
    MOSDL_CURSOR_HIDDEN = 12
};

/* Bity mosdl_event.mods (stejne jako inputproto::mods). */
enum {
    MOSDL_MOD_SHIFT = 1 << 0,
    MOSDL_MOD_CTRL = 1 << 1,
    MOSDL_MOD_ALT = 1 << 2,
    MOSDL_MOD_ALTGR = 1 << 3,
    MOSDL_MOD_META = 1 << 4,
    MOSDL_MOD_CAPS = 1 << 5,
    MOSDL_MOD_NUM = 1 << 6
};

typedef struct mosdl_event {
    uint32_t type;
    uint32_t mods;
    /* cas udalosti v ns (stejne hodiny jako CLOCK_MONOTONIC) */
    uint64_t time_ns;
    int32_t a, b, c, d;
    /* 1 = umely nebo vzdaleny vstup (ne mistni fyzicke zarizeni) */
    uint32_t synthetic;
    uint32_t reserved;
} mosdl_event;

/* Udaje o vystupu okna (pred otevrenim okna nuly). */
typedef struct mosdl_display {
    uint32_t screen_w, screen_h;   /* cela obrazovka, logicke px */
    uint32_t work_w, work_h;       /* pracovni plocha bez listy */
    uint32_t scale_pct;            /* skala vystupu v procentech (100 = 1x) */
    uint32_t refresh_mhz;          /* obnovovaci frekvence v mHz (0 = neznama) */
} mosdl_display;

/* Je proces spusten s oknem (ma kanal ke kompozitoru)? 1 ano, 0 ne. */
int mosdl_available(void);

/* Udaje o obrazovce: bez okna se okno na chvili nevytvari, vraci jen to,
   co zna (pred otevrenim okna obrazovku z pozdravu). */
int mosdl_display_info(mosdl_display *out);

/* Otevre okno w x h (0 = nechat na systemu) s titulkem (UTF-8). Skutecnou
   velikost vrati v *out_w, *out_h. Druhe okno: -NOT_SUPPORTED. */
int mosdl_window_open(uint32_t w, uint32_t h, const char *title, uint32_t *out_w, uint32_t *out_h);
int mosdl_window_set_title(const char *title);
/* Zadost o velikost (kompozitor odpovi udalosti RESIZE). */
int mosdl_window_set_size(uint32_t w, uint32_t h, uint32_t min_w, uint32_t min_h);
void mosdl_window_close(void);

/* Ukaze snimek: pixely B G R A po radcich (SDL_PIXELFORMAT_XRGB8888 /
   ARGB8888 na little endian), krok radku `pitch` bajtu. Snimek jine
   velikosti nez okno se orizne nebo doplni cernou. Ceka jen na uvolneni
   bufferu kompozitorem (prirozene tempo snimku). */
int mosdl_window_present(const void *pixels, uint32_t pitch, uint32_t w, uint32_t h);

/* Dalsi udalost: ceka nejdel timeout_ns (0 = necekat, < 0 = bez limitu).
   1 = udalost v *ev, 0 = vyprsel cas nebo probuzeni (mosdl_wake). */
int mosdl_next_event(mosdl_event *ev, int64_t timeout_ns);

/* Probudi cekajici mosdl_next_event (z libovolneho vlakna). */
int mosdl_wake(void);

/* Cela obrazovka (1) / navrat (0). Vstup jen v reakci na fyzicky stisk
   (pravidlo kompozitoru); vysledek prijde jako MOSDL_EV_RESIZE. */
int mosdl_window_set_fullscreen(int on);

/* Kurzor nad obsahem okna: systemovy tvar, nebo vlastni obrazek
   w x h pixelu B G R A (0xAARRGGBB) nejvys 64 x 64 s aktivnim bodem. */
int mosdl_set_cursor_shape(uint32_t shape);
int mosdl_set_cursor_image(const uint32_t *pixels, uint32_t w, uint32_t h, uint32_t hot_x, uint32_t hot_y);

/* Zamek ukazatele (relativni pohyb, skryty kurzor). Jen okno s fokusem
   po fyzickem stisku; vysledek a zruseni systemem = MOSDL_EV_POINTER_LOCK. */
int mosdl_pointer_lock(int on);

/* Schranka (text UTF-8). Kopirovani jen v reakci na fyzicky stisk.
   mosdl_clipboard_fetch pozada o text a pocka na nej (nejdel 1 s):
   vrati delku v bajtech (0 = prazdna), zaporne = chyba; text pak
   zkopiruje mosdl_clipboard_copy (nejvys cap - 1 bajtu a nula). */
int mosdl_clipboard_set_text(const char *text);
int mosdl_clipboard_has_text(void);
int64_t mosdl_clipboard_fetch(void);
int64_t mosdl_clipboard_copy(char *buf, size_t cap);

/* Zvuk: relace aplikace u audiod (stitek AUDIO ze startovni zpravy).
   Vzorky (MOSDL_SAMPLE_*) prokladane po kanalech, little endian. */
enum {
    MOSDL_SAMPLE_U8 = 0,
    MOSDL_SAMPLE_S16 = 1,
    MOSDL_SAMPLE_S32 = 2,
    MOSDL_SAMPLE_F32 = 3
};

/* Ma proces relaci zvuku? 1 ano, 0 ne (pak SDL zvoli ovladac dummy). */
int mosdl_audio_available(void);
/* Otevre stream (dir 0 vystup, 1 vstup); vrati id > 0, nebo zaporny kod
   (ACCESS_DENIED = nahravani nepovoleno; OPEN vstupu muze cekat na
   rozhodnuti uzivatele v systemovem dialogu). *period_frames = takt sluzby. */
int mosdl_audio_open(int dir, uint32_t rate, uint32_t channels, uint32_t sample, uint32_t latency_us, uint32_t *period_frames);
/* Zapise cele snimky; pri plnem bufferu ceka (nejdel timeout_ns, < 0 bez
   limitu). Stream se spusti, jakmile se buffer poprve naplni. */
int mosdl_audio_write(int id, const void *data, uint32_t bytes, int64_t timeout_ns);
/* Precte dostupna nahrana data; vrati pocet bajtu. */
int mosdl_audio_read(int id, void *data, uint32_t bytes);
/* Ceka na udalost streamu (misto / nova data). */
int mosdl_audio_wait(int id, int64_t timeout_ns);
/* Ceka, az se do bufferu vejde `bytes` bajtu (nejdel timeout_ns, < 0 bez
   limitu); 0 = misto je. Plny buffer nespusteneho streamu ho spusti.
   Ovladac SDL: cekat tady (WaitDevice, mimo zamek), zapis pak neblokuje. */
int mosdl_audio_wait_space(int id, uint32_t bytes, int64_t timeout_ns);
int mosdl_audio_start(int id);
void mosdl_audio_close(int id);

#ifdef __cplusplus
}
#endif

#endif
