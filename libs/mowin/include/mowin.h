/*
 * mowin.h - rozhrani C pro vic oken kompozitoru v jednom procesu
 * (libs/mowin, soucast libc.a). Pouziva ho ovladac grafiky Wine
 * (winemo.drv); popis: docs/cs/windows-vrstva.md, kapitola W3.
 *
 * Jedno spojeni s kompozitorem na proces: relace okna zdedena roli Gui
 * (abi::params::tag::GUI, osrt::gui), nebo kanal okna ze startovni
 * zpravy aplikace. Vsechna okna procesu jdou pres nej (vlastnik okna
 * musi byt okno tehoz spojeni).
 *
 * Funkce smi volat kterekoli vlakno (Wine kresli z vlaken aplikace);
 * mowin_next_event vola jedno vlakno (vlakno udalosti ovladace) a ceka bez
 * drzeni zamku.
 *
 * Navratove hodnoty int: 0 = uspech, zaporne = kod chyby jadra
 * (abi::Error, docs/cs/chyby.md). Rozmery jsou logicke px.
 */
#ifndef MOWIN_H
#define MOWIN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Typ udalosti (mowin_event.type); 1-14 stejne jako MOSDL_EV_*. */
enum {
    MOWIN_EV_NONE = 0,
    /* a = kod klavesy evdev, b = akce (0 uvolneni, 1 stisk, 2 opakovani),
       c = znak podle rozlozeni (Unicode, 0 = zadny) */
    MOWIN_EV_KEY = 1,
    /* a = vlozeny znak (Unicode) */
    MOWIN_EV_TEXT = 2,
    /* a, b = poloha ukazatele v obsahu okna (i v popupu mimo okno) */
    MOWIN_EV_MOTION = 3,
    MOWIN_EV_LEAVE = 4,
    /* a = tlacitko evdev (BTN_LEFT 0x110 ...), b = stisknuto, c, d = poloha */
    MOWIN_EV_BUTTON = 5,
    /* a = vodorovne kroky, b = svisle kroky (kladne nahoru) */
    MOWIN_EV_WHEEL = 6,
    /* a = 1 fokus ziskan, 0 ztracen */
    MOWIN_EV_FOCUS = 7,
    /* a, b = nova velikost obsahu, c = stav (MOWIN_STATE_*); dalsi
       mowin_window_present v teto velikosti ji potvrdi */
    MOWIN_EV_RESIZE = 8,
    /* uzivatel zavira okno */
    MOWIN_EV_CLOSE = 9,
    /* okno je treba prekreslit */
    MOWIN_EV_EXPOSE = 10,
    MOWIN_EV_RELMOTION = 11,
    MOWIN_EV_POINTER_LOCK = 12,
    /* zmenil se vystup okna (obrazovka, skala): mowin_display_info */
    MOWIN_EV_DISPLAY = 13,
    MOWIN_EV_STATE = 14,
    /* stisk mimo popupy okna: zavrit nabidky a rozbalene seznamy */
    MOWIN_EV_POPUP_DISMISS = 15,
    /* a, b, c, d = poloha a velikost obsahu okna na obrazovce */
    MOWIN_EV_PLACE = 16,
    /* a = 1 ve schrance je text jine aplikace (mowin_clipboard_fetch),
       0 = schranka bez textu */
    MOWIN_EV_CLIPBOARD = 17
};

/* Rezim okna (kdo kresli ram a titulek). */
enum {
    /* ram, titulek a tlacitka kresli system */
    MOWIN_MODE_STANDARD = 0,
    /* bez ramu: vse kresli aplikace (odznak duvery pri najeti) */
    MOWIN_MODE_FRAMELESS = 2
};

/* Bity stavu okna (MOWIN_EV_RESIZE c, guiproto::window::wstate). */
enum {
    MOWIN_STATE_FOCUSED = 1 << 0,
    MOWIN_STATE_MAXIMIZED = 1 << 1,
    MOWIN_STATE_FULLSCREEN = 1 << 2
};

