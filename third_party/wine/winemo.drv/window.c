/*
 * MagnumOpus graphics driver - windows and window surfaces
 *
 * Copyright 2026 MagnumOpus contributors
 * Based on the Wayland driver (winewayland.drv), Copyright 2020 Alexandros
 * Frantzis for Collabora Ltd.
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

/* Okna Wine u kompozitoru (rozhodnuti W3):
 * - top-level okno (spravovane) = okno kompozitoru; s titulkem a beznym
 *   neklientskym kreslenim ram a titulek kresli system (6C,
 *   MO_GetWindowStyleMasks -> win32u je z plochy okna vynecha), okno,
 *   ktere si neklientskou oblast zrusilo (WM_NCCALCSIZE: klient = cele
 *   okno, viditelny obdelnik = okno), je bez ramu;
 * - vlastneny popup (nabidka, bublina, rozbaleny seznam) = popup mowin
 *   (vrstva POPOVER okna vlastnika) v jeho souradnicich (5A);
 * - polohu top-level oken urcuje kompozitor; Wine dostane skutecnou polohu
 *   obsahu (MOWIN_EV_PLACE) a velikost (MOWIN_EV_RESIZE). */

#if 0
#pragma makedep unix
#endif

#include "config.h"

#include <stdlib.h>

#include "modrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(modrv);

static pthread_mutex_t win_data_mutex = PTHREAD_MUTEX_INITIALIZER;

static int win_data_cmp(const void *key, const struct rb_entry *entry)
{
    HWND key_hwnd = (HWND)key;
    HWND entry_hwnd = RB_ENTRY_VALUE(entry, struct modrv_win, entry)->hwnd;
    if (key_hwnd < entry_hwnd) return -1;
    if (key_hwnd > entry_hwnd) return 1;
    return 0;
}

static struct rb_tree win_data_rb = { win_data_cmp };

static struct modrv_win *win_data_get_nolock(HWND hwnd)
{
    struct rb_entry *e = rb_get(&win_data_rb, hwnd);
    return e ? RB_ENTRY_VALUE(e, struct modrv_win, entry) : NULL;
}

/* Data okna se zamkem (uvolnit modrv_win_release), nebo NULL. */
struct modrv_win *modrv_win_get(HWND hwnd)
{
    struct modrv_win *data;
    pthread_mutex_lock(&win_data_mutex);
    if ((data = win_data_get_nolock(hwnd))) return data;
    pthread_mutex_unlock(&win_data_mutex);
    return NULL;
}

void modrv_win_release(struct modrv_win *data)
{
    if (data) pthread_mutex_unlock(&win_data_mutex);
}

/* Okno Wine pro okno kompozitoru `win` a jeho viditelny obdelnik. */
HWND modrv_hwnd_for_window(uint32_t win, RECT *visible)
{
    struct modrv_win *data;
    HWND hwnd = 0;

    if (!win) return 0;
    pthread_mutex_lock(&win_data_mutex);
    RB_FOR_EACH_ENTRY(data, &win_data_rb, struct modrv_win, entry)
    {
        if (data->win != win) continue;
        hwnd = data->hwnd;
        if (visible) *visible = data->rects.visible;
        break;
    }
    pthread_mutex_unlock(&win_data_mutex);
    return hwnd;
}

/* Zrusi okno nebo popup u kompozitoru (data zustanou). */
static void release_host(struct modrv_win *data)
{
    if (data->win) mowin_window_destroy(data->win);
    if (data->popup) mowin_popup_destroy(data->popup);
    data->win = data->popup = 0;
    data->owner = 0;
}

/***********************************************************************
 * Spravovana okna (jako winewayland: is_window_managed).
 */

static BOOL is_managed(HWND hwnd)
{
    struct modrv_win *data = modrv_win_get(hwnd);
    BOOL ret = data && data->win;
    modrv_win_release(data);
    return ret;
}

static HWND *build_hwnd_list(void)
{
    NTSTATUS status;
    HWND *list;
    ULONG count = 128;

    for (;;)
    {
        if (!(list = malloc(count * sizeof(*list)))) return NULL;
        status = NtUserBuildHwndList(0, 0, 0, 0, 0, count, list, &count);
        if (!status) return list;
        free(list);
        if (status != STATUS_BUFFER_TOO_SMALL) return NULL;
    }
}

