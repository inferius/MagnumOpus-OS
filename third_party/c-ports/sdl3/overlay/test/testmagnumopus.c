/*
 * Test portu SDL na MagnumOpus (sada `sdl`, user/init/src/tests/sdl.rs):
 * kurzor, skala displeje, zamek ukazatele (relativni mys), schranka a cela
 * obrazovka. Kod MagnumOpus, licence zlib jako SDL.
 *
 * Program vypisuje kroky na stdout (init je cte a podle nich posila
 * fyzicky vstup pres QMP):
 *   ready <w> <h> <scale>     okno je videt, kurzor je kriz
 *   lock <0|1>                levy klik zapne zamek; Super ho zrusi
 *   rel <dx> <dy>             relativni pohyb pri zamku (soucet)
 *   copy                      druhy levy klik zkopiruje text
 *   paste <text>              Ctrl+V vlozi text ze schranky
 *   fullscreen <0|1>          klavesa F prepne celou obrazovku
 * Esc program ukonci (kod 0). Argument "bench" = mereni cesty videa 1080p,
 * "audio" = prehravani a nahravani (povoleni).
 */
#include <SDL3/SDL.h>
#include <stdio.h>

#define TEXT "magnumopus-sdl"

static void say(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vprintf(fmt, ap);
    va_end(ap);
    printf("\n");
    fflush(stdout);
}

/* Mereni cesty videa (argument "bench"): YUV textura I420 1920x1080 ->
   SDL_UpdateYUVTexture (prevod v SDL) -> RenderTexture pres cele okno ->
   RenderPresent (kopie do bufferu okna). Vypise "bench <ms na snimek>"
   (prumer z 60 snimku po 10 zahrivacich). */
static int bench(void)
{
    const int W = 1920, H = 1080, WARM = 10, N = 60;
    SDL_Window *win;
    SDL_Renderer *ren;
    SDL_Texture *tex;
    Uint8 *y, *u, *v;
    Uint64 t0 = 0;
    int i;

    if (!SDL_Init(SDL_INIT_VIDEO) || !SDL_CreateWindowAndRenderer("bench", 960, 540, 0, &win, &ren)) {
        say("chyba: %s", SDL_GetError());
        return 1;
    }
    tex = SDL_CreateTexture(ren, SDL_PIXELFORMAT_IYUV, SDL_TEXTUREACCESS_STREAMING, W, H);
    y = (Uint8 *)SDL_malloc((size_t)W * H);
    u = (Uint8 *)SDL_malloc((size_t)W * H / 4);
    v = (Uint8 *)SDL_malloc((size_t)W * H / 4);
    if (!tex || !y || !u || !v) {
        say("chyba: %s", SDL_GetError());
        return 1;
    }
    for (i = 0; i < WARM + N; i++) {
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
        }
        if (i == WARM) {
            t0 = SDL_GetTicksNS();
        }
        SDL_memset(y, (i * 7) & 0xFF, (size_t)W * H);
        SDL_memset(u, 128, (size_t)W * H / 4);
        SDL_memset(v, (i * 3) & 0xFF, (size_t)W * H / 4);
        SDL_UpdateYUVTexture(tex, NULL, y, W, u, W / 2, v, W / 2);
        SDL_RenderTexture(ren, tex, NULL, NULL);
        SDL_RenderPresent(ren);
    }
    say("bench %.2f", (double)(SDL_GetTicksNS() - t0) / 1e6 / N);
    SDL_Quit();
    return 0;
}

/* Zvuk (argument "audio"): prehrani tonu pres SDL a pokus o nahravani.
   Vypise: "audio-driver <jmeno>", "out ok" (ton dohral), "capture-ask"
   (pred otevrenim nahravani, ktere muze cekat na dialog povoleni),
   "capture 1|0" a pri povoleni "capture-data <bajty>". */
static int audio_test(void)
{
    SDL_AudioSpec spec;
    SDL_AudioStream *out, *in;
    Sint16 buf[4800 * 2];
    Uint64 end;
    int i;

    if (!SDL_Init(SDL_INIT_AUDIO)) {
        say("chyba SDL_Init: %s", SDL_GetError());
        return 1;
    }
    say("audio-driver %s", SDL_GetCurrentAudioDriver());
    spec.format = SDL_AUDIO_S16;
    spec.channels = 2;
    spec.freq = 48000;
    out = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &spec, NULL, NULL);
    if (!out) {
        say("out chyba %s", SDL_GetError());
        return 1;
    }
    /* 0,3 s tonu 440 Hz (3 bloky po 0,1 s). */
    for (i = 0; i < 4800; i++) {
        Sint16 v = (Sint16)(SDL_sin(2.0 * SDL_PI_D * 440.0 * i / 48000.0) * 8000.0);
        buf[2 * i] = v;
        buf[2 * i + 1] = v;
    }
    for (i = 0; i < 3; i++) {
        SDL_PutAudioStreamData(out, buf, sizeof(buf));
    }
    SDL_ResumeAudioStreamDevice(out);
    end = SDL_GetTicks() + 5000;
    while (SDL_GetAudioStreamQueued(out) > 0 && SDL_GetTicks() < end) {
        SDL_Delay(20);
    }
    say(SDL_GetAudioStreamQueued(out) > 0 ? "out zaseknuto" : "out ok");
    SDL_DestroyAudioStream(out);

    say("capture-ask");
    in = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_RECORDING, &spec, NULL, NULL);
    if (!in) {
        say("capture 0");
    } else {
        int got = 0;
        say("capture 1");
        SDL_ResumeAudioStreamDevice(in);
        end = SDL_GetTicks() + 5000;
        while (got == 0 && SDL_GetTicks() < end) {
            got = SDL_GetAudioStreamAvailable(in);
            SDL_Delay(20);
        }
        say("capture-data %d", got);
        SDL_DestroyAudioStream(in);
    }
    SDL_Quit();
    return 0;
}

