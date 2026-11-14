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
    MOSDL_EV_EXPOSE = 10
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

#ifdef __cplusplus
}
#endif

#endif
