/*
 * Ovladac videa SDL pro MagnumOpus: spolecne deklarace (SDL_mosvideo.c,
 * SDL_mosextra.c). Kod MagnumOpus, licence zlib jako SDL.
 */
#ifndef SDL_mosvideo_h_
#define SDL_mosvideo_h_

#include "../SDL_sysvideo.h"

/* Displej pridany ve VideoInit. */
extern SDL_DisplayID MOS_display;

/* Vystup okna (obrazovka, skala, frekvence) do displeje SDL. */
void MOS_UpdateDisplay(void);

/* Schranka (text). */
bool MOS_SetClipboardText(SDL_VideoDevice *_this, const char *text);
char *MOS_GetClipboardText(SDL_VideoDevice *_this);
bool MOS_HasClipboardText(SDL_VideoDevice *_this);

/* Cela obrazovka (vysledek prijde jako MOSDL_EV_STATE). */
SDL_FullscreenResult MOS_SetWindowFullscreen(SDL_VideoDevice *_this, SDL_Window *window, SDL_VideoDisplay *display, SDL_FullscreenOp op);

/* Kurzory a relativni mys (SDL_Mouse). */
void MOS_InitMouse(void);

#endif
