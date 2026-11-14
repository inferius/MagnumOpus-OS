/* string.h - doplnek cstd nad std (strerror); zaklad v cstd-core. */
#ifndef _CSTD_POSIX_STRING_H
#define _CSTD_POSIX_STRING_H

#include_next <string.h>

_CSTD_BEGIN

char *strerror(int e);
int strerror_r(int e, char *buf, size_t n);
size_t strspn(const char *s, const char *accept);
size_t strcspn(const char *s, const char *reject);
char *strpbrk(const char *s, const char *accept);
char *strtok_r(char *s, const char *delim, char **save);
char *strdup(const char *s);
char *strndup(const char *s, size_t n);
char *stpcpy(char *dst, const char *src);
int strcoll(const char *a, const char *b);
char *strsignal(int sig);
int strcasecmp(const char *a, const char *b);
int strncasecmp(const char *a, const char *b, size_t n);

_CSTD_END

#endif
