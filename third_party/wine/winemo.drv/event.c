/*
 * MagnumOpus graphics driver - compositor events and clipboard
 *
 * Copyright 2026 MagnumOpus contributors
 * Key scan code table from the Wayland driver (winewayland.drv),
 * Copyright 2020 Alexandros Frantzis for Collabora Ltd.
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

/* Udalosti kompozitoru (vlakno udalosti ovladace, mowin_next_event):
 * - klavesy: kod evdev -> scan kod sady 1 (VK podle tabulek klavesnice
 *   Wine); tisknutelny znak bez Ctrl/Alt jde jako Unicode (znak podle
 *   rozlozeni systemu, ne podle tabulek Wine). Compose a IME pozdeji.
 * - mys: souradnice obsahu okna -> obrazovka (viditelny obdelnik okna),
 *   vstup v popupu prijde oknu vlastnika v jeho souradnicich a Wine najde
 *   popup podle polohy,
 * - velikost, poloha, fokus a zavreni jdou jako zpravy vlaknu okna,
 * - stisk mimo popupy = WM_CANCELMODE (zavre nabidky),
 * - schranka: text jine aplikace se ve Wine nabidne odlozene a stahne se
 *   az na zadost (WM_RENDERFORMAT); zmena schranky Wine jde kompozitoru. */

#if 0
#pragma makedep unix
#endif

#include "config.h"

#include <stdlib.h>
#include <string.h>

#include "modrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(modrv);

/* Kody evdev (linux/input.h), ktere ovladac potrebuje. */
#define KEY_KPDOT       83
#define KEY_102ND       86
#define KEY_F11         87
#define KEY_F12         88
#define KEY_KPENTER     96
#define KEY_RIGHTCTRL   97
#define KEY_KPSLASH     98
#define KEY_SYSRQ       99
#define KEY_RIGHTALT    100
#define KEY_HOME        102
#define KEY_UP          103
#define KEY_PAGEUP      104
#define KEY_LEFT        105
#define KEY_RIGHT       106
#define KEY_END         107
#define KEY_DOWN        108
#define KEY_PAGEDOWN    109
#define KEY_INSERT      110
#define KEY_DELETE      111
#define KEY_PAUSE       119
#define KEY_LEFTMETA    125
#define KEY_RIGHTMETA   126
#define KEY_COMPOSE     127
#define KEY_MAX_TRACKED 256
#define BTN_LEFT        0x110
#define BTN_RIGHT       0x111
#define BTN_MIDDLE      0x112
#define BTN_SIDE        0x113
#define BTN_EXTRA       0x114

/* Bity modifikatoru (inputproto::mods). */
#define MO_MOD_CTRL  (1 << 1)
#define MO_MOD_ALT   (1 << 2)
#define MO_MOD_META  (1 << 4)

static WORD key2scan(UINT key)
{
    /* zakladni klavesy jsou primo scan kody sady 1 */
    if (key <= KEY_KPDOT) return key;

    switch (key)
    {
    case KEY_SYSRQ: return 0x0054;
    case KEY_102ND: return 0x0056;
    case KEY_F11: return 0x0057;
    case KEY_F12: return 0x0058;
    case KEY_COMPOSE: return 0x015d;
    case KEY_KPENTER: return 0x011c;
    case KEY_RIGHTCTRL: return 0x011d;
    case KEY_KPSLASH: return 0x0135;
    case KEY_RIGHTALT: return 0x0138;
    case KEY_HOME: return 0x0147;
    case KEY_UP: return 0x0148;
    case KEY_PAGEUP: return 0x0149;
    case KEY_LEFT: return 0x014b;
    case KEY_RIGHT: return 0x014d;
    case KEY_END: return 0x014f;
    case KEY_DOWN: return 0x0150;
    case KEY_PAGEDOWN: return 0x0151;
    case KEY_INSERT: return 0x0152;
    case KEY_DELETE: return 0x0153;
    case KEY_LEFTMETA: return 0x015b;
    case KEY_RIGHTMETA: return 0x015c;
    case KEY_PAUSE: return 0x021d;
    }
    return 0x200 | (key & 0x7f);
}

