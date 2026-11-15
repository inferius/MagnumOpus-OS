/*
 * Test sirokych znaku a locale v cstd (wchar.h, wctype.h, locale.h):
 * prevody v locale C a C.UTF-8, retezce, cisla, tridy Unicode, wprintf,
 * wscanf, siroky proud a locale vlakna (newlocale/uselocale).
 * Vraci 0 = ok, jinak cislo kroku. Na konci vrati globalni locale na C.
 */
#include <ctype.h>
#include <errno.h>
#include <langinfo.h>
#include <limits.h>
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wchar.h>
#include <wctype.h>

static int c_locale(void) {
    /* Proces zacina v locale C: jednobajtove, bajty 0x80-0xFF vratne. */
    if (MB_CUR_MAX != 1) return 1;
    if (strcmp(setlocale(LC_ALL, NULL), "C") != 0) return 2;
    wchar_t wc = 0;
    if (mbrtowc(&wc, "\xe9", 1, NULL) != 1 || (unsigned)wc != 0xDFE9u) return 3;
    char b[MB_LEN_MAX];
    if (wcrtomb(b, wc, NULL) != 1 || (unsigned char)b[0] != 0xE9) return 4;
    if (wcrtomb(b, L'č', NULL) != (size_t)-1 || errno != EILSEQ) return 5;
    if (strcmp(nl_langinfo(CODESET), "ANSI_X3.4-1968") != 0) return 6;
    if (MB_LEN_MAX < 4) return 7;
    return 0;
}

static int utf8_conversions(void) {
    if (setlocale(LC_ALL, "cs_CZ.UTF-8") == NULL) return 10;
    if (MB_CUR_MAX != 4 || strcmp(nl_langinfo(CODESET), "UTF-8") != 0) return 11;
    if (setlocale(LC_ALL, "cs_CZ.ISO-8859-2") != NULL) return 12;
    mbstate_t st;
    memset(&st, 0, sizeof st);
    wchar_t wc;
    const char *s = "\xc5\xbe"; /* z s hackem */
    if (mbrtowc(&wc, s, 1, &st) != (size_t)-2 || mbsinit(&st)) return 13;
    if (mbrtowc(&wc, s + 1, 1, &st) != 1 || wc != L'ž') return 14;
    wchar_t buf[32];
    const char *src = "P\xc5\x99\xc3\xadli\xc5\xa1";
    if (mbsrtowcs(buf, &src, 32, NULL) != 6 || src != NULL) return 15;
    if (wcscmp(buf, L"Příliš") != 0) return 16;
    char out[32];
    const wchar_t *wsrc = buf;
    if (wcsrtombs(out, &wsrc, sizeof out, NULL) != 9 || strcmp(out, "P\xc5\x99\xc3\xadli\xc5\xa1") != 0) return 17;
    if (mbstowcs(NULL, "a\xc5\xbe", 0) != 2 || wcstombs(NULL, L"€", 0) != 3) return 18;
    if (mblen("\xff", 1) != -1) return 19;
    return 0;
}

static int strings_and_numbers(void) {
    if (wcslen(L"kůň") != 3 || wcscasecmp(L"Čaj", L"čAJ") != 0) return 20;
    if (wcsstr(L"žluťoučký", L"čk") == NULL) return 21;
    wchar_t *end;
    if (wcstol(L"  -42x", &end, 10) != -42 || *end != L'x') return 22;
    if (wcstod(L"2.5e1", NULL) != 25.0) return 23;
    if (wcstoull(L"0x10", NULL, 0) != 16) return 24;
    long double ld = wcstold(L"0.5", NULL);
    if (ld != 0.5L) return 25;
    struct tm t;
    memset(&t, 0, sizeof t);
    t.tm_year = 126;
    t.tm_mon = 10;
    t.tm_mday = 15;
    wchar_t tb[32];
    if (wcsftime(tb, 32, L"%Y-%m-%d ž", &t) != 12 || wcscmp(tb, L"2026-11-15 ž") != 0) return 26;
    if (!iswalpha(L'č') || !iswupper(L'Č') || towupper(L'ř') != L'Ř') return 27;
    if (iswdigit(0x0663) || !iswspace(0x3000) || iswspace(0xA0)) return 28;
    if (!iswctype(L'ž', wctype("alpha")) || towctrans(L'a', wctrans("toupper")) != L'A') return 29;
    if (wcwidth(L'字') != 2 || wcwidth(0x0301) != 0 || wcswidth(L"a字", 5) != 3) return 30;
    return 0;
}

