/*
 * stdio.h - proudy FILE (cstd nad std): stdin/stdout/stderr procesu
 * a soubory. Zaklad (snprintf, ...) je v cstd-core.
 */
#ifndef _CSTD_POSIX_STDIO_H
#define _CSTD_POSIX_STDIO_H

#include_next <stdio.h>
#include <sys/types.h>

_CSTD_BEGIN

typedef struct FILE FILE;

extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

#define EOF (-1)
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2
#define BUFSIZ 4096
#define FILENAME_MAX 4096
#define FOPEN_MAX 4096
#define L_tmpnam 64

FILE *fopen(const char *path, const char *mode);
int fclose(FILE *f);
int fflush(FILE *f);
size_t fread(void *p, size_t size, size_t n, FILE *f);
size_t fwrite(const void *p, size_t size, size_t n, FILE *f);
int fgetc(FILE *f);
int getc(FILE *f);
int getchar(void);
int ungetc(int c, FILE *f);
char *fgets(char *buf, int n, FILE *f);
int fputc(int c, FILE *f);
int putc(int c, FILE *f);
int fputs(const char *s, FILE *f);
_CSTD_PRINTF(2, 3) int fprintf(FILE *f, const char *fmt, ...);
int vfprintf(FILE *f, const char *fmt, va_list ap);
/* Vystup do noveho bufferu z malloc (uvolnit free); chyba = -1. */
_CSTD_PRINTF(2, 3) int asprintf(char **out, const char *fmt, ...);
int vasprintf(char **out, const char *fmt, va_list ap);

/* sprintf a vsprintf s pojistkou jako _FORTIFY_SOURCE (src/fortify.rs):
 * zna-li prekladac velikost bufferu, preteceni = abort se zpravou misto
 * prepsani pameti; jinak bezny sprintf. Zaroven tvar __sprintf_chk z glibc.
 * Pro novy kod je lepsi snprintf. */
int sprintf(char *s, const char *fmt, ...);
int vsprintf(char *s, const char *fmt, va_list ap);
int __sprintf_chk(char *s, int flag, size_t slen, const char *fmt, ...);
int __vsprintf_chk(char *s, int flag, size_t slen, const char *fmt, va_list ap);
#if !defined(__cplusplus) && !defined(_CSTD_NO_FORTIFY)
#define sprintf(s, ...) __sprintf_chk((s), 0, __builtin_object_size((s), 1), __VA_ARGS__)
#define vsprintf(s, fmt, ap) __vsprintf_chk((s), 0, __builtin_object_size((s), 1), (fmt), (ap))
#endif
int fseek(FILE *f, long off, int whence);
long ftell(FILE *f);
void rewind(FILE *f);
int feof(FILE *f);
int ferror(FILE *f);
void clearerr(FILE *f);
int setvbuf(FILE *f, char *buf, int mode, size_t size);
void setbuf(FILE *f, char *buf);
void perror(const char *s);
int remove(const char *path);
int rename(const char *from, const char *to);

/* Proudy nad cisly fd (POSIX). */
FILE *fdopen(int fd, const char *mode);
int fileno(FILE *f);
int fseeko(FILE *f, off_t off, int whence);
off_t ftello(FILE *f);
ssize_t getline(char **line, size_t *cap, FILE *f);
ssize_t getdelim(char **line, size_t *cap, int delim, FILE *f);

int sscanf(const char *s, const char *fmt, ...);
int vsscanf(const char *s, const char *fmt, va_list ap);
int fscanf(FILE *f, const char *fmt, ...);
int vfscanf(FILE *f, const char *fmt, va_list ap);
int scanf(const char *fmt, ...);

_CSTD_END

#endif