static BOOL has_owned_popups(HWND hwnd)
{
    HWND *list;
    UINT i;
    BOOL ret = FALSE;

    if (!(list = build_hwnd_list())) return FALSE;
    for (i = 0; list[i] != HWND_BOTTOM; i++)
    {
        if (list[i] == hwnd) break;
        if (NtUserGetWindowRelative(list[i], GW_OWNER) != hwnd) continue;
        if ((ret = is_managed(list[i]))) break;
    }
    free(list);
    return ret;
}

static inline HWND get_active_window(void)
{
    GUITHREADINFO info;
    info.cbSize = sizeof(info);
    return NtUserGetGUIThreadInfo(GetCurrentThreadId(), &info) ? info.hwndActive : 0;
}

static BOOL is_window_managed(HWND hwnd, UINT swp_flags, BOOL fullscreen)
{
    DWORD style, ex_style;

    style = NtUserGetWindowLongW(hwnd, GWL_STYLE);
    if ((style & (WS_CHILD|WS_POPUP)) == WS_CHILD) return FALSE;
    if (!(swp_flags & (SWP_NOACTIVATE|SWP_HIDEWINDOW))) return TRUE;
    if (hwnd == get_active_window()) return TRUE;
    if ((style & WS_CAPTION) == WS_CAPTION) return TRUE;
    if (style & WS_THICKFRAME) return TRUE;
    if (style & WS_POPUP)
    {
        if (style & WS_SYSMENU) return TRUE;
        if (fullscreen) return TRUE;
    }
    ex_style = NtUserGetWindowLongW(hwnd, GWL_EXSTYLE);
    if (ex_style & WS_EX_APPWINDOW) return TRUE;
    if (has_owned_popups(hwnd)) return TRUE;
    return FALSE;
}

/* Titulek okna v UTF-8. */
static void window_title(HWND hwnd, char *buf, size_t size)
{
    WCHAR text[256];
    DWORD len = 0;
    INT n = NtUserInternalGetWindowText(hwnd, text, ARRAY_SIZE(text));

    buf[0] = 0;
    if (n <= 0) return;
    if (!RtlUnicodeToUTF8N(buf, size - 1, &len, text, n * sizeof(WCHAR))) buf[len] = 0;
}

/***********************************************************************
 *           MO_WindowPosChanging
 */
BOOL MO_WindowPosChanging(HWND hwnd, UINT swp_flags, BOOL shaped, const struct window_rects *rects)
{
    struct modrv_win *data;

    TRACE("hwnd %p flags %04x rects %s\n", hwnd, swp_flags, debugstr_window_rects(rects));

    pthread_mutex_lock(&win_data_mutex);
    if (!(data = win_data_get_nolock(hwnd)))
    {
        if (!(data = calloc(1, sizeof(*data))))
        {
            pthread_mutex_unlock(&win_data_mutex);
            return FALSE;
        }
        data->hwnd = hwnd;
        data->rects = *rects;
        rb_put(&win_data_rb, hwnd, &data->entry);
    }
    pthread_mutex_unlock(&win_data_mutex);
    return TRUE;
}

/* Top-level okno u kompozitoru: vytvorit, nebo pozadat o zmenu velikosti,
 * kterou udelala aplikace (SetWindowPos). Polohu urcuje kompozitor. */
