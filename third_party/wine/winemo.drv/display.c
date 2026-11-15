/*
 * MagnumOpus graphics driver - display devices
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

#include "modrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(modrv);

/* Jeden vystup: obrazovka a pracovni plocha (bez listy) od kompozitoru.
 * Skala je zatim vzdy 100 % = 96 DPI (rozhodnuti 4A); kompozitor polohy
 * oken neurcuje od klienta, monitor zacina v 0,0. */
UINT MO_UpdateDisplayDevices(const struct gdi_device_manager *device_manager, void *param)
{
    DWORD state_flags = DISPLAY_DEVICE_ATTACHED_TO_DESKTOP | DISPLAY_DEVICE_PRIMARY_DEVICE;
    struct gdi_monitor monitor = {0};
    struct pci_id pci_id = {0};
    DEVMODEW mode = {.dmSize = sizeof(mode)};
    mowin_display info;
    UINT dpi = NtUserGetSystemDpiForProcess(NULL);

    if (mowin_display_info(&info) < 0 || !info.screen_w || !info.screen_h)
    {
        ERR("no display info from the compositor\n");
        return STATUS_UNSUCCESSFUL;
    }
    TRACE("screen %ux%u work %ux%u scale %u\n", info.screen_w, info.screen_h, info.work_w, info.work_h, info.scale_pct);

    device_manager->add_gpu(NULL, &pci_id, NULL, param);
    device_manager->add_source("MagnumOpus", state_flags, dpi, param);

    SetRect(&monitor.rc_monitor, 0, 0, info.screen_w, info.screen_h);
    SetRect(&monitor.rc_work, 0, 0, info.work_w ? info.work_w : info.screen_w,
            info.work_h ? info.work_h : info.screen_h);
    device_manager->add_monitor(&monitor, param);

    mode.dmFields = DM_DISPLAYORIENTATION | DM_BITSPERPEL | DM_PELSWIDTH | DM_PELSHEIGHT |
                    DM_DISPLAYFLAGS | DM_DISPLAYFREQUENCY | DM_POSITION;
    mode.dmDisplayOrientation = DMDO_DEFAULT;
    mode.dmBitsPerPel = 32;
    mode.dmPelsWidth = info.screen_w;
    mode.dmPelsHeight = info.screen_h;
    mode.dmDisplayFrequency = info.refresh_mhz ? (info.refresh_mhz + 500) / 1000 : 60;
    device_manager->add_modes(&mode, 1, &mode, param);
    return STATUS_SUCCESS;
}