typedef struct mowin_event {
    uint32_t type;
    uint32_t mods;      /* inputproto::mods (MOSDL_MOD_*) */
    uint64_t time_ns;
    int32_t a, b, c, d;
    uint32_t synthetic; /* 1 = umely nebo vzdaleny vstup */
    uint32_t window;    /* okno udalosti (id z mowin_window_create) */
} mowin_event;

typedef struct mowin_display {
    uint32_t screen_w, screen_h;   /* cela obrazovka */
    uint32_t work_w, work_h;       /* pracovni plocha bez listy */
    uint32_t scale_pct;            /* skala v procentech (100 = 96 DPI) */
    uint32_t refresh_mhz;          /* 0 = neznama */
} mowin_display;

/* Ma proces spojeni s kompozitorem? 1 ano, 0 ne. */
int mowin_available(void);

/* Pozdrav kompozitoru (jinak pri prvnim oknu); udaje o obrazovce. */
int mowin_display_info(mowin_display *out);

/* Nove okno (zatim skryte, ukaze ho prvni mowin_window_present).
   owner = jine okno tohoto procesu (0 = zadne): okno se vystredi nad
   nim a zustava nad nim. Skutecnou velikost vrati udalost RESIZE. */
int mowin_window_create(uint32_t mode, uint32_t w, uint32_t h, const char *title, uint32_t owner, uint32_t *out_id);
int mowin_window_destroy(uint32_t win);
int mowin_window_set_title(uint32_t win, const char *title);
/* Zadost o velikost obsahu (kompozitor odpovi udalosti RESIZE). */
int mowin_window_set_size(uint32_t win, uint32_t w, uint32_t h, uint32_t min_w, uint32_t min_h);
/* Posledni velikost od kompozitoru. */
int mowin_window_size(uint32_t win, uint32_t *w, uint32_t *h);

/* Ukaze obsah okna: pixely B G R A po radcich, krok `pitch` bajtu.
   Jina velikost nez okno se orizne nebo doplni cernou. */
int mowin_window_present(uint32_t win, const void *pixels, uint32_t pitch, uint32_t w, uint32_t h);

/* Popup (nabidka, bublina, rozbaleny seznam) okna `win`: vrstva nad vsemi
   okny na pozici x, y v souradnicich obsahu okna (klidne mimo nej).
   Kompozitor ho orizne na pracovni plochu; vstup v nem prijde oknu `win`
   v jeho souradnicich. */
int mowin_popup_create(uint32_t win, int32_t x, int32_t y, uint32_t w, uint32_t h, uint32_t *out_id);
int mowin_popup_move(uint32_t popup, int32_t x, int32_t y, uint32_t w, uint32_t h);
int mowin_popup_present(uint32_t popup, const void *pixels, uint32_t pitch, uint32_t w, uint32_t h);
int mowin_popup_destroy(uint32_t popup);

/* Dalsi udalost: ceka nejdel timeout_ns (0 = necekat, < 0 = bez limitu).
   1 = udalost v *ev, 0 = vyprsel cas nebo probuzeni (mowin_wake). */
int mowin_next_event(mowin_event *ev, int64_t timeout_ns);
int mowin_wake(void);

/* Kurzor nad obsahem okna (tvary jako MOSDL_CURSOR_*). */
int mowin_set_cursor_shape(uint32_t win, uint32_t shape);
int mowin_set_cursor_image(uint32_t win, const uint32_t *pixels, uint32_t w, uint32_t h, uint32_t hot_x, uint32_t hot_y);

/* Schranka (text UTF-8). Kopirovani i vlozeni plati jen v reakci na
   fyzicky stisk v okne s fokusem (pouzije se posledni stisk doruceny
   oknum procesu); jinak kompozitor odmitne a zapise to do logu. */
int mowin_clipboard_set_text(const char *text);
int mowin_clipboard_has_text(void);
/* Pozada o text a pocka na nej (nejdel 1 s); delka v bajtech, zaporne =
   chyba. Text drzi pro mowin_clipboard_copy. */
int64_t mowin_clipboard_fetch(void);
int64_t mowin_clipboard_copy(char *buf, size_t cap);

#ifdef __cplusplus
}
#endif

#endif