static void update_toplevel(struct modrv_win *data, HWND hwnd, int w, int h)
{
    if (data->popup)
    {
        mowin_popup_destroy(data->popup);
        data->popup = 0;
        data->owner = 0;
    }
    if (!data->win)
    {
        /* Neklientska oblast zrusena (WM_NCCALCSIZE) nebo okno bez titulku:
         * cele okno kresli Wine. */
        uint32_t mode = EqualRect(&data->rects.visible, &data->rects.window) ? MOWIN_MODE_FRAMELESS : MOWIN_MODE_STANDARD;
        HWND owner = NtUserGetWindowRelative(hwnd, GW_OWNER);
        struct modrv_win *od = owner ? win_data_get_nolock(NtUserGetAncestor(owner, GA_ROOT)) : NULL;
        char title[768];
        int r;

        window_title(hwnd, title, sizeof(title));
        r = mowin_window_create(mode, w, h, title, od ? od->win : 0, &data->win);
        if (r < 0)
        {
            ERR("hwnd %p: mowin_window_create failed %d\n", hwnd, r);
            data->win = 0;
            return;
        }
        data->conf_w = w;
        data->conf_h = h;
        TRACE("hwnd %p -> window %u mode %u %dx%d\n", hwnd, data->win, mode, w, h);
    }
    else if (w != data->conf_w || h != data->conf_h)
    {
        data->conf_w = w;
        data->conf_h = h;
        mowin_window_set_size(data->win, w, h, 0, 0);
    }
}

/* Popup (vrstva okna vlastnika) v souradnicich obsahu vlastnika. */
static void update_popup(struct modrv_win *data, struct modrv_win *od, int w, int h)
{
    int x = data->rects.visible.left - od->rects.visible.left;
    int y = data->rects.visible.top - od->rects.visible.top;

    if (data->win)
    {
        mowin_window_destroy(data->win);
        data->win = 0;
    }
    if (data->popup && data->owner != od->hwnd)
    {
        mowin_popup_destroy(data->popup);
        data->popup = 0;
    }
    if (!data->popup)
    {
        if (mowin_popup_create(od->win, x, y, w, h, &data->popup) < 0) data->popup = 0;
        data->owner = od->hwnd;
    }
    else mowin_popup_move(data->popup, x, y, w, h);
}

/***********************************************************************
 *           MO_WindowPosChanged
 */
void MO_WindowPosChanged(HWND hwnd, HWND insert_after, HWND owner_hint, UINT swp_flags,
                         const struct window_rects *new_rects, struct window_surface *surface)
{
    BOOL managed = is_window_managed(hwnd, swp_flags, swp_flags & WINE_SWP_FULLSCREEN);
    DWORD style = NtUserGetWindowLongW(hwnd, GWL_STYLE);
    HWND owner = managed ? 0 : (owner_hint ? owner_hint : NtUserGetWindowRelative(hwnd, GW_OWNER));
    struct modrv_win *data, *od;
    int w, h;

    TRACE("hwnd %p rects %s after %p flags %08x managed %d\n", hwnd, debugstr_window_rects(new_rects),
          insert_after, swp_flags, managed);

    if (owner) owner = NtUserGetAncestor(owner, GA_ROOT);
    if (!(data = modrv_win_get(hwnd))) return;
    data->rects = *new_rects;
    w = new_rects->visible.right - new_rects->visible.left;
    h = new_rects->visible.bottom - new_rects->visible.top;

    if (!surface || !(style & WS_VISIBLE) || w <= 0 || h <= 0)
        release_host(data);
    else if (owner && owner != hwnd && (od = win_data_get_nolock(owner)) && od->win)
        update_popup(data, od, w, h);
    else
        update_toplevel(data, hwnd, w, h);

    modrv_win_release(data);
}

/***********************************************************************
 *           MO_DestroyWindow
 */
void MO_DestroyWindow(HWND hwnd)
{
    struct modrv_win *data;

    TRACE("%p\n", hwnd);
    if (!(data = modrv_win_get(hwnd))) return;
    release_host(data);
    rb_remove(&win_data_rb, &data->entry);
    free(data);
    pthread_mutex_unlock(&win_data_mutex);
}

/***********************************************************************
 *           MO_GetWindowStyleMasks
 *
 * Ram a titulek top-level okna s titulkem kresli kompozitor (6C).
 */
BOOL MO_GetWindowStyleMasks(HWND hwnd, UINT style, UINT ex_style, UINT *style_mask, UINT *ex_style_mask)
{
    *style_mask = *ex_style_mask = 0;
    if (NtUserGetAncestor(hwnd, GA_PARENT) != NtUserGetDesktopWindow()) return TRUE;
    if ((style & WS_CAPTION) != WS_CAPTION || (ex_style & WS_EX_LAYERED)) return TRUE;
    *style_mask = WS_CAPTION | WS_DLGFRAME | WS_THICKFRAME;
    *ex_style_mask = WS_EX_DLGMODALFRAME;
    return TRUE;
}

