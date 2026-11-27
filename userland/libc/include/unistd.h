#ifndef _GOETHITE_UNISTD_H
#define _GOETHITE_UNISTD_H

#include <stddef.h>

typedef long ssize_t;
typedef int pid_t;

extern char **environ;

#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define WNOHANG 1

int open(const char *path, int flags, unsigned int mode);
int close(int fd);
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
pid_t fork(void);
int execve(const char *path, char *const argv[], char *const envp[]);
pid_t waitpid(pid_t pid, int *status, int options);
pid_t wait(int *status);
int sched_yield(void);
void _exit(int status);

#endif
