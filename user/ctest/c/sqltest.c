/*
 * Test sqlite nad zamky souboru (port sqlite, sada "cstd porty"): dva
 * procesy nad jednou databazi s vychozim unixovym VFS (zamky fcntl drzi
 * fsd). Pouziti: sqltest a|b DB ZNACKY
 *
 *  A: zalozi tabulku -> BEGIN EXCLUSIVE + 100 radku -> znacka a-locked ->
 *     ceka na b-busy -> COMMIT -> znacka a-done.
 *  B: ceka na a-locked -> zapis bez cekani = SQLITE_BUSY -> b-busy ->
 *     s busy_timeout zapis uspeje az po COMMIT procesu A -> 101 radku.
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>

#include "sqlite3.h"

static const char *marks;

static void mark(const char *name) {
    char p[256];
    snprintf(p, sizeof p, "%s/%s", marks, name);
    int fd = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) close(fd);
}

static int wait_mark(const char *name) {
    char p[256];
    snprintf(p, sizeof p, "%s/%s", marks, name);
    for (int i = 0; i < 4000; i++) {
        if (access(p, F_OK) == 0) return 0;
        usleep(5000);
    }
    return -1;
}

static long count(sqlite3 *db) {
    sqlite3_stmt *st;
    long n = -1;
    if (sqlite3_prepare_v2(db, "SELECT count(*) FROM t", -1, &st, NULL) == SQLITE_OK && sqlite3_step(st) == SQLITE_ROW)
        n = sqlite3_column_int64(st, 0);
    sqlite3_finalize(st);
    return n;
}

static int side_a(sqlite3 *db) {
    if (sqlite3_exec(db, "CREATE TABLE IF NOT EXISTS t(x INTEGER, s TEXT)", 0, 0, 0) != SQLITE_OK) return 1;
    mark("a-ready");
    if (sqlite3_exec(db, "BEGIN EXCLUSIVE", 0, 0, 0) != SQLITE_OK) return 2;
    for (int i = 0; i < 100; i++) {
        char sql[96];
        snprintf(sql, sizeof sql, "INSERT INTO t VALUES(%d, 'radek %d')", i, i);
        if (sqlite3_exec(db, sql, 0, 0, 0) != SQLITE_OK) return 3;
    }
    mark("a-locked");
    if (wait_mark("b-busy") != 0) return 4;
    if (sqlite3_exec(db, "COMMIT", 0, 0, 0) != SQLITE_OK) return 5;
    if (count(db) != 100) return 6;
    mark("a-done");
    return 0;
}

static int side_b(sqlite3 *db) {
    if (wait_mark("a-locked") != 0) return 10;
    sqlite3_busy_timeout(db, 0);
    int rc = sqlite3_exec(db, "INSERT INTO t VALUES(1000, 'od B')", 0, 0, 0);
    if (rc != SQLITE_BUSY) {
        fprintf(stderr, "sqltest b: cekal SQLITE_BUSY, je %d (%s)\n", rc, sqlite3_errmsg(db));
        return 11;
    }
    mark("b-busy");
    sqlite3_busy_timeout(db, 20000);
    if (sqlite3_exec(db, "INSERT INTO t VALUES(1000, 'od B')", 0, 0, 0) != SQLITE_OK) return 12;
    if (wait_mark("a-done") != 0) return 13;
    if (count(db) != 101) return 14;
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr, "pouziti: sqltest a|b DB ZNACKY\n");
        return 2;
    }
    marks = argv[3];
    sqlite3 *db;
    if (sqlite3_open(argv[2], &db) != SQLITE_OK) {
        fprintf(stderr, "sqltest: open: %s\n", sqlite3_errmsg(db));
        return 1;
    }
    int r = strcmp(argv[1], "a") == 0 ? side_a(db) : side_b(db);
    if (r != 0) fprintf(stderr, "sqltest %s: krok %d: %s\n", argv[1], r, sqlite3_errmsg(db));
    sqlite3_close(db);
    if (r != 0) return 1;
    printf("ok sqlite-%s\nsqltest: hotovo\n", argv[1]);
    return 0;
}