/***********************************************************************
 *           MO_SetWindowText
 */
void MO_SetWindowText(HWND hwnd, LPCWSTR text)
{
    struct modrv_win *data;
    char title[768];
    uint32_t win = 0;

    if ((data = modrv_win_get(hwnd)))
    {
        win = data->win;
        modrv_win_release(data);
    }
    if (!win) return;
    window_title(hwnd, title, sizeof(title));
    mowin_window_set_title(win, title);
}

/***********************************************************************
 *           MO_SysCommand
 *
 * Presun a zmena velikosti oken s ramem jsou v kompozitoru; ostatni
 * prikazy beznym zpusobem.
 */
LRESULT MO_SysCommand(HWND hwnd, WPARAM wparam, LPARAM lparam, const POINT *pos)
{
    return -1;
}

LRESULT MO_DesktopWindowProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    return NtUserMessageCall(hwnd, msg, wp, lp, 0, NtUserDefWindowProc, FALSE);
}

/***********************************************************************
 *           MO_WindowMessage
 */
LRESULT MO_WindowMessage(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    struct modrv_win *data;
    RECT visible, rect;
    UINT flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;

    switch (msg)
    {
    case WM_MO_CONFIGURE:
        /* Velikost obsahu od kompozitoru: okno Wine s ramem kolem. */
        if (!(data = modrv_win_get(hwnd))) return 0;
        data->conf_w = (int)wp;
        data->conf_h = (int)lp;
        visible = data->rects.visible;
        visible.right = visible.left + (int)wp;
        visible.bottom = visible.top + (int)lp;
        rect = window_rect_from_visible(&data->rects, visible);
        modrv_win_release(data);
        NtUserSetRawWindowPos(hwnd, rect, flags | SWP_NOMOVE, FALSE);
        return 0;
    case WM_MO_PLACE:
        /* Skutecna poloha obsahu na obrazovce (nabidky a popupy se pocitaji
         * od ni). */
        if (!(data = modrv_win_get(hwnd))) return 0;
        visible = data->rects.visible;
        if (visible.left == (int)wp && visible.top == (int)lp)
        {
            modrv_win_release(data);
            return 0;
        }
        OffsetRect(&visible, (int)wp - visible.left, (int)lp - visible.top);
        rect = window_rect_from_visible(&data->rects, visible);
        modrv_win_release(data);
        NtUserSetRawWindowPos(hwnd, rect, flags | SWP_NOSIZE, FALSE);
        return 0;
    case WM_MO_SET_FOREGROUND:
        NtUserSetForegroundWindowInternal(hwnd);
        return 0;
    default:
        FIXME("got window msg %x hwnd %p wp %lx lp %lx\n", msg, hwnd, (long)wp, lp);
        return 0;
    }
}

/***********************************************************************
 * Kurzor: systemove kurzory user32 podle id zdroje na tvary kompozitoru;
 * vlastni obrazky kurzoru zatim jako sipka.
 */
static uint32_t cursor_shape(HCURSOR cursor)
{
    WCHAR module_buf[MAX_PATH], res_buf[MAX_PATH];
    UNICODE_STRING module = {0, sizeof(module_buf) - sizeof(WCHAR), module_buf};
    UNICODE_STRING res_name = {0, sizeof(res_buf) - sizeof(WCHAR), res_buf};
    ICONINFO info;
    WORD id;

    if (!cursor) return 12; /* skryty */
    if (!NtUserGetIconInfo(cursor, &info, &module, &res_name, NULL, 0)) return 0;
    if (info.hbmColor) NtGdiDeleteObjectApp(info.hbmColor);
    if (info.hbmMask) NtGdiDeleteObjectApp(info.hbmMask);
    id = res_name.Length ? 0 : LOWORD(res_name.Buffer);
    switch (id)
    {
    case 32513: return 1;  /* IDC_IBEAM */
    case 32514: return 2;  /* IDC_WAIT */
    case 32515: return 3;  /* IDC_CROSS */
    case 32650: return 4;  /* IDC_APPSTARTING */
    case 32642: return 5;  /* IDC_SIZENWSE */
    case 32643: return 6;  /* IDC_SIZENESW */
    case 32644: return 7;  /* IDC_SIZEWE */
    case 32645: return 8;  /* IDC_SIZENS */
    case 32646: return 9;  /* IDC_SIZEALL */
    case 32648: return 10; /* IDC_NO */
    case 32649: return 11; /* IDC_HAND */
    default: return 0;
    }
}

