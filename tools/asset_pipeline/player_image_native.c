/* Memory AES-CBC and LZX for the setup-time player-image helper. Not part of the game. */
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <mspack.h>
#include <lzx.h>
#include "rijndael-alg-fst.h"

typedef struct {
  struct mspack_system sys;
  unsigned char *buffer;
  off_t size;
  off_t offset;
} memfile;

static int mread(struct mspack_file *file, void *buffer, int chars) {
  memfile *m = (memfile *)file;
  off_t remain = m->size - m->offset;
  off_t n = chars < remain ? chars : remain;
  if (n < 0) n = 0;
  memcpy(buffer, m->buffer + m->offset, (size_t)n);
  m->offset += n;
  return (int)n;
}

static int mwrite(struct mspack_file *file, void *buffer, int chars) {
  memfile *m = (memfile *)file;
  off_t remain = m->size - m->offset;
  off_t n = chars < remain ? chars : remain;
  if (n < 0) n = 0;
  memcpy(m->buffer + m->offset, buffer, (size_t)n);
  m->offset += n;
  return (int)n;
}

static void *malloc_z(struct mspack_system *sys, size_t n) {
  (void)sys;
  return calloc(1, n);
}

static void mfree(void *p) { free(p); }
static void mcopy(void *src, void *dest, size_t n) { memcpy(dest, src, n); }
static void mmess(struct mspack_file *file, const char *format, ...) {
  (void)file;
  (void)format;
}

int lzx_decompress(const void *src, size_t src_len, void *dest, size_t dest_len,
                   uint32_t window_size, const void *window_data, size_t window_data_len) {
  uint32_t bits = 0, w = window_size;
  while (w > 1 && (w & 1) == 0) {
    w >>= 1;
    bits++;
  }
  if (w != 1 || (window_data && window_data_len > window_size)) return 1;
  struct mspack_system sys = {0};
  sys.read = mread;
  sys.write = mwrite;
  sys.alloc = malloc_z;
  sys.free = mfree;
  sys.copy = mcopy;
  sys.message = mmess;
  memfile in = {.sys = sys, .buffer = (unsigned char *)src, .size = (off_t)src_len};
  memfile out = {.sys = sys, .buffer = dest, .size = (off_t)dest_len};
  struct lzxd_stream *lzxd = lzxd_init(&sys, (struct mspack_file *)&in, (struct mspack_file *)&out,
                                       (int)bits, 0, 0x8000, (off_t)dest_len, 0);
  if (!lzxd) return 2;
  if (window_data) {
    size_t pad = window_size - window_data_len;
    memset(lzxd->window, 0, pad);
    memcpy(lzxd->window + pad, window_data, window_data_len);
    lzxd->ref_data_size = window_size;
  }
  int rc = lzxd_decompress(lzxd, (off_t)dest_len);
  lzxd_free(lzxd);
  return rc;
}

void aes_cbc_decrypt(const uint8_t key[16], const uint8_t *in, size_t len, uint8_t *out) {
  uint32_t rk[4 * (MAXNR + 1)];
  uint8_t ivec[16] = {0};
  int Nr = rijndaelKeySetupDec(rk, key, 128);
  for (size_t n = 0; n < len; n += 16) {
    rijndaelDecrypt(rk, Nr, in + n, out + n);
    for (int i = 0; i < 16; i++) {
      out[n + i] ^= ivec[i];
      ivec[i] = in[n + i];
    }
  }
}
