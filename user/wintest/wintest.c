/* Konzolovy test Windows vrstvy (W1, docs/cs/windows-vrstva.md): obycejny
 * program Windows (PE, ucrtbase + kernel32), prelozeny nastroji sestaveni
 * Wine (tools/wsl/build_wine.sh). Spousti ho sada `wine` v QEMU:
 *
 *   wine wintest.exe alfa "beta gama"   (prostredi WINTEST_VAR=hodnota)
 *
 * Kontroly: printf, argv, prostredi (getenv i GetEnvironmentVariableA),
 * soubory (fopen/fprintf/fgets v textovem rezimu, atributy, smazani),
 * vlakno (CreateThread, WaitForSingleObject, kod vlakna). Kod konce 42 =
 * vse sedi, 100 + bity chyb jinak. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

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
