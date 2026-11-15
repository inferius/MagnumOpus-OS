/*
 * unistd.h - cisla fd, soubory, pracovni adresar (cstd, docs/cs/cstd.md).
 *
 * Procesy vznikaji jen pres posix_spawn: fork, vfork a exec* existuji, ale
 * vraci -1 s errno ENOSYS (prekladac na ne upozorni).
 */
#ifndef _CSTD_UNISTD_H
#define _CSTD_UNISTD_H

#include <_cstd.h>
#include <sys/types.h>

_CSTD_BEGIN

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define R_OK 4
#define W_OK 2
#define X_OK 1
#define F_OK 0

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define _SC_CLK_TCK 2
#define _SC_OPEN_MAX 4
#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE _SC_PAGESIZE
#define _SC_NPROCESSORS_CONF 83
#define _SC_NPROCESSORS_ONLN 84

#define _CSTD_NO_FORK __attribute__((__deprecated__("MagnumOpus: no fork/exec, use posix_spawn")))

int close(int fd);
ssize_t read(int fd, void *buf, size_t n);
ssize_t write(int fd, const void *buf, size_t n);
ssize_t pread(int fd, void *buf, size_t n, off_t off);
ssize_t pwrite(int fd, const void *buf, size_t n, off_t off);
off_t lseek(int fd, off_t off, int whence);
int dup(int fd);
int dup2(int oldfd, int newfd);
int dup3(int oldfd, int newfd, int flags);
int pipe(int fds[2]);
int pipe2(int fds[2], int flags);
int isatty(int fd);
int fsync(int fd);
int fdatasync(int fd);
int ftruncate(int fd, off_t len);
int truncate(const char *path, off_t len);
int unlink(const char *path);
int rmdir(const char *path);
char *getcwd(char *buf, size_t size);
int chdir(const char *path);
int access(const char *path, int mode);
long sysconf(int name);
int getpagesize(void);
pid_t getpid(void);
uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);
int fchown(int fd, uid_t u, gid_t g);
int chown(const char *path, uid_t u, gid_t g);
ssize_t readlink(const char *path, char *buf, size_t n);
int symlink(const char *target, const char *path);
int link(const char *old, const char *path);
pid_t getppid(void);
unsigned alarm(unsigned secs);
int pause(void);
unsigned sleep(unsigned secs);
int usleep(unsigned us);
/* Nahodne bajty z generatoru jadra (nejvic GETENTROPY_MAX). */
#define GETENTROPY_MAX 256
int getentropy(void *buf, size_t len);
/* Relativne k otevrenemu adresari (AT_* ve fcntl.h). */
int unlinkat(int dirfd, const char *path, int flags);
/* pathconf: stejne hodnoty pro cely FS. */
#define _PC_LINK_MAX 0
#define _PC_NAME_MAX 3
#define _PC_PATH_MAX 4
#define _PC_PIPE_BUF 5
long pathconf(const char *path, int name);
long fpathconf(int fd, int name);

_CSTD_NO_FORK pid_t fork(void);
_CSTD_NO_FORK pid_t vfork(void);
_CSTD_NO_FORK int execv(const char *path, char *const argv[]);
_CSTD_NO_FORK int execvp(const char *file, char *const argv[]);
_CSTD_NO_FORK int execve(const char *path, char *const argv[], char *const envp[]);
_CSTD_NO_FORK int execl(const char *path, const char *arg, ...);
_CSTD_NO_FORK int execlp(const char *file, const char *arg, ...);

_CSTD_END

#endif
