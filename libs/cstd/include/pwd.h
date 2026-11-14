/* pwd.h - uzivatel relace (cstd): jmeno z USER, domov z HOME, cisla 0. */
#ifndef _CSTD_PWD_H
#define _CSTD_PWD_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

struct passwd {
    char *pw_name;
    char *pw_passwd;
    uid_t pw_uid;
    gid_t pw_gid;
    char *pw_gecos;
    char *pw_dir;
    char *pw_shell;
};

struct passwd *getpwuid(uid_t uid);
struct passwd *getpwnam(const char *name);

_CSTD_END

#endif
