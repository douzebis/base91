/*
 * SPDX-FileCopyrightText: 2000-2006 Joachim Henke
 * SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
 *
 * SPDX-License-Identifier: BSD-3-Clause
 * SPDX-License-Identifier: MIT
 */

/*
 * libbase91: base91 encoding and decoding, built from base91-rs.
 *
 * Two APIs:
 *
 * - Henke's basE91 API, as in basE91 0.6.0 (struct basE91 and the five
 *   basE91_* functions): binary compatible, so a program built against
 *   Henke's library runs unchanged with this one, and faster.
 *
 * - The base91_* API, for both formats: Henke's, and the SIMD variant
 *   (a fixed-width format, "-" prefix, encoded and decoded with SSE4.1,
 *   AVX2 or NEON).  No allocation; sizes are checked before anything is
 *   written.
 */

#ifndef BASE91_H
#define BASE91_H 1

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------------
 * Henke's basE91 API (basE91 0.6.0)
 *
 * The state lives in a struct basE91 the caller allocates; basE91_init
 * prepares it, then basE91_encode (or basE91_decode) is called on each
 * chunk, and basE91_encode_end (or basE91_decode_end) flushes the last
 * bytes and resets the state.  The output buffer must hold what the call
 * may write: about 16/13 of the input when encoding, 14/16 when decoding,
 * plus 2.  Input and output may overlap as in Henke's command-line tool,
 * when the output never overtakes the unread input.
 * --------------------------------------------------------------------- */

struct basE91 {
	unsigned long queue;
	unsigned int nbits;
	int val;
};

void basE91_init(struct basE91 *);

size_t basE91_encode(struct basE91 *, const void *, size_t, void *);

size_t basE91_encode_end(struct basE91 *, void *);

size_t basE91_decode(struct basE91 *, const void *, size_t, void *);

size_t basE91_decode_end(struct basE91 *, void *);

/* ------------------------------------------------------------------------
 * The base91_* API: both formats, one shot, no allocation
 *
 * Input and output must not overlap.
 * --------------------------------------------------------------------- */

typedef enum {
	BASE91_HENKE = 0, /* Henke's format */
	BASE91_SIMD = 1   /* the SIMD variant */
} base91_format;

/* Return codes. */
enum {
	BASE91_OK = 0,
	BASE91_ERR_BUFFER_TOO_SMALL,  /* *written is set to the size needed */
	BASE91_ERR_INVALID_WIDTH,     /* wrap: not 0 or a multiple of 32 */
	BASE91_ERR_INVALID_CHARACTER, /* a byte outside the SIMD alphabet */
	BASE91_ERR_INVALID_LAYOUT,    /* a broken line layout */
	BASE91_ERR_INVALID_LENGTH,    /* an impossible payload length */
	BASE91_ERR_INVALID_VALUE,     /* a pair of characters above 8191 */
	BASE91_ERR_NULL_POINTER,      /* a null pointer with a non-zero length */
	BASE91_ERR_INVALID_FORMAT     /* neither BASE91_HENKE nor BASE91_SIMD */
};

/*
 * The output size base91_encode needs for len input bytes: exact for the
 * SIMD variant, an upper bound for Henke's format.  wrap is the SIMD line
 * width (0, or a multiple of 32); it must be 0 for Henke's format.
 * Returns 0 for an invalid format or width.
 */
size_t base91_encoded_len_max(base91_format format, size_t len, size_t wrap);

/*
 * Encode len bytes from in into out (out_cap bytes); set *written to the
 * bytes written.  Returns BASE91_OK, or BASE91_ERR_*: nothing is written
 * then, except *written for BASE91_ERR_BUFFER_TOO_SMALL.
 */
int base91_encode(base91_format format, const void *in, size_t len,
                  size_t wrap, void *out, size_t out_cap, size_t *written);

/* The output size base91_decode needs for len encoded bytes (an upper
 * bound). */
size_t base91_decoded_len_max(size_t len);

/*
 * Decode len bytes from in, in either format: after leading whitespace, "-"
 * starts a SIMD stream, decoded strictly; anything else is Henke's format,
 * whose bytes outside the alphabet are ignored.  Sets *written to the bytes
 * written; on a decoding error, *error_offset (if error_offset is not NULL)
 * to the offset of the offending byte, and the contents of out are
 * unspecified.
 */
int base91_decode(const void *in, size_t len, void *out, size_t out_cap,
                  size_t *written, size_t *error_offset);

/* The SIMD kernel this CPU runs: "avx2", "sse4.1", "neon" or "scalar". */
const char *base91_simd_level(void);

/* A static description of a return code. */
const char *base91_strerror(int code);

/* The library's version, e.g. "0.3.3". */
const char *base91_version(void);

#ifdef __cplusplus
}
#endif

#endif /* base91.h */
