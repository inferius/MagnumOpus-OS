/*
 * stdio.h - proudy FILE (cstd nad std): stdin/stdout/stderr procesu
 * a soubory. Zaklad (snprintf, ...) je v cstd-core.
 */
#ifndef _CSTD_POSIX_STDIO_H
#define _CSTD_POSIX_STDIO_H

#include_next <stdio.h>

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
int fseek(FILE *f, long off, int whence);
long ftell(FILE *f);
void rewind(FILE *f);
int feof(FILE *f);
int ferror(FILE *f);
void clearerr(FILE *f);
int setvbuf(FILE *f, char *buf, int mode, size_t size);
void perror(const char *s);
int remove(const char *path);
int rename(const char *from, const char *to);

_CSTD_END

#endif