int main(int argc, char *argv[])
{
    SDL_Window *win;
    SDL_Renderer *ren;
    SDL_Cursor *cross;
    const SDL_DisplayMode *mode;
    int clicks = 0, done = 0, relx = 0, rely = 0, reported_rel = 0;
    bool locked = false;

    if (argc > 1 && SDL_strcmp(argv[1], "bench") == 0) {
        return bench();
    }
    if (argc > 1 && SDL_strcmp(argv[1], "audio") == 0) {
        return audio_test();
    }
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        say("chyba SDL_Init: %s", SDL_GetError());
        return 1;
    }
    if (!SDL_CreateWindowAndRenderer("testmagnumopus", 400, 300, 0, &win, &ren)) {
        say("chyba okna: %s", SDL_GetError());
        return 1;
    }
    cross = SDL_CreateSystemCursor(SDL_SYSTEM_CURSOR_CROSSHAIR);
    SDL_SetCursor(cross);
    SDL_SetRenderDrawColor(ren, 0x20, 0x60, 0xA0, 0xFF);
    SDL_RenderClear(ren);
    SDL_RenderPresent(ren);
    mode = SDL_GetDesktopDisplayMode(SDL_GetDisplayForWindow(win));
    say("ready %d %d %.2f", mode ? mode->w : 0, mode ? mode->h : 0, SDL_GetDisplayContentScale(SDL_GetDisplayForWindow(win)));

    while (!done) {
        SDL_Event e;
        /* Stav zamku podle SDL (zapnuti i zruseni systemem, ktere zadnou
           udalost SDL nevyvola): kontrola v kazdem kole. */
        if (SDL_GetWindowRelativeMouseMode(win) != locked) {
            locked = SDL_GetWindowRelativeMouseMode(win);
            say("lock %d", locked ? 1 : 0);
        }
        if (!SDL_WaitEventTimeout(&e, 100)) {
            continue;
        }
        switch (e.type) {
        case SDL_EVENT_QUIT:
            done = 1;
            break;
        case SDL_EVENT_MOUSE_BUTTON_DOWN:
            if (e.button.button != SDL_BUTTON_LEFT) {
                break;
            }
            clicks++;
            if (clicks == 1) {
                SDL_SetWindowRelativeMouseMode(win, true);
            } else if (clicks == 2) {
                if (SDL_SetClipboardText(TEXT)) {
                    say("copy");
                } else {
                    say("chyba kopie: %s", SDL_GetError());
                }
            }
            break;
        case SDL_EVENT_MOUSE_MOTION:
            if (locked && e.motion.which != SDL_TOUCH_MOUSEID) {
                relx += (int)e.motion.xrel;
                rely += (int)e.motion.yrel;
                if (!reported_rel && (relx != 0 || rely != 0)) {
                    reported_rel = 1;
                    say("rel %d %d", relx, rely);
                }
            }
            break;
        case SDL_EVENT_KEY_DOWN:
            if (e.key.key == SDLK_ESCAPE) {
                done = 1;
            } else if (e.key.key == SDLK_V && (e.key.mod & SDL_KMOD_CTRL)) {
                char *t = SDL_GetClipboardText();
                say("paste %s", t ? t : "");
                SDL_free(t);
            } else if (e.key.key == SDLK_F) {
                SDL_SetWindowFullscreen(win, !(SDL_GetWindowFlags(win) & SDL_WINDOW_FULLSCREEN));
            }
            break;
        case SDL_EVENT_WINDOW_ENTER_FULLSCREEN:
            say("fullscreen 1");
            break;
        case SDL_EVENT_WINDOW_LEAVE_FULLSCREEN:
            say("fullscreen 0");
            break;
        default:
            break;
        }
        SDL_RenderClear(ren);
        SDL_RenderPresent(ren);
    }
    SDL_DestroyCursor(cross);
    SDL_DestroyRenderer(ren);
    SDL_DestroyWindow(win);
    SDL_Quit();
    return 0;
}
