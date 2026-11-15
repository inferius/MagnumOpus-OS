/* Rezimy GUI testu Windows vrstvy (W3, docs/cs/windows-vrstva.md,
 * kapitola W3): okna pres ovladac grafiky winemo.drv a kompozitor.
 *
 *   wine wintest.exe gdi    okno 320 x 200 s barevnymi obdelniky z GDI
 *                           (FillRect); test overi pixely v kompozitoru
 *   wine wintest.exe ramy   dve okna: bezne s titulkem (ram a titulek
 *                           kresli system) a okno, ktere si neklientskou
 *                           oblast zrusi (WM_NCCALCSIZE: klient = cele
 *                           okno), cele kreslene aplikaci (bez ramu)
 *
 * Program vypise radek na stdout, kdyz ma vse nakreslene, a skonci po
 * zavreni oken nebo nejpozdeji za 120 s. */

#include <stdio.h>
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

int gui_frames(void)
{
    expected = 2;
    done_line = "wintest ramy: nakresleno";
    if (!make_window(L"wintest-ram", gdi_proc, L"ramy-standard", 240, 120)) return 1;
    if (!make_window(L"wintest-vlastni", custom_frame_proc, L"ramy-vlastni", 240, 120)) return 1;
    return run_loop();
}
