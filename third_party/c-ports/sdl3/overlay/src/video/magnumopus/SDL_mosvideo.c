/*
 * Ovladac videa SDL pro MagnumOpus: okno kompozitoru pres libs/mosdl
 * (include/mosdl.h v sysrootu C). Popis: docs/cs/sdl.md.
 *
 * Kod MagnumOpus (ne prevzaty ze SDL); licence jako zbytek SDL (zlib),
 * aby sel port sirit spolu s knihovnou.
 *
 * Etapa 6-1: jedno okno na proces, softwarovy framebuffer (snimek se pri
 * UpdateWindowFramebuffer kopiruje do bufferu okna), klavesnice (kody evdev
 * -> scancode tabulkou Linuxu), mys, kolecko, text, fokus, zmena velikosti.
 * Kurzor, relativni mys, HiDPI a cela obrazovka: etapa 6-2.
 */
#include "SDL_internal.h"

#ifdef SDL_VIDEO_DRIVER_MAGNUMOPUS

#include <time.h>

#include <mosdl.h>

#include "../SDL_sysvideo.h"
#include "../../events/SDL_events_c.h"
#include "../../events/SDL_keyboard_c.h"
#include "../../events/SDL_mouse_c.h"
#include "../../events/SDL_windowevents_c.h"
#include "../../events/SDL_scancode_tables_c.h"

#define MOS_DRIVER_NAME "magnumopus"
#define MOS_SURFACE "SDL.internal.window.surface"

/* Kody tlacitek mysi evdev (linux/input-event-codes.h). */
#define MOS_BTN_LEFT 0x110
#define MOS_BTN_RIGHT 0x111
#define MOS_BTN_MIDDLE 0x112
#define MOS_BTN_SIDE 0x113
#define MOS_BTN_EXTRA 0x114

/* Jedine okno procesu (kanal okna ze startovni zpravy je jeden). */
static SDL_Window *mos_window;

/* Cas udalosti (ns od startu systemu, CLOCK_MONOTONIC) -> cas SDL. */
static Uint64 MOS_Timestamp(Uint64 event_ns)
{
    struct timespec ts;
    Uint64 now_ns, ticks;
    if (event_ns == 0 || clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }
    now_ns = (Uint64)ts.tv_sec * SDL_NS_PER_SECOND + (Uint64)ts.tv_nsec;
    ticks = SDL_GetTicksNS();
    if (event_ns > now_ns || now_ns - event_ns > ticks) {
        return 0;
    }
    return ticks - (now_ns - event_ns);
}

static Uint8 MOS_Button(int code)
{
    switch (code) {
    case MOS_BTN_LEFT:
        return SDL_BUTTON_LEFT;
    case MOS_BTN_RIGHT:
        return SDL_BUTTON_RIGHT;
    case MOS_BTN_MIDDLE:
        return SDL_BUTTON_MIDDLE;
    case MOS_BTN_SIDE:
        return SDL_BUTTON_X1;
    case MOS_BTN_EXTRA:
        return SDL_BUTTON_X2;
    default:
        return 0;
    }
}

/* Caps Lock a Num Lock podle systemu (prepinace, ne drzene klavesy). */
static void MOS_SyncLocks(Uint32 mods)
{
    SDL_ToggleModState(SDL_KMOD_CAPS, (mods & MOSDL_MOD_CAPS) != 0);
    SDL_ToggleModState(SDL_KMOD_NUM, (mods & MOSDL_MOD_NUM) != 0);
}