/* Klavesnice (rozhodnuti W3, docs/cs/windows-vrstva.md):
 * - kazda klavesa jde jako WM_KEYDOWN/WM_KEYUP s VK a scan kodem sady 1;
 *   pismena a cislice maji VK podle znaku z rozlozeni systemu (Y na
 *   QWERTZ = VK_Z), ostatni klavesy VK z tabulek Wine (pozice US),
 * - WM_CHAR: TranslateMessage se pta MO_ToUnicodeEx, ta vrati znak, ktery
 *   pri poslednim stisku te klavesy poslal kompozitor (podle rozlozeni
 *   systemu); udalost textu stejneho znaku se pak uz nevklada,
 * - text bez stisku (mrtva klavesa, slozeny znak) jde jako Unicode
 *   (VK_PACKET). Compose a IME pozdeji. */

/* Znak posledniho stisku podle scan kodu (0x000-0x1ff): 0 = rozhodnou
 * tabulky Wine (Enter, Tab, zkratky s Ctrl), NO_CHAR = klavesa znak nedava
 * (mrtva klavesa, sipky). */
#define NO_CHAR 0xffff
static WCHAR key_char[0x200];
/* VK stisknute klavesy (pusteni posle stejny VK). */
static WORD key_vk[KEY_MAX_TRACKED];
/* Znak, ktery dal posledni stisk: stejna udalost textu se nevklada. */
static WCHAR expect_text;

static void send_unicode(HWND hwnd, WCHAR ch, BOOL up)
{
    INPUT input = {0};
    input.type = INPUT_KEYBOARD;
    input.ki.wScan = ch;
    input.ki.dwFlags = KEYEVENTF_UNICODE | (up ? KEYEVENTF_KEYUP : 0);
    NtUserSendHardwareInput(hwnd, 0, &input, 0);
}

/* Klavesa s VK `vk` (0 = podle scan kodu a tabulek Wine). */
static void send_key(HWND hwnd, UINT key, WORD vk, BOOL up)
{
    UINT scan = key2scan(key);
    INPUT input = {0};
    input.type = INPUT_KEYBOARD;
    input.ki.wVk = vk;
    input.ki.wScan = (scan & 0x300) ? scan + 0xdf00 : scan;
    input.ki.dwFlags = vk ? 0 : KEYEVENTF_SCANCODE;
    if (scan & ~0xff) input.ki.dwFlags |= KEYEVENTF_EXTENDEDKEY;
    if (up) input.ki.dwFlags |= KEYEVENTF_KEYUP;
    NtUserSendHardwareInput(hwnd, 0, &input, 0);
}

