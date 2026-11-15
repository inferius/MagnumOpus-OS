/*
 * MagnumOpus graphics driver - initialization
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

#if 0
#pragma makedep unix
#endif

#include "config.h"

#include <stdlib.h>

#include "modrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(modrv);

static const struct user_driver_funcs modrv_funcs =
{
    .pClipboardWindowProc = MO_ClipboardWindowProc,
    .pCreateWindowSurface = MO_CreateWindowSurface,
    .pDesktopWindowProc = MO_DesktopWindowProc,
    .pDestroyWindow = MO_DestroyWindow,
    .pGetWindowStyleMasks = MO_GetWindowStyleMasks,
    .pSetCursor = MO_SetCursor,
    .pSetWindowText = MO_SetWindowText,
    .pSysCommand = MO_SysCommand,
    .pUpdateDisplayDevices = MO_UpdateDisplayDevices,
    .pWindowMessage = MO_WindowMessage,
    .pWindowPosChanged = MO_WindowPosChanged,
    .pWindowPosChanging = MO_WindowPosChanging,
};

static NTSTATUS modrv_unix_init(void *arg)
{
    /* Proces bez spojeni s kompozitorem (bez role Gui, napr. konzolovy
     * program): ovladac se nenacte a Wine pouzije zadny displej. */
    if (!mowin_available()) return STATUS_UNSUCCESSFUL;
    __wine_set_user_driver(&modrv_funcs, WINE_GDI_DRIVER_VERSION);
    return STATUS_SUCCESS;
}

/* Vlakno udalosti kompozitoru (vola ho strana PE, dllmain.c). Vrati se jen
 * pri ztrate spojeni. */
static NTSTATUS modrv_unix_read_events(void *arg)
{
    mowin_event ev;
    int r;

    while ((r = mowin_next_event(&ev, -1)) >= 0)
    {
        if (r == 1) modrv_handle_event(&ev);
    }
    ERR("mowin_next_event failed: %d\n", r);
    return STATUS_UNSUCCESSFUL;
}

const unixlib_entry_t __wine_unix_call_funcs[] =
{
    modrv_unix_init,
    modrv_unix_read_events,
};

C_ASSERT(ARRAYSIZE(__wine_unix_call_funcs) == modrv_unix_func_count);

#ifdef _WIN64

const unixlib_entry_t __wine_unix_call_wow64_funcs[] =
{
    modrv_unix_init,
    modrv_unix_read_events,
};

C_ASSERT(ARRAYSIZE(__wine_unix_call_wow64_funcs) == modrv_unix_func_count);

#endif /* _WIN64 */
