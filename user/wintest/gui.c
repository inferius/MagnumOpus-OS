/* Rezimy GUI testu Windows vrstvy (W3, docs/cs/windows-vrstva.md,
 * kapitola W3): okna pres ovladac grafiky winemo.drv a kompozitor.
 *
 *   wine wintest.exe gdi    okno 320 x 200 s barevnymi obdelniky z GDI
 *                           (FillRect); test overi pixely v kompozitoru
 *   wine wintest.exe klavesy okno, ktere vypise kazde WM_KEYDOWN
 *                           ("down <vk>") a WM_CHAR ("char <znak>") na
 *                           stdout (test klaves z QEMU: VK i u pismen bez
 *                           modifikatoru, znak jednou)
 *   wine wintest.exe ramy   dve okna: bezne s titulkem (ram a titulek
 *                           kresli system) a okno, ktere si neklientskou
 *                           oblast zrusi (WM_NCCALCSIZE: klient = cele
 *                           okno), cele kreslene aplikaci (bez ramu)
 *   wine wintest.exe obnova okno vlastnika (bile, cerveny ctverec), jim
 *                           vlastneny dialog (modry) a popup dialogu
 *                           (zeleny, bez aktivace); kazde WM_PAINT vypise
 *                           "paint <jmeno>" (W4: obnova po padu
 *                           kompozitoru, test overi prekresleni po EXPOSE),
 *                           poloha popupu proti klientske oblasti
 *                           dialogu "popup <dx> <dy>"
 *
 * Program vypise radek na stdout, kdyz ma vse nakreslene, a skonci po
 * zavreni oken nebo nejpozdeji za 120 s. */

#include <stdio.h>
#include <string.h>
#include <windows.h>

/* Barvy obdelniku (test je cte v kompozitoru jako 0xFFRRGGBB). */
#define GDI_RED   RGB(255, 0, 0)
#define GDI_GREEN RGB(0, 255, 0)
#define GDI_BLUE  RGB(0, 0, 255)

static int windows_open;
static int painted;
static int expected;
static const char *done_line;

static void fill(HDC dc, int l, int t, int r, int b, COLORREF c)
{
    RECT rc = { l, t, r, b };
    HBRUSH br = CreateSolidBrush(c);
    FillRect(dc, &rc, br);
    DeleteObject(br);
}

static void after_paint(HWND hwnd)
{
    if (GetPropA(hwnd, "wintest-painted")) return;
    SetPropA(hwnd, "wintest-painted", (HANDLE)1);
    if (++painted == expected)
    {
        printf("%s\n", done_line);
        fflush(stdout);
    }
}

static LRESULT CALLBACK gdi_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    PAINTSTRUCT ps;
    RECT rc;
    HDC dc;

    switch (msg)
    {
    case WM_PAINT:
        dc = BeginPaint(hwnd, &ps);
        GetClientRect(hwnd, &rc);
        fill(dc, 0, 0, rc.right, rc.bottom, RGB(255, 255, 255));
        fill(dc, 10, 10, 60, 60, GDI_RED);
        fill(dc, 70, 10, 120, 60, GDI_GREEN);
        fill(dc, 130, 10, 180, 60, GDI_BLUE);
        EndPaint(hwnd, &ps);
        after_paint(hwnd);
        return 0;
    case WM_DESTROY:
        if (--windows_open == 0) PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

static LRESULT CALLBACK keys_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    switch (msg)
    {
    case WM_KEYDOWN:
        printf("down %02x\n", (unsigned)wp);
        fflush(stdout);
        break;
    case WM_CHAR:
        printf("char %04x\n", (unsigned)wp);
        fflush(stdout);
        return 0;
    }
    return gdi_proc(hwnd, msg, wp, lp);
}

/* Okno s vlastni neklientskou oblasti: klient = cele okno, aplikace kresli
 * i "titulek" (zeleny pruh nahore). */