static void MOS_Dispatch(const mosdl_event *ev)
{
    SDL_Window *w = mos_window;
    Uint64 t = MOS_Timestamp(ev->time_ns);
    char text[5];
    Uint8 b;

    if (!w) {
        return;
    }
    switch (ev->type) {
    case MOSDL_EV_KEY:
        MOS_SyncLocks(ev->mods);
        SDL_SendKeyboardKey(t, SDL_DEFAULT_KEYBOARD_ID, ev->a,
                            SDL_GetScancodeFromTable(SDL_SCANCODE_TABLE_LINUX, ev->a), ev->b != 0);
        break;
    case MOSDL_EV_TEXT:
        if (SDL_UCS4ToUTF8((Uint32)ev->a, text) != text) {
            *SDL_UCS4ToUTF8((Uint32)ev->a, text) = '\0';
            SDL_SendKeyboardText(text);
        }
        break;
    case MOSDL_EV_MOTION:
        SDL_SetMouseFocus(w);
        SDL_SendMouseMotion(t, w, SDL_GLOBAL_MOUSE_ID, false, (float)ev->a, (float)ev->b);
        break;
    case MOSDL_EV_LEAVE:
        SDL_SetMouseFocus(NULL);
        break;
    case MOSDL_EV_BUTTON:
        b = MOS_Button(ev->a);
        if (b) {
            SDL_SetMouseFocus(w);
            SDL_SendMouseMotion(t, w, SDL_GLOBAL_MOUSE_ID, false, (float)ev->c, (float)ev->d);
            SDL_SendMouseButton(t, w, SDL_GLOBAL_MOUSE_ID, b, ev->b != 0);
        }
        break;
    case MOSDL_EV_WHEEL:
        SDL_SendMouseWheel(t, w, SDL_GLOBAL_MOUSE_ID, (float)ev->a, (float)ev->b, SDL_MOUSEWHEEL_NORMAL);
        break;
    case MOSDL_EV_FOCUS:
        SDL_SetKeyboardFocus(ev->a ? w : NULL);
        break;
    case MOSDL_EV_RESIZE:
        SDL_SendWindowEvent(w, SDL_EVENT_WINDOW_RESIZED, ev->a, ev->b);
        break;
    case MOSDL_EV_CLOSE:
        SDL_SendWindowEvent(w, SDL_EVENT_WINDOW_CLOSE_REQUESTED, 0, 0);
        break;
    case MOSDL_EV_EXPOSE:
        SDL_SendWindowEvent(w, SDL_EVENT_WINDOW_EXPOSED, 0, 0);
        break;
    default:
        break;
    }
}

static void MOS_PumpEvents(SDL_VideoDevice *_this)
{
    mosdl_event ev;
    /* Nejvys 256 udalosti za jedno volani: hlavni smycka aplikace nesmi
       uviznout, kdyz udalosti chodi rychleji, nez je zpracuje. */
    for (int i = 0; i < 256 && mosdl_next_event(&ev, 0) > 0; i++) {
        MOS_Dispatch(&ev);
    }
}

static int MOS_WaitEventTimeout(SDL_VideoDevice *_this, Sint64 timeoutNS)
{
    mosdl_event ev;
    int r = mosdl_next_event(&ev, timeoutNS);
    if (r > 0) {
        MOS_Dispatch(&ev);
        return 1;
    }
    return 0;
}

static void MOS_SendWakeupEvent(SDL_VideoDevice *_this, SDL_Window *window)
{
    mosdl_wake();
}

static bool MOS_CreateWindow(SDL_VideoDevice *_this, SDL_Window *window, SDL_PropertiesID create_props)
{
    Uint32 w = 0, h = 0;
    int r;

    if (mos_window) {
        return SDL_Unsupported(); /* jedno okno na proces (etapa 6-1) */
    }
    r = mosdl_window_open((Uint32)window->floating.w, (Uint32)window->floating.h, window->title, &w, &h);
    if (r < 0) {
        return SDL_SetError("magnumopus: okno nelze otevrit (chyba %d)", r);
    }
    mos_window = window;
    window->x = window->floating.x = 0;
    window->y = window->floating.y = 0;
    if ((int)w != window->w || (int)h != window->h) {
        SDL_SendWindowEvent(window, SDL_EVENT_WINDOW_RESIZED, (int)w, (int)h);
    }
    SDL_SetMouseFocus(window);
    SDL_SetKeyboardFocus(window);
    return true;
}

static void MOS_SetWindowTitle(SDL_VideoDevice *_this, SDL_Window *window)
{
    if (window == mos_window) {
        mosdl_window_set_title(window->title);
    }
}

static void MOS_SetWindowSize(SDL_VideoDevice *_this, SDL_Window *window)
{
    if (window == mos_window) {
        /* Kompozitor odpovi udalosti RESIZE (nebo velikost nezmeni). */
        mosdl_window_set_size((Uint32)window->pending.w, (Uint32)window->pending.h,
                              (Uint32)window->min_w, (Uint32)window->min_h);
    }
}

static void MOS_DestroyWindow(SDL_VideoDevice *_this, SDL_Window *window)
{
    if (window == mos_window) {
        mosdl_window_close();
        mos_window = NULL;
    }
}

