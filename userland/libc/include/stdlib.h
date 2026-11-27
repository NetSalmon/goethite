#ifndef _GOETHITE_STDLIB_H
#define _GOETHITE_STDLIB_H

#include <stddef.h>

extern char **environ;

void *malloc(size_t size);
void free(void *ptr);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);

char *getenv(const char *name);
int setenv(const char *name, const char *value, int overwrite);
int unsetenv(const char *name);
int putenv(char *string);
int clearenv(void);

int atoi(const char *nptr);
void exit(int status);
void _exit(int status);
void abort(void);

#endif