/* VK pismena nebo cislice podle znaku z rozlozeni, jinak 0. */
static WORD char_vk(WCHAR ch)
{
    if (ch >= 'a' && ch <= 'z') return ch - 'a' + 'A';
    if ((ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9')) return ch;
    return 0;
}

/* Klavesy, ktere znak davaji vzdy podle tabulek Wine (ridici znaky). */
static BOOL control_key(UINT scan)
{
    switch (scan)
    {
    case 0x01: case 0x0e: case 0x0f: case 0x1c: case 0x11c: return TRUE;
    default: return FALSE;
    }
}

static void handle_key(HWND hwnd, const mowin_event *ev)
{
    UINT key = ev->a, scan = key2scan(key) & 0x1ff;
    BOOL up = ev->b == 0;
    WCHAR ch = ev->c > 0 && ev->c < 0x10000 ? ev->c : 0;
    BOOL mods = (ev->mods & (MO_MOD_CTRL | MO_MOD_ALT | MO_MOD_META)) != 0;
    BOOL text = ch >= 0x20 && ch != 0x7f && !mods;
    WORD vk;

    if (up)
    {
        vk = key < KEY_MAX_TRACKED ? key_vk[key] : 0;
        send_key(hwnd, key, vk, TRUE);
        return;
    }
    /* Znak pro MO_ToUnicodeEx (pred vstupem: TranslateMessage se pta hned). */
    if (text) key_char[scan] = ch;
    else if (mods || control_key(scan)) key_char[scan] = 0;
    else key_char[scan] = ch ? 0 : NO_CHAR;
    if (text) expect_text = ch;
    vk = char_vk(ch);
    if (key < KEY_MAX_TRACKED) key_vk[key] = vk;
    send_key(hwnd, key, vk, FALSE);
}

/***********************************************************************
 *           MO_ToUnicodeEx
 *
 * Znak klavesy podle rozlozeni systemu (posledni stisk); ostatni (Ctrl,
 * Alt, ridici klavesy) tabulky Wine (-2 = vychozi).
 */
INT MO_ToUnicodeEx(UINT virt, UINT scan, const BYTE *state, LPWSTR str, int size, UINT flags, HKL layout)
{
    WCHAR c;

    if (virt == VK_PACKET) return -2;
    if ((state[VK_CONTROL] & 0x80) || (state[VK_MENU] & 0x80)) return -2;
    c = key_char[(scan & 0xff) | (scan & 0x100)];
    if (!c) return -2;
    if (c == NO_CHAR) return 0;
    if (size > 0) str[0] = c;
    if (size > 1) str[1] = 0;
    return 1;
}

static void send_mouse(HWND hwnd, const RECT *visible, int x, int y, DWORD flags, DWORD data)
{
    INPUT input = {0};
    input.type = INPUT_MOUSE;
    input.mi.dx = visible->left + x;
    input.mi.dy = visible->top + y;
    input.mi.mouseData = data;
    input.mi.dwFlags = flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
    NtUserSendHardwareInput(hwnd, 0, &input, 0);
}

/* Posledni poloha ukazatele v obsahu (pro tlacitka a kolecko). */
static int last_x, last_y;

static void handle_button(HWND hwnd, const RECT *visible, const mowin_event *ev)
{
    DWORD flags, data = 0;

    switch (ev->a)
    {
    case BTN_LEFT: flags = MOUSEEVENTF_LEFTDOWN; break;
    case BTN_RIGHT: flags = MOUSEEVENTF_RIGHTDOWN; break;
    case BTN_MIDDLE: flags = MOUSEEVENTF_MIDDLEDOWN; break;
    case BTN_SIDE: flags = MOUSEEVENTF_XDOWN; data = XBUTTON1; break;
    case BTN_EXTRA: flags = MOUSEEVENTF_XDOWN; data = XBUTTON2; break;
    default: return;
    }
    if (!ev->b) flags <<= 1; /* *DOWN -> *UP */
    last_x = ev->c;
    last_y = ev->d;
    send_mouse(hwnd, visible, ev->c, ev->d, flags, data);
}

static void handle_wheel(HWND hwnd, const RECT *visible, const mowin_event *ev)
{
    if (ev->b) send_mouse(hwnd, visible, last_x, last_y, MOUSEEVENTF_WHEEL, ev->b * WHEEL_DELTA);
    if (ev->a) send_mouse(hwnd, visible, last_x, last_y, MOUSEEVENTF_HWHEEL, ev->a * WHEEL_DELTA);
}

/***********************************************************************
 * Schranka
 */

static HWND clipboard_hwnd;

/* Text jine aplikace: Wine ho dostane odlozene (WM_RENDERFORMAT). */
static void clipboard_from_host(BOOL has_text)
{
    struct set_clipboard_params params = {0};

    if (!clipboard_hwnd || !NtUserOpenClipboard(clipboard_hwnd, 0)) return;
    NtUserEmptyClipboard();
    if (has_text) NtUserSetClipboardData(CF_UNICODETEXT, 0, &params);
    NtUserCloseClipboard();
}

/* WM_RENDERFORMAT: stahnout text od kompozitoru (posledni fyzicky stisk
 * procesu, napr. Ctrl+V). */
static void render_format(UINT format)
{
    struct set_clipboard_params params = {0};
    int64_t len;
    char *utf8;
    WCHAR *text;
    DWORD wlen = 0;

    if (format != CF_UNICODETEXT) return;
    if ((len = mowin_clipboard_fetch()) < 0)
    {
        WARN("clipboard fetch failed %d\n", (int)len);
        return;
    }
    if (!(utf8 = malloc(len + 1))) return;
    mowin_clipboard_copy(utf8, len + 1);
    if ((text = malloc((len + 1) * sizeof(WCHAR))))
    {
        if (!RtlUTF8ToUnicodeN(text, len * sizeof(WCHAR), &wlen, utf8, len))
        {
            text[wlen / sizeof(WCHAR)] = 0;
            params.data = text;
            params.size = wlen + sizeof(WCHAR);
            NtUserSetClipboardData(CF_UNICODETEXT, 0, &params);
        }
        free(text);
    }
    free(utf8);
}

/* Zmena schranky Wine (kopirovani v aplikaci): text kompozitoru. */
static void clipboard_to_host(void)
{
    struct get_clipboard_params params = { .data_only = TRUE, .size = 1024 };
    char *utf8 = NULL;
    DWORD len = 0;

    if (!NtUserOpenClipboard(clipboard_hwnd, 0)) return;
    for (;;)
    {
        if (!(params.data = malloc(params.size))) break;
        if (NtUserGetClipboardData(CF_UNICODETEXT, &params))
        {
            const WCHAR *w = params.data;
            UINT n = params.size / sizeof(WCHAR);
            while (n && !w[n - 1]) n--;
            if ((utf8 = malloc(n * 3 + 1)) && !RtlUnicodeToUTF8N(utf8, n * 3, &len, w, n * sizeof(WCHAR)))
                utf8[len] = 0;
            break;
        }
        if (!params.data_size) break;
        free(params.data);
        params.data = NULL;
        params.size = params.data_size;
        params.data_size = 0;
    }
    NtUserCloseClipboard();
    free(params.data);
    if (utf8)
    {
        int r = mowin_clipboard_set_text(utf8);
        if (r < 0) WARN("clipboard offer failed %d\n", r);
        free(utf8);
    }
}

static BOOL is_modrv_clipboard_hwnd(HWND hwnd)
{
    static const WCHAR classnameW[] = {'_','_','w','i','n','e','m','o','_','c','l','i','p','b','o','a','r','d','_',
                                       'm','a','n','a','g','e','r',0};
    WCHAR buffer[64];
    UNICODE_STRING name = {.Buffer = buffer, .MaximumLength = sizeof(buffer)};

    if (!NtUserGetClassName(hwnd, FALSE, &name)) return FALSE;
    return !wcscmp(buffer, classnameW);
}

/***********************************************************************
 *           MO_ClipboardWindowProc
 */
LRESULT MO_ClipboardWindowProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    switch (msg)
    {
    case WM_NCCREATE:
        /* Vychozi okno schranky procesu plochy nahrazuje okno ovladace. */
        if (!is_modrv_clipboard_hwnd(hwnd)) return FALSE;
        clipboard_hwnd = hwnd;
        NtUserAddClipboardFormatListener(hwnd);
        return TRUE;
    case WM_CLIPBOARDUPDATE:
        if (NtUserGetClipboardOwner() == clipboard_hwnd) break;
        clipboard_to_host();
        break;
    case WM_RENDERFORMAT:
        render_format(wp);
        break;
    }
    return NtUserMessageCall(hwnd, msg, wp, lp, NULL, NtUserDefWindowProc, FALSE);
}

/***********************************************************************
 *           modrv_handle_event
 */
void modrv_handle_event(const mowin_event *ev)
{
    RECT visible;
    HWND hwnd;

    if (ev->type == MOWIN_EV_CLIPBOARD)
    {
        clipboard_from_host(ev->a != 0);
        return;
    }
    if (!(hwnd = modrv_hwnd_for_window(ev->window, &visible))) return;

    switch (ev->type)
    {
    case MOWIN_EV_KEY:
        handle_key(hwnd, ev);
        break;
    case MOWIN_EV_TEXT:
        /* Znak stisku uz dala klavesa (MO_ToUnicodeEx); jinak (mrtva
         * klavesa, slozeny znak) jako Unicode. */
        if (ev->a == expect_text)
        {
            expect_text = 0;
            break;
        }
        if (ev->a > 0 && ev->a < 0x10000)
        {
            send_unicode(hwnd, ev->a, FALSE);
            send_unicode(hwnd, ev->a, TRUE);
        }
        break;
    case MOWIN_EV_MOTION:
        last_x = ev->a;
        last_y = ev->b;
        send_mouse(hwnd, &visible, ev->a, ev->b, 0, 0);
        break;
    case MOWIN_EV_BUTTON:
        handle_button(hwnd, &visible, ev);
        break;
    case MOWIN_EV_WHEEL:
        handle_wheel(hwnd, &visible, ev);
        break;
    case MOWIN_EV_FOCUS:
        if (ev->a) NtUserPostMessage(hwnd, WM_MO_SET_FOREGROUND, 0, 0);
        break;
    case MOWIN_EV_RESIZE:
        NtUserPostMessage(hwnd, WM_MO_CONFIGURE, ev->a, ev->b);
        break;
    case MOWIN_EV_PLACE:
        NtUserPostMessage(hwnd, WM_MO_PLACE, ev->a, ev->b);
        break;
    case MOWIN_EV_CLOSE:
        NtUserPostMessage(hwnd, WM_SYSCOMMAND, SC_CLOSE, 0);
        break;
    case MOWIN_EV_POPUP_DISMISS:
        NtUserPostMessage(hwnd, WM_CANCELMODE, 0, 0);
        break;
    case MOWIN_EV_EXPOSE:
        NtUserRedrawWindow(hwnd, NULL, 0, RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN);
        break;
    default:
        break;
    }
}
