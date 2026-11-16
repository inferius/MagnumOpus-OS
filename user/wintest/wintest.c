/* Konzolovy test Windows vrstvy (W1, docs/cs/windows-vrstva.md): obycejny
 * program Windows (PE, ucrtbase + kernel32), prelozeny nastroji sestaveni
 * Wine (tools/wsl/build_wine.sh). Spousti ho sada `wine` v QEMU:
 *
 *   wine wintest.exe alfa "beta gama"   (prostredi WINTEST_VAR=hodnota)
 *   wine wintest.exe procesy            (W2: dalsi proces, pojmenovane
 *                                        udalosti, pamet jineho procesu)
 *   wine wintest.exe sekce              (W2: sdilena sekce nad souborem
 *                                        mezi procesy, koherence s I/O)
 *   wine wintest.exe gdi | ramy         (W3: okna, gui.c)
 *   wine wintest.exe obnova             (W4: obnova oken po padu
 *                                        kompozitoru, gui.c)
 *   wine wintest.exe ton <soubor.wav>   (W4: zvuk, audio.c)
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

/* gui.c (W3) */
int gui_gdi(void);
int gui_frames(void);
int gui_keys(void);
int gui_recover(void);

/* audio.c (W4) */
int audio_tone(const char *path);

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

/* Rezim "sekce" (W2): sdilena sekce nad souborem (CreateFileMapping
 * se jmenem), pohled v rodici i v diteti = tytez stranky, koherence
 * pohledu s ReadFile/WriteFile obema smery, mapovani vetsi nez soubor
 * soubor zvetsi. Kod konce ditete 77. */
#define SECTION_SIZE 0x10000

static int section_child(void)
{
    HANDLE map = OpenFileMappingA(FILE_MAP_ALL_ACCESS, FALSE, "Local\\wintest_sekce");
    char *view;

    if (!map) return 70;
    view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, 0);
    if (!view) return 71;
    if (memcmp(view, "rodic", 6)) return 72;
    memcpy(view + 100, "dite", 5);
    UnmapViewOfFile(view);
    CloseHandle(map);
    return 77;
}

static int sections(void)
{
    PROCESS_INFORMATION pi;
    STARTUPINFOA si;
    char self[MAX_PATH], cmd[MAX_PATH + 16], path[MAX_PATH], buf[8];
    HANDLE file, map, reader;
    DWORD code = 0, n = 0;
    LARGE_INTEGER size;
    OVERLAPPED ov;
    char *view;
    int bad = 0;

    if (!GetTempPathA(sizeof(path) - 24, path)) return 100 + 1;
    strcat(path, "wintest-sekce.bin");
    file = CreateFileA(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
                       CREATE_ALWAYS, 0, NULL);
    if (file == INVALID_HANDLE_VALUE) return 100 + 1;
    /* Prazdny soubor, sekce 64 KiB: Wine soubor zvetsi (ftruncate s VMO). */
    map = CreateFileMappingA(file, NULL, PAGE_READWRITE, 0, SECTION_SIZE, "Local\\wintest_sekce");
    if (!map)
    {
        printf("wintest sekce: CreateFileMapping %lu\n", GetLastError());
        return 100 + 2;
    }
    view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, 0);
    if (!view)
    {
        printf("wintest sekce: MapViewOfFile %lu\n", GetLastError());
        return 100 + 2;
    }
    if (!GetFileSizeEx(file, &size) || size.QuadPart != SECTION_SIZE) bad |= 4;
    memcpy(view, "rodic", 6);

    GetModuleFileNameA(NULL, self, sizeof(self));
    snprintf(cmd, sizeof(cmd), "\"%s\" sekce-dite", self);
    memset(&si, 0, sizeof(si));
    si.cb = sizeof(si);
    if (!CreateProcessA(NULL, cmd, NULL, NULL, FALSE, 0, NULL, NULL, &si, &pi)) return 100 + 8;
    if (WaitForSingleObject(pi.hProcess, 60000) != WAIT_OBJECT_0 || !GetExitCodeProcess(pi.hProcess, &code) ||
        code != 77)
    {
        printf("wintest sekce: kod ditete %lu\n", code);
        bad |= 8;
    }
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    /* Zapis ditete v pohledu rodice (tytez stranky). */
    if (memcmp(view + 100, "dite", 5)) bad |= 16;

    /* Pohled -> soubor: ReadFile jinym handlem vidi zapis ditete. */
    FlushViewOfFile(view, 0);
    reader = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL);
    memset(&ov, 0, sizeof(ov));
    ov.Offset = 100;
    if (reader == INVALID_HANDLE_VALUE || !ReadFile(reader, buf, 5, &n, &ov) || n != 5 || memcmp(buf, "dite", 5))
        bad |= 32;
    /* Soubor -> pohled: WriteFile se ukaze v pohledu. */
    memset(&ov, 0, sizeof(ov));
    ov.Offset = 200;
    if (!WriteFile(file, "zapis", 6, &n, &ov) || n != 6 || memcmp(view + 200, "zapis", 6)) bad |= 64;

    if (reader != INVALID_HANDLE_VALUE) CloseHandle(reader);
    UnmapViewOfFile(view);
    CloseHandle(map);
    CloseHandle(file);
    DeleteFileA(path);
    printf("wintest sekce: chyby %d\n", bad);
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
    if (argc >= 2 && !strcmp(argv[1], "sekce-dite")) return section_child();
    if (argc >= 2 && !strcmp(argv[1], "sekce")) return sections();
    if (argc >= 2 && !strcmp(argv[1], "gdi")) return gui_gdi();
    if (argc >= 2 && !strcmp(argv[1], "ramy")) return gui_frames();
    if (argc >= 2 && !strcmp(argv[1], "klavesy")) return gui_keys();
    if (argc >= 2 && !strcmp(argv[1], "obnova")) return gui_recover();
    if (argc >= 3 && !strcmp(argv[1], "ton")) return audio_tone(argv[2]);

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
