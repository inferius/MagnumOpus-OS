/*
 * Ovladac videa SDL pro MagnumOpus (etapa 6-2): schranka, kurzory,
 * relativni mys (zamek ukazatele kompozitoru), cela obrazovka a skala
 * displeje. Kod MagnumOpus, licence zlib jako SDL. Popis: docs/cs/sdl.md.
 */
#include "SDL_internal.h"

#ifdef SDL_VIDEO_DRIVER_MAGNUMOPUS

#include <mosdl.h>

#include "SDL_mosvideo.h"
#include "../../events/SDL_mouse_c.h"

SDL_DisplayID MOS_display;

/* --- displej ------------------------------------------------------------ */

void MOS_UpdateDisplay(void)
{
    mosdl_display d;
    SDL_VideoDisplay *display = SDL_GetVideoDisplay(MOS_display);
    SDL_DisplayMode mode;

    if (!display || mosdl_display_info(&d) != 0 || d.screen_w == 0 || d.screen_h == 0) {
        return;
    }
    SDL_zero(mode);
    mode.format = SDL_PIXELFORMAT_XRGB8888;
    mode.w = (int)d.screen_w;
    mode.h = (int)d.screen_h;
    mode.refresh_rate = (float)d.refresh_mhz / 1000.0f;
    SDL_SetDesktopDisplayMode(display, &mode);
    SDL_SetCurrentDisplayMode(display, &mode);
    /* Buffery okna jsou 1:1 (kompozitor zatim neskaluje): skala jen jako
       doporuceni pro velikost obsahu (SDL_GetDisplayContentScale). */
    SDL_SetDisplayContentScale(display, d.scale_pct ? (float)d.scale_pct / 100.0f : 1.0f);
}

/* --- schranka ----------------------------------------------------------- */

bool MOS_SetClipboardText(SDL_VideoDevice *_this, const char *text)
{
    int r = mosdl_clipboard_set_text(text);
    if (r < 0) {
        return SDL_SetError("magnumopus: schranka (chyba %d)", r);
    }
    return true;
}

char *MOS_GetClipboardText(SDL_VideoDevice *_this)
{
    int64_t n = mosdl_clipboard_fetch();
    char *s;

    if (n <= 0) {
        return SDL_strdup("");
    }
    s = (char *)SDL_malloc((size_t)n + 1);
    if (!s) {
        return NULL;
    }
    mosdl_clipboard_copy(s, (size_t)n + 1);
    return s;
}

bool MOS_HasClipboardText(SDL_VideoDevice *_this)
{
    return mosdl_clipboard_has_text() != 0;
}

/* --- cela obrazovka ----------------------------------------------------- */

SDL_FullscreenResult MOS_SetWindowFullscreen(SDL_VideoDevice *_this, SDL_Window *window, SDL_VideoDisplay *display, SDL_FullscreenOp op)
{
    /* Kompozitor povoli vstup jen po fyzickem stisku; odpoved (CONFIGURE)
       prijde jako MOSDL_EV_STATE a RESIZE. */
    if (mosdl_window_set_fullscreen(op != SDL_FULLSCREEN_OP_LEAVE) < 0) {
        return SDL_FULLSCREEN_FAILED;
    }
    return SDL_FULLSCREEN_PENDING;
}

/* --- kurzory a relativni mys -------------------------------------------- */

/* Data kurzoru: systemovy tvar, nebo obrazek B G R A. */
struct SDL_CursorData
{
    int shape; /* MOSDL_CURSOR_*, -1 = obrazek */
    Uint32 *pixels;
    int w, h, hot_x, hot_y;
};

static SDL_Cursor *MOS_NewCursor(int shape)
{
    SDL_Cursor *c = (SDL_Cursor *)SDL_calloc(1, sizeof(*c));
    SDL_CursorData *d = (SDL_CursorData *)SDL_calloc(1, sizeof(*d));
    if (!c || !d) {
        SDL_free(c);
        SDL_free(d);
        return NULL;
    }
    d->shape = shape;
    c->internal = d;
    return c;
}