static LRESULT CALLBACK custom_frame_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    PAINTSTRUCT ps;
    RECT rc;
    HDC dc;

    switch (msg)
    {
    case WM_NCCALCSIZE:
        if (wp) return 0;
        break;
    case WM_PAINT:
        dc = BeginPaint(hwnd, &ps);
        GetClientRect(hwnd, &rc);
        fill(dc, 0, 0, rc.right, rc.bottom, RGB(255, 255, 255));
        fill(dc, 0, 0, rc.right, 30, GDI_GREEN);
        EndPaint(hwnd, &ps);
        after_paint(hwnd);
        return 0;
    case WM_DESTROY:
        if (--windows_open == 0) PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

static HWND make_window(const WCHAR *cls, WNDPROC proc, const WCHAR *title, int w, int h)
{
    WNDCLASSW wc = {0};
    RECT rc = { 0, 0, w, h };
    HWND hwnd;

    wc.lpfnWndProc = proc;
    wc.hInstance = GetModuleHandleW(NULL);
    wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
    wc.lpszClassName = cls;
    RegisterClassW(&wc);
    AdjustWindowRect(&rc, WS_OVERLAPPEDWINDOW, FALSE);
    hwnd = CreateWindowW(cls, title, WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT,
                         rc.right - rc.left, rc.bottom - rc.top, NULL, NULL, wc.hInstance, NULL);
    if (!hwnd) return NULL;
    windows_open++;
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    return hwnd;
}

static int run_loop(void)
{
    MSG msg;
    UINT_PTR timer = SetTimer(NULL, 0, 120000, NULL);

    while (GetMessageW(&msg, NULL, 0, 0))
    {
        if (msg.message == WM_TIMER && msg.wParam == timer) break;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return 0;
}

int gui_gdi(void)
{
    expected = 1;
    done_line = "wintest gdi: nakresleno";
    if (!make_window(L"wintest-gdi", gdi_proc, L"wintest gdi", 320, 200)) return 1;
    return run_loop();
}

int gui_keys(void)
{
    expected = 1;
    done_line = "wintest klavesy: nakresleno";
    if (!make_window(L"wintest-klavesy", keys_proc, L"wintest klavesy", 320, 200)) return 1;
    return run_loop();
}

/* Rezim "obnova" (W4). */
static HWND owner_wnd;

/* Vlastneny dialog a jeho popup az po prvnim snimku vlastnika (casovac):
 * popup okna, ktere jeste nema snimek, ovladac odmitne. Popup patri
 * dialogu: kompozitor kresli popovery jen okna s fokusem (nabidky aktivniho
 * okna) a fokus ma dialog. */
static HWND dialog_wnd;

static void create_dialog(void)
{
    RECT rc = { 0, 0, 160, 100 };
    HINSTANCE inst = GetModuleHandleW(NULL);

    /* Vlastneny dialog (s titulkem = okno kompozitoru, WINDOW_OWNER). */
    AdjustWindowRect(&rc, WS_CAPTION | WS_SYSMENU, FALSE);
    dialog_wnd = CreateWindowW(L"wintest-obnova", L"obnova-dialog", WS_CAPTION | WS_SYSMENU, CW_USEDEFAULT,
                               CW_USEDEFAULT, rc.right - rc.left, rc.bottom - rc.top, owner_wnd, NULL, inst, NULL);
    if (!dialog_wnd) PostQuitMessage(1);
    ShowWindow(dialog_wnd, SW_SHOW);
    UpdateWindow(dialog_wnd);
}

static void create_popup(void)
{
    POINT pt = { 10, 60 }, o = { 0, 0 };
    RECT pr;
    HWND pop, dlg = dialog_wnd;
    HINSTANCE inst = GetModuleHandleW(NULL);

    /* Popup dialogu (bez titulku, bez aktivace = popup mowin) dole
     * v klientske oblasti dialogu. */
    ClientToScreen(dlg, &pt);
    pop = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, L"wintest-obnova", L"obnova-popup", WS_POPUP,
                          pt.x, pt.y, 80, 30, dlg, NULL, inst, NULL);
    if (!pop) PostQuitMessage(1);
    ShowWindow(pop, SW_SHOWNOACTIVATE);
    UpdateWindow(pop);
    /* Posun popupu proti obsahu dialogu (jen do hlaseni testu). */
    GetWindowRect(pop, &pr);
    ClientToScreen(dlg, &o);
    printf("popup %ld %ld\n", (long)(pr.left - o.x), (long)(pr.top - o.y));
    fflush(stdout);
}

static LRESULT CALLBACK recover_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp)
{
    PAINTSTRUCT ps;
    RECT rc;
    HDC dc;
    char name[32];

    switch (msg)
    {
    case WM_PAINT:
        dc = BeginPaint(hwnd, &ps);
        GetClientRect(hwnd, &rc);
        GetWindowTextA(hwnd, name, sizeof(name));
        /* Podle titulku: prvni WM_PAINT vlastnika prijde jeste uvnitr
         * make_window, kdy owner_wnd neni nastavene. */
        if (!strcmp(name, "obnova-vlastnik"))
        {
            fill(dc, 0, 0, rc.right, rc.bottom, RGB(255, 255, 255));
            fill(dc, 10, 10, 60, 60, GDI_RED);
        }
        else if (!strcmp(name, "obnova-dialog")) fill(dc, 0, 0, rc.right, rc.bottom, GDI_BLUE);
        else fill(dc, 0, 0, rc.right, rc.bottom, GDI_GREEN);
        EndPaint(hwnd, &ps);
        printf("paint %s\n", name);
        fflush(stdout);
        after_paint(hwnd);
        return 0;
    case WM_TIMER:
        /* 1: dialog po snimku vlastnika, 2: popup po snimku dialogu. */
        if (hwnd == owner_wnd && wp == 1)
        {
            KillTimer(hwnd, 1);
            create_dialog();
            SetTimer(owner_wnd, 2, 500, NULL);
        }
        else if (hwnd == owner_wnd && wp == 2)
        {
            KillTimer(hwnd, 2);
            create_popup();
        }
        return 0;
    case WM_DESTROY:
        if (hwnd == owner_wnd) PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

int gui_recover(void)
{
    expected = 3;
    done_line = "wintest obnova: nakresleno";
    owner_wnd = make_window(L"wintest-obnova", recover_proc, L"obnova-vlastnik", 400, 300);
    if (!owner_wnd) return 1;
    windows_open = 0;
    SetTimer(owner_wnd, 1, 500, NULL);
    return run_loop();
}

int gui_frames(void)
{
    expected = 2;
    done_line = "wintest ramy: nakresleno";
    if (!make_window(L"wintest-ram", gdi_proc, L"ramy-standard", 240, 120)) return 1;
    if (!make_window(L"wintest-vlastni", custom_frame_proc, L"ramy-vlastni", 240, 120)) return 1;
    return run_loop();
}
