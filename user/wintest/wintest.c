/* Konzolovy test Windows vrstvy (W1, docs/cs/windows-vrstva.md): obycejny
 * program Windows (PE, ucrtbase + kernel32), prelozeny nastroji sestaveni
 * Wine (tools/wsl/build_wine.sh). Spousti ho sada `wine` v QEMU:
 *
 *   wine wintest.exe alfa "beta gama"   (prostredi WINTEST_VAR=hodnota)
 *   wine wintest.exe procesy            (W2: dalsi proces, pojmenovane
 *                                        udalosti, pamet jineho procesu)
 *
 * Kontroly: printf, argv, prostredi (getenv i GetEnvironmentVariableA),
 * soubory (fopen/fprintf/fgets v textovem rezimu, atributy, smazani),
 * vlakno (CreateThread, WaitForSingleObject, kod vlakna). Kod konce 42 =
 * vse sedi, 100 + bity chyb jinak. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

/* Rezim "procesy" (W2): dite nastavi marker, rodic ho precte a prepise
 * (ReadProcessMemory/WriteProcessMemory = PROCESS_MEMORY pres wineserver),
 * synchronizace pojmenovanymi udalostmi, kod konce ditete 77. */
static volatile LONG marker = 0x1234;

static int child(void)
{
    HANDLE ready = OpenEventA(EVENT_MODIFY_STATE | SYNCHRONIZE, FALSE, "Local\\wintest_ready");
    HANDLE quit = OpenEventA(SYNCHRONIZE, FALSE, "Local\\wintest_quit");

    marker = 0x5678;
    if (!ready || !quit || !SetEvent(ready)) return 70;
    if (WaitForSingleObject(quit, 60000) != WAIT_OBJECT_0) return 71;
    return marker == 0x9abc ? 77 : 78;
}

static int processes(void)
{
    PROCESS_INFORMATION pi;
    STARTUPINFOA si;
    char self[MAX_PATH], cmd[MAX_PATH + 16];
    HANDLE ready, quit;
    LONG value = 0;
    SIZE_T n = 0;
    DWORD code = 0;
    int bad = 0;

    ready = CreateEventA(NULL, TRUE, FALSE, "Local\\wintest_ready");
    quit = CreateEventA(NULL, TRUE, FALSE, "Local\\wintest_quit");
    if (!ready || !quit) bad |= 1;
    GetModuleFileNameA(NULL, self, sizeof(self));
    snprintf(cmd, sizeof(cmd), "\"%s\" dite", self);
    memset(&si, 0, sizeof(si));
    si.cb = sizeof(si);
    if (!CreateProcessA(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi))
    {
        printf("wintest procesy: CreateProcess %lu\n", GetLastError());
        return 100 + 2;
    }
    if (WaitForSingleObject(ready, 60000) != WAIT_OBJECT_0) bad |= 4;
    if (!ReadProcessMemory(pi.hProcess, (void *)&marker, &value, sizeof(value), &n) || n != sizeof(value) ||
        value != 0x5678)
    {
        printf("wintest procesy: ReadProcessMemory %lu, hodnota %lx\n", GetLastError(), (unsigned long)value);
        bad |= 8;
    }
    value = 0x9abc;
    if (!WriteProcessMemory(pi.hProcess, (void *)&marker, &value, sizeof(value), &n) || n != sizeof(value))
    {
        printf("wintest procesy: WriteProcessMemory %lu\n", GetLastError());
        bad |= 16;
    }
    SetEvent(quit);
    if (WaitForSingleObject(pi.hProcess, 60000) != WAIT_OBJECT_0 || !GetExitCodeProcess(pi.hProcess, &code) ||
        code != 77)
    {
        printf("wintest procesy: kod ditete %lu\n", code);
        bad |= 32;
    }
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    printf("wintest procesy: chyby %d\n", bad);
    return bad ? 100 + bad : 42;
}

static DWORD WINAPI worker(void *arg)
{
    *(volatile int *)arg = 42;
    return 7;
}

int main(int argc, char **argv)
{
    WIN32_FILE_ATTRIBUTE_DATA attrs;
    char path[MAX_PATH], buf[64], line[32];
    DWORD code = 0;
    HANDLE thread;
    const char *env;
    int bad = 0, value = 0;
    FILE *f;

    if (argc >= 2 && !strcmp(argv[1], "dite")) return child();
    if (argc >= 2 && !strcmp(argv[1], "procesy")) return processes();

    printf("wintest: start\n");

    if (argc != 3 || strcmp(argv[1], "alfa") || strcmp(argv[2], "beta gama")) bad |= 1;

    env = getenv("WINTEST_VAR");
    if (!env || strcmp(env, "hodnota")) bad |= 2;
    if (!GetEnvironmentVariableA("WINTEST_VAR", buf, sizeof(buf)) || strcmp(buf, "hodnota")) bad |= 2;

    if (!GetTempPathA(sizeof(path) - 16, path)) bad |= 4;
    strcat(path, "wintest.txt");
    f = fopen(path, "w");
    if (!f) bad |= 4;
    else
    {
        fprintf(f, "radek 1\nradek 2\n");
        fclose(f);
    }
    f = fopen(path, "r");
    if (!f || !fgets(line, sizeof(line), f) || strcmp(line, "radek 1\n")) bad |= 8;
    if (f) fclose(f);
    /* Textovy rezim: kazdy \n je na disku \r\n (2 x 9 bajtu). */
    if (!GetFileAttributesExA(path, GetFileExInfoStandard, &attrs) || attrs.nFileSizeLow != 18) bad |= 16;
    if (!DeleteFileA(path) || GetFileAttributesA(path) != INVALID_FILE_ATTRIBUTES) bad |= 32;

    thread = CreateThread(NULL, 0, worker, &value, 0, NULL);
    if (!thread || WaitForSingleObject(thread, 10000) != WAIT_OBJECT_0 ||
        !GetExitCodeThread(thread, &code) || code != 7 || value != 42)
        bad |= 64;
    if (thread) CloseHandle(thread);

    printf("wintest: chyby %d\n", bad);
    return bad ? 100 + bad : 42;
}