static SDL_Cursor *MOS_CreateSystemCursor(SDL_SystemCursor id)
{
    /* Prvnich 12 tvaru SDL ma stejna cisla jako MOSDL_CURSOR_*; okraje
       okna (NW_RESIZE ...) na dvojite sipky. */
    static const int edges[] = {
        MOSDL_CURSOR_RESIZE_NWSE, /* NW */
        MOSDL_CURSOR_RESIZE_NS,   /* N */
        MOSDL_CURSOR_RESIZE_NESW, /* NE */
        MOSDL_CURSOR_RESIZE_EW,   /* E */
        MOSDL_CURSOR_RESIZE_NWSE, /* SE */
        MOSDL_CURSOR_RESIZE_NS,   /* S */
        MOSDL_CURSOR_RESIZE_NESW, /* SW */
        MOSDL_CURSOR_RESIZE_EW,   /* W */
    };
    int shape;
    if ((int)id <= MOSDL_CURSOR_POINTER) {
        shape = (int)id;
    } else if ((int)id - SDL_SYSTEM_CURSOR_NW_RESIZE < (int)SDL_arraysize(edges)) {
        shape = edges[(int)id - SDL_SYSTEM_CURSOR_NW_RESIZE];
    } else {
        shape = MOSDL_CURSOR_DEFAULT;
    }
    return MOS_NewCursor(shape);
}

static SDL_Cursor *MOS_CreateCursor(SDL_Surface *surface, int hot_x, int hot_y)
{
    SDL_Surface *s;
    SDL_Cursor *c;
    int y;

    if (surface->w > 64 || surface->h > 64) {
        SDL_SetError("magnumopus: kurzor nejvys 64x64");
        return NULL;
    }
    s = SDL_ConvertSurface(surface, SDL_PIXELFORMAT_ARGB8888);
    if (!s) {
        return NULL;
    }
    c = MOS_NewCursor(-1);
    if (c) {
        SDL_CursorData *d = c->internal;
        d->w = s->w;
        d->h = s->h;
        d->hot_x = SDL_clamp(hot_x, 0, s->w - 1);
        d->hot_y = SDL_clamp(hot_y, 0, s->h - 1);
        d->pixels = (Uint32 *)SDL_malloc((size_t)s->w * (size_t)s->h * 4);
        if (!d->pixels) {
            SDL_free(d);
            SDL_free(c);
            c = NULL;
        } else {
            for (y = 0; y < s->h; y++) {
                SDL_memcpy(d->pixels + (size_t)y * (size_t)s->w, (Uint8 *)s->pixels + (size_t)y * (size_t)s->pitch, (size_t)s->w * 4);
            }
        }
    }
    SDL_DestroySurface(s);
    return c;
}

static bool MOS_ShowCursor(SDL_Cursor *cursor)
{
    SDL_CursorData *d = cursor ? cursor->internal : NULL;
    int r;
    if (!d) {
        r = mosdl_set_cursor_shape(MOSDL_CURSOR_HIDDEN);
    } else if (d->shape >= 0) {
        r = mosdl_set_cursor_shape((uint32_t)d->shape);
    } else {
        r = mosdl_set_cursor_image(d->pixels, (uint32_t)d->w, (uint32_t)d->h, (uint32_t)d->hot_x, (uint32_t)d->hot_y);
    }
    /* Bez okna (SDL nastavuje vychozi kurzor uz pri startu) neni chyba:
       kurzor se posle znovu, az okno dostane fokus mysi. */
    (void)r;
    return true;
}

static void MOS_FreeCursor(SDL_Cursor *cursor)
{
    if (cursor) {
        if (cursor->internal) {
            SDL_free(cursor->internal->pixels);
            SDL_free(cursor->internal);
        }
        SDL_free(cursor);
    }
}

static bool MOS_SetRelativeMouseMode(bool enabled)
{
    /* Kompozitor zamek povoli jen oknu s fokusem po fyzickem stisku;
       vysledek prijde jako MOSDL_EV_POINTER_LOCK. */
    if (mosdl_pointer_lock(enabled ? 1 : 0) < 0) {
        return SDL_SetError("magnumopus: zamek ukazatele nelze");
    }
    return true;
}

void MOS_InitMouse(void)
{
    SDL_Mouse *mouse = SDL_GetMouse();
    mouse->CreateCursor = MOS_CreateCursor;
    mouse->CreateSystemCursor = MOS_CreateSystemCursor;
    mouse->ShowCursor = MOS_ShowCursor;
    mouse->FreeCursor = MOS_FreeCursor;
    mouse->SetRelativeMouseMode = MOS_SetRelativeMouseMode;
    SDL_SetDefaultCursor(MOS_CreateSystemCursor(SDL_SYSTEM_CURSOR_DEFAULT));
}

#endif /* SDL_VIDEO_DRIVER_MAGNUMOPUS */
