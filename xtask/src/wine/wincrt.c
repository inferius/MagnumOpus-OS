/* Doplnek startu pro programy Windows prekladane na hostiteli nad SDK Wine
 * (PuTTY, xtask/src/wine/putty.rs; nas kod, ne z Wine ani z CRT MSVC).
 *
 * - main: Wine dava v ucrtbase.lib jen mainCRTStartup (inicializace CRT,
 *   argumenty, volani main); program s WinMain dostane tenhle main, ktery
 *   WinMain zavola s prikazovou radkou bez jmena programu.
 * - TLS: promenne __declspec(thread) potrebuji adresar TLS (_tls_used,
 *   ktery linker zapise do hlavicky PE) a _tls_index (index modulu, plni
 *   zavadec). V CRT MSVC je to tlssup; Wine ho pro exe nedava.
 *
 * Preklad: clang --target=x86_64-pc-windows-msvc s hlavickami Wine. */

#include <windows.h>

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPSTR cmdline, int show);

/* TLS: data mezi .tls a .tls$ZZZ, zpetna volani mezi .CRT$XLA a .CRT$XLZ
 * (linker radi sekce podle jmena za '$'). */
#pragma section(".tls", read, write)
#pragma section(".tls$ZZZ", read, write)
#pragma section(".CRT$XLA", read)
#pragma section(".CRT$XLZ", read)

ULONG _tls_index = 0;
__declspec(allocate(".tls")) char _tls_start = 0;
__declspec(allocate(".tls$ZZZ")) char _tls_end = 0;
__declspec(allocate(".CRT$XLA")) PIMAGE_TLS_CALLBACK __xl_a = 0;
__declspec(allocate(".CRT$XLZ")) PIMAGE_TLS_CALLBACK __xl_z = 0;

const IMAGE_TLS_DIRECTORY64 _tls_used = {
    (ULONGLONG)&_tls_start,
    (ULONGLONG)&_tls_end,
    (ULONGLONG)&_tls_index,
    (ULONGLONG)(&__xl_a + 1),
    0,
    0,
};

int main(void)
{
    char *cmd = GetCommandLineA();
    STARTUPINFOA si;
    int show = SW_SHOWDEFAULT;

    /* Jmeno programu (i v uvozovkach) a mezery za nim preskocit. */
    if (*cmd == '"')
    {
        cmd++;
        while (*cmd && *cmd != '"') cmd++;
        if (*cmd) cmd++;
    }
    else
        while (*cmd > ' ') cmd++;
    while (*cmd == ' ' || *cmd == '\t') cmd++;

    si.cb = sizeof(si);
    GetStartupInfoA(&si);
    if (si.dwFlags & STARTF_USESHOWWINDOW) show = si.wShowWindow;
    return WinMain(GetModuleHandleA(NULL), NULL, cmd, show);
}