/***********************************************************************
 *           MO_SetCursor
 */
void MO_SetCursor(HWND hwnd, HCURSOR cursor)
{
    struct modrv_win *data;
    uint32_t win = 0;

    if (!hwnd) return;
    if ((data = modrv_win_get(NtUserGetAncestor(hwnd, GA_ROOT))))
    {
        win = data->win;
        if (!win && data->owner)
        {
            struct modrv_win *od = win_data_get_nolock(data->owner);
            if (od) win = od->win;
        }
        modrv_win_release(data);
    }
    if (win) mowin_set_cursor_shape(win, cursor_shape(cursor));
}

/***********************************************************************
 * Plocha okna: GDI kresli do bitmapy plochy, flush ji cele zkopiruje do
 * bufferu okna nebo popupu (mowin_*_present).
 */

static void mo_surface_set_clip(struct window_surface *surface, const RECT *rects, UINT count)
{
}

static BOOL mo_surface_flush(struct window_surface *surface, const RECT *rect, const RECT *dirty,
                             const BITMAPINFO *color_info, const void *color_bits, BOOL shape_changed,
                             const BITMAPINFO *shape_info, const void *shape_bits)
{
    int w = color_info->bmiHeader.biWidth, h = abs(color_info->bmiHeader.biHeight);
    struct modrv_win *data;
    uint32_t win = 0, popup = 0;
    int r;

    if ((data = modrv_win_get(surface->hwnd)))
    {
        win = data->win;
        popup = data->popup;
        modrv_win_release(data);
    }
    if (popup) r = mowin_popup_present(popup, color_bits, w * 4, w, h);
    else if (win) r = mowin_window_present(win, color_bits, w * 4, w, h);
    else return FALSE; /* okno jeste neni u kompozitoru: zustane poskozene */
    if (r < 0) WARN("hwnd %p: present failed %d\n", surface->hwnd, r);
    return r >= 0;
}

static void mo_surface_destroy(struct window_surface *surface)
{
}

static const struct window_surface_funcs mo_surface_funcs =
{
    mo_surface_set_clip,
    mo_surface_flush,
    mo_surface_destroy
};

static struct window_surface *mo_surface_create(HWND hwnd, const RECT *rect)
{
    char buffer[FIELD_OFFSET(BITMAPINFO, bmiColors[256])];
    BITMAPINFO *info = (BITMAPINFO *)buffer;
    int width = rect->right - rect->left;
    int height = rect->bottom - rect->top;

    memset(info, 0, sizeof(*info));
    info->bmiHeader.biSize = sizeof(info->bmiHeader);
    info->bmiHeader.biWidth = width;
    info->bmiHeader.biHeight = -height; /* shora dolu */
    info->bmiHeader.biPlanes = 1;
    info->bmiHeader.biBitCount = 32;
    info->bmiHeader.biSizeImage = width * height * 4;
    info->bmiHeader.biCompression = BI_RGB;
    return window_surface_create(sizeof(struct window_surface), &mo_surface_funcs, hwnd, rect, info, 0);
}

/***********************************************************************
 *           MO_CreateWindowSurface
 */
BOOL MO_CreateWindowSurface(HWND hwnd, BOOL layered, const RECT *surface_rect, struct window_surface **surface)
{
    struct window_surface *previous;
    struct modrv_win *data;

    TRACE("hwnd %p layered %u rect %s\n", hwnd, layered, wine_dbgstr_rect(surface_rect));

    if ((previous = *surface) && previous->funcs == &mo_surface_funcs) return TRUE;
    if (!(data = modrv_win_get(hwnd))) return TRUE; /* vychozi plocha */
    modrv_win_release(data);
    if (previous) window_surface_release(previous);
    *surface = mo_surface_create(hwnd, surface_rect);
    return TRUE;
}
