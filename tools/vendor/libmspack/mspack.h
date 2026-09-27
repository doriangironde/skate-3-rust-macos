/* Minimal declarations for the vendored lzxd.c. The full libmspack API is not used. */
#ifndef MSPACK_H
#define MSPACK_H 1
#include <sys/types.h>

struct mspack_file { int dummy; };

struct mspack_system {
  struct mspack_file *(*open)(struct mspack_system *self, const char *filename, int mode);
  void (*close)(struct mspack_file *file);
  int (*read)(struct mspack_file *file, void *buffer, int bytes);
  int (*write)(struct mspack_file *file, void *buffer, int bytes);
  int (*seek)(struct mspack_file *file, off_t offset, int mode);
  off_t (*tell)(struct mspack_file *file);
  void (*message)(struct mspack_file *file, const char *format, ...);
  void *(*alloc)(struct mspack_system *self, size_t bytes);
  void (*free)(void *ptr);
  void (*copy)(void *src, void *dest, size_t bytes);
  void *null_ptr;
};

#define MSPACK_ERR_OK 0
#define MSPACK_ERR_ARGS 1
#define MSPACK_ERR_OPEN 2
#define MSPACK_ERR_READ 3
#define MSPACK_ERR_WRITE 4
#define MSPACK_ERR_SEEK 5
#define MSPACK_ERR_NOMEMORY 6
#define MSPACK_ERR_SIGNATURE 7
#define MSPACK_ERR_DATAFORMAT 8
#define MSPACK_ERR_CHECKSUM 9
#define MSPACK_ERR_CRUNCH 10
#define MSPACK_ERR_DECRUNCH 11

#endif
