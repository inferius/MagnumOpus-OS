/*
 * MagnumOpus graphics driver - PE side
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

/* Strana PE: spusti unixovou cast (spojeni s kompozitorem pres mowin),
 * vlakno udalosti kompozitoru a vlakno okna schranky procesu. */

#include <stdarg.h>
#include "windef.h"
#include "winbase.h"
#include "ntuser.h"
#include "winuser.h"

#include "unixlib.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(modrv);

static DWORD WINAPI read_events_thread(void *arg)
{
    MODRV_UNIX_CALL(read_events, NULL);
    /* Vlakno konci jen pri ztrate spojeni s kompozitorem. */
    ERR("connection to the compositor lost, terminating process\n");
    TerminateProcess(GetCurrentProcess(), 1);
    return 0;
}

static LRESULT CALLBACK clipboard_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    switch (msg)
    {
    case WM_NCCREATE:
    case WM_CLIPBOARDUPDATE:
    case WM_RENDERFORMAT:
    case WM_DESTROYCLIPBOARD:
        return NtUserMessageCall(hwnd, msg, wp, lp, 0, NtUserClipboardWindowProc, FALSE);
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

/* Okno schranky procesu: zmeny schranky Wine jdou kompozitoru, text
 * jine aplikace se vlozi az na zadost (WM_RENDERFORMAT). */
static DWORD WINAPI clipboard_thread(void *arg)
{
    static const WCHAR clipboard_classname[] = L"__winemo_clipboard_manager";
    WNDCLASSW class;
    ATOM atom;
    MSG msg;
    HWND hwnd;

    memset(&class, 0, sizeof(class));
    class.lpfnWndProc = clipboard_wndproc;
    class.lpszClassName = clipboard_classname;

    if (!(atom = RegisterClassW(&class)) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS)
    {
        ERR("could not register clipboard window class err %lu\n", GetLastError());
        return 0;
    }
    /* Rodic HWND_MESSAGE jeste nemusi existovat: zkouset znovu. */
    while (!(hwnd = CreateWindowW(clipboard_classname, NULL, 0, 0, 0, 0, 0, HWND_MESSAGE, 0, 0, NULL)) &&
           GetLastError() == ERROR_INVALID_WINDOW_HANDLE)
    {
        SwitchToThread();
    }
    if (!hwnd)
    {
        TRACE("failed to create clipboard window err %lu\n", GetLastError());
        UnregisterClassW(MAKEINTRESOURCEW(atom), NULL);
        return 0;
    }

    while (GetMessageW(&msg, 0, 0, 0)) DispatchMessageW(&msg);
    return 0;
}

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, void *reserved)
{
    DWORD tid;

    if (reason != DLL_PROCESS_ATTACH) return TRUE;

    DisableThreadLibraryCalls(instance);
    if (__wine_init_unix_call()) return FALSE;
    /* Bez spojeni s kompozitorem (proces bez role Gui) ovladac neni. */
    if (MODRV_UNIX_CALL(init, NULL)) return FALSE;

    CloseHandle(CreateThread(NULL, 0, read_events_thread, NULL, 0, &tid));
    CloseHandle(CreateThread(NULL, 0, clipboard_thread, NULL, 0, &tid));
    return TRUE;
}