static int formatted(const char *dir) {
    wchar_t buf[64];
    int n = swprintf(buf, 64, L"%d|%ls|%s|%5.1f|%lc", 7, L"čaj", "k\xc5\xaf\xc5\x88", 2.25, (wint_t)0x1F600);
    if (n != 17 || wcscmp(buf, L"7|čaj|kůň|  2.2|\U0001F600") != 0) return 40;
    if (swprintf(buf, 4, L"%s", "dlouhy") != -1) return 41;
    char nb[32];
    if (snprintf(nb, sizeof nb, "%ls/%lc", L"ž", (wint_t)L'€') != 6 || strcmp(nb, "\xc5\xbe/\xe2\x82\xac") != 0) return 42;
    char *dyn = NULL;
    if (asprintf(&dyn, "%s-%d-%ls", "a", 12, L"ž") != 7 || strcmp(dyn, "a-12-\xc5\xbe") != 0) return 55;
    free(dyn);
    int num = 0;
    wchar_t word[16];
    char mb[16];
    if (swscanf(L"　čaj 42 kůň", L"%ls %d %s", word, &num, mb) != 3) return 43;
    if (wcscmp(word, L"čaj") != 0 || num != 42 || strcmp(mb, "k\xc5\xaf\xc5\x88") != 0) return 44;
    if (sscanf("\xc5\xbe\xc3\xa1" "ba", "%ls", word) != 1 || wcscmp(word, L"žába") != 0) return 45;

    /* Siroky proud: zapis, orientace, cteni zpet. */
    char path[256];
    snprintf(path, sizeof path, "%s/wide.txt", dir);
    FILE *f = fopen(path, "w+");
    if (f == NULL) return 46;
    if (fwide(f, 0) != 0) return 47;
    if (fwprintf(f, L"%ls %d\n", L"čaj", 5) != 6 || fwide(f, 0) <= 0) return 48;
    if (fputws(L"kůň\n", f) < 0 || fputwc(L'€', f) == WEOF) return 49;
    rewind(f);
    if (fgetwc(f) != L'č' || ungetwc(L'Č', f) != L'Č') return 50;
    if (fgetws(buf, 64, f) == NULL || wcscmp(buf, L"Čaj 5\n") != 0) return 51;
    int k = 0;
    if (fwscanf(f, L"%ls", word) != 1 || wcscmp(word, L"kůň") != 0) return 52;
    while (fgetwc(f) != L'€') {
        if (++k > 4) return 53;
    }
    if (fgetwc(f) != WEOF) return 54;
    fclose(f);
    remove(path);
    return 0;
}

static int thread_locale(void) {
    /* Globalne C, vlakno UTF-8 pres uselocale. */
    if (setlocale(LC_ALL, "C") == NULL || MB_CUR_MAX != 1) return 60;
    locale_t l = newlocale(LC_CTYPE_MASK, "C.UTF-8", (locale_t)0);
    if (l == (locale_t)0) return 61;
    if (uselocale(l) != LC_GLOBAL_LOCALE || MB_CUR_MAX != 4) return 62;
    if (strcmp(nl_langinfo_l(CODESET, l), "UTF-8") != 0) return 63;
    if (!isalpha_l('a', l) || toupper_l('b', l) != 'B' || !iswalpha_l(L'č', l)) return 64;
    if (strtol_l("12", NULL, 10, l) != 12 || strcoll_l("a", "b", l) >= 0) return 65;
    locale_t d = duplocale(l);
    if (uselocale(LC_GLOBAL_LOCALE) != l || MB_CUR_MAX != 1) return 66;
    freelocale(d);
    freelocale(l);
    /* Slozene jmeno: jen LC_CTYPE v UTF-8. */
    setlocale(LC_CTYPE, "C.UTF-8");
    const char *all = setlocale(LC_ALL, NULL);
    if (all == NULL || strcmp(all, "C.UTF-8;C;C;C;C;C") != 0) return 67;
    char saved[64];
    snprintf(saved, sizeof saved, "%s", all);
    setlocale(LC_ALL, "C");
    if (setlocale(LC_ALL, saved) == NULL || MB_CUR_MAX != 4) return 68;
    struct lconv *lc = localeconv();
    if (lc == NULL || strcmp(lc->decimal_point, ".") != 0 || lc->frac_digits != CHAR_MAX) return 69;
    setlocale(LC_ALL, "C");
    return 0;
}

int wchar_test(const char *dir) {
    int r;
    if ((r = c_locale()) != 0) return r;
    if ((r = utf8_conversions()) != 0) goto out;
    if ((r = strings_and_numbers()) != 0) goto out;
    if ((r = formatted(dir)) != 0) goto out;
    r = thread_locale();
out:
    setlocale(LC_ALL, "C");
    return r;
}