static bool MOS_CreateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, SDL_PixelFormat *format, void **pixels, int *pitch)
{
    SDL_Surface *surface;
    int w, h;

    SDL_GetWindowSizeInPixels(window, &w, &h);
    surface = SDL_CreateSurface(w, h, SDL_PIXELFORMAT_XRGB8888);
    if (!surface) {
        return false;
    }
    SDL_SetSurfaceProperty(SDL_GetWindowProperties(window), MOS_SURFACE, surface);
    *format = SDL_PIXELFORMAT_XRGB8888;
    *pixels = surface->pixels;
    *pitch = surface->pitch;
    return true;
}

static bool MOS_UpdateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, const SDL_Rect *rects, int numrects)
{
    SDL_Surface *s = (SDL_Surface *)SDL_GetPointerProperty(SDL_GetWindowProperties(window), MOS_SURFACE, NULL);
    int r;

    if (!s) {
        return SDL_SetError("magnumopus: okno nema framebuffer");
    }
    if (window != mos_window) {
        return true;
    }
    /* Cely snimek: buffery okna jsou dvojite, vyrez by nechal starsi obsah. */
    r = mosdl_window_present(s->pixels, (Uint32)s->pitch, (Uint32)s->w, (Uint32)s->h);
    if (r < 0) {
        return SDL_SetError("magnumopus: snimek nelze ukazat (chyba %d)", r);
    }
    return true;
}

static void MOS_DestroyWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window)
{
    SDL_ClearProperty(SDL_GetWindowProperties(window), MOS_SURFACE);
}

static bool MOS_VideoInit(SDL_VideoDevice *_this)
{
    SDL_DisplayMode mode;
    mosdl_display d;

    SDL_zero(mode);
    mode.format = SDL_PIXELFORMAT_XRGB8888;
    /* Pred otevrenim okna obrazovku jeste nezname: vychozi rezim, po
       otevreni okna ho nahradi skutecny (etapa 6-2, HiDPI). */
    mode.w = 1280;
    mode.h = 800;
    if (mosdl_display_info(&d) == 0 && d.screen_w > 0 && d.screen_h > 0) {
        mode.w = (int)d.screen_w;
        mode.h = (int)d.screen_h;
        mode.refresh_rate = (float)d.refresh_mhz / 1000.0f;
    }
    if (SDL_AddBasicVideoDisplay(&mode) == 0) {
        return false;
    }
    SDL_AddKeyboard(SDL_DEFAULT_KEYBOARD_ID, NULL);
    SDL_AddMouse(SDL_DEFAULT_MOUSE_ID, NULL);
    return true;
}

static void MOS_VideoQuit(SDL_VideoDevice *_this)
{
    SDL_RemoveKeyboard(SDL_DEFAULT_KEYBOARD_ID);
    SDL_RemoveMouse(SDL_DEFAULT_MOUSE_ID);
}

static void MOS_DeleteDevice(SDL_VideoDevice *device)
{
    SDL_free(device);
}

static SDL_VideoDevice *MOS_CreateDevice(void)
{
    SDL_VideoDevice *device;

    if (!mosdl_available()) {
        return NULL; /* proces nema kanal okna (spusten bez GUI) */
    }
    device = (SDL_VideoDevice *)SDL_calloc(1, sizeof(SDL_VideoDevice));
    if (!device) {
        return NULL;
    }
    device->VideoInit = MOS_VideoInit;
    device->VideoQuit = MOS_VideoQuit;
    device->PumpEvents = MOS_PumpEvents;
    device->WaitEventTimeout = MOS_WaitEventTimeout;
    device->SendWakeupEvent = MOS_SendWakeupEvent;
    device->CreateSDLWindow = MOS_CreateWindow;
    device->SetWindowTitle = MOS_SetWindowTitle;
    device->SetWindowSize = MOS_SetWindowSize;
    device->DestroyWindow = MOS_DestroyWindow;
    device->CreateWindowFramebuffer = MOS_CreateWindowFramebuffer;
    device->UpdateWindowFramebuffer = MOS_UpdateWindowFramebuffer;
    device->DestroyWindowFramebuffer = MOS_DestroyWindowFramebuffer;
    device->free = MOS_DeleteDevice;
    return device;
}

VideoBootStrap MAGNUMOPUS_bootstrap = {
    MOS_DRIVER_NAME, "MagnumOpus compositor",
    MOS_CreateDevice,
    NULL, /* ShowMessageBox: etapa 6-2 */
    false
};

#endif /* SDL_VIDEO_DRIVER_MAGNUMOPUS */
