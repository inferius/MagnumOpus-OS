/*
 * MagnumOpus graphics driver - unix side definitions
 *
 * Copyright 2026 MagnumOpus contributors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin St, Fifth Floor, Boston, MA 02110-1301, USA
 */

/* Ovladac grafiky Wine pro kompozitor MagnumOpus (docs/cs/windows-vrstva.md,
 * kapitola W3). S kompozitorem mluvi pres knihovnu mowin (libc.so systemu,
 * <mowin.h>): jedno spojeni na proces, okna, popupy, udalosti, schranka. */

#ifndef __WINE_MODRV_H
#define __WINE_MODRV_H

#include <stdarg.h>
#include <pthread.h>
#include <mowin.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winbase.h"
#include "ntgdi.h"
#include "ntuser.h"
#include "wine/gdi_driver.h"
#include "wine/rbtree.h"

#include "unixlib.h"

/* Soukrome zpravy ovladace (vlakno okna). */
enum modrv_window_message
{
    /* wp, lp = nova velikost obsahu od kompozitoru */
    WM_MO_CONFIGURE = WM_WINE_FIRST_DRIVER_MSG,
    /* wp, lp = poloha obsahu okna na obrazovce */
    WM_MO_PLACE,
    WM_MO_SET_FOREGROUND,
};

/* Okno Wine a co ho zastupuje u kompozitoru. */
struct modrv_win
{
    struct rb_entry entry;
    HWND hwnd;
    struct window_rects rects;
    /* Okno kompozitoru (top-level) nebo 0. */
    uint32_t win;
    /* Popup (vrstva okna `owner`) nebo 0. */
    uint32_t popup;
    HWND owner;
    /* Velikost obsahu, kterou kompozitor potvrdil (CONFIGURE). */
    int conf_w, conf_h;
};

/* window.c */
struct modrv_win *modrv_win_get(HWND hwnd);
void modrv_win_release(struct modrv_win *data);
HWND modrv_hwnd_for_window(uint32_t win, RECT *visible);

BOOL MO_CreateWindowSurface(HWND hwnd, BOOL layered, const RECT *surface_rect, struct window_surface **surface);
void MO_DestroyWindow(HWND hwnd);
BOOL MO_GetWindowStyleMasks(HWND hwnd, UINT style, UINT ex_style, UINT *style_mask, UINT *ex_style_mask);
void MO_SetCursor(HWND hwnd, HCURSOR cursor);
void MO_SetWindowText(HWND hwnd, LPCWSTR text);
LRESULT MO_SysCommand(HWND hwnd, WPARAM wparam, LPARAM lparam, const POINT *pos);
LRESULT MO_WindowMessage(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp);
BOOL MO_WindowPosChanging(HWND hwnd, UINT swp_flags, BOOL shaped, const struct window_rects *rects);
void MO_WindowPosChanged(HWND hwnd, HWND insert_after, HWND owner_hint, UINT swp_flags,
                         const struct window_rects *new_rects, struct window_surface *surface);
LRESULT MO_DesktopWindowProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp);

/* event.c */
void modrv_handle_event(const mowin_event *ev);
LRESULT MO_ClipboardWindowProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp);

/* display.c */
UINT MO_UpdateDisplayDevices(const struct gdi_device_manager *device_manager, void *param);

static inline LRESULT send_message(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    return NtUserMessageCall(hwnd, msg, wparam, lparam, NULL, NtUserSendMessage, FALSE);
}

#endif /* __WINE_MODRV_H */
